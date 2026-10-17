/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public License
 * as published by the Free Software Foundation; either version 2.1 of
 * the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful, but
 * WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see
 * <https://www.gnu.org/licenses/>.
 */
// Modified 2026-10-17: translated into Rust from fluid_rvoice_mixer.c and
// fluid_rvoice_mixer.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE),
// without the mixer threads (ENABLE_MIXER_THREADS), OpenMP and LADSPA,
// which SDL_mixer's build leaves out.

use crate::fluidsynth_priv::{
    RvoiceParam, RvoicePtr, FLUID_BUFSIZE, FLUID_MIXER_MAX_BUFFERS_DEFAULT, MAX_EVENT_PARAMS,
};
use crate::rvoice::adsr_env::FLUID_VOICE_ENVFINISHED;
use crate::rvoice::chorus::{FluidChorus, FLUID_CHORUS_PARAM_LAST};
use crate::rvoice::rev::{FluidRevmodel, FLUID_REVERB_PARAM_LAST};
use crate::rvoice::rvoice::{fluid_rvoice_write, FluidRvoice, FluidRvoiceBuffers};
use crate::synth::synth::{SYNTH_CHORUS_CHANNEL, SYNTH_REVERB_CHANNEL};
use crate::utils::ringbuffer::FluidRingbuffer;
use crate::utils::sys::{fluid_log, FLUID_ERR};
use crate::FluidReal;

/// The size of each sample buffer.
const SAMPLECOUNT: usize = FLUID_BUFSIZE * FLUID_MIXER_MAX_BUFFERS_DEFAULT;

/// Translation of `fluid_mixer_buffers_t` (its sample buffers are
/// `Vec`s rather than aligned arrays).
#[derive(Debug, Default)]
pub(crate) struct FluidMixerBuffers {
    finished_voices: Vec<usize>, /* List of voices who have finished */

    local_buf: Vec<FluidReal>,

    pub buf_count: i32,
    pub fx_buf_count: i32,

    /** buffer to store the left part of a stereo channel to.
     * Specifically a two dimensional array, containing \c buf_count sample buffers
     * (i.e. for each synth.audio-groups), of which each contains
     * FLUID_BUFSIZE * FLUID_MIXER_MAX_BUFFERS_DEFAULT audio items (=samples)
     */
    pub left_buf: Vec<FluidReal>,

    /** ditto, but for right part of a stereo channel */
    pub right_buf: Vec<FluidReal>,

    /** buffer to store the left part of a stereo effects channel to.
     * Specifically a two dimensional array, containing \c fx_buf_count buffers
     * (i.e. for each synth.effects-channels), of which each buffer contains
     * FLUID_BUFSIZE * FLUID_MIXER_MAX_BUFFERS_DEFAULT audio items (=samples)
     */
    pub fx_left_buf: Vec<FluidReal>,
    pub fx_right_buf: Vec<FluidReal>,
}

/// Translation of `fluid_mixer_fx_t`.
#[derive(Debug)]
pub(crate) struct FluidMixerFx {
    reverb: FluidRevmodel, /*< Reverb unit */
    /* reverb shadow parameters here will be returned if queried */
    pub reverb_param: [f64; FLUID_REVERB_PARAM_LAST],
    reverb_on: i32, /* reverb on/off */

    chorus: FluidChorus, /*< Chorus unit */
    /* chorus shadow parameters here will be returned if queried */
    pub chorus_param: [f64; FLUID_CHORUS_PARAM_LAST],
    chorus_on: i32, /* chorus on/off */
}

/// Translation of `fluid_rvoice_mixer_t`.
///
/// The rvoices themselves (which upstream's voices allocate, two each, and
/// the mixer points to) live in `rvoice_pool`; the voices, events and the
/// mixer's voice array refer to them by index.
pub(crate) struct FluidRvoiceMixer {
    pub fx: Vec<FluidMixerFx>,

    pub buffers: FluidMixerBuffers, /*< Used by mixer only: own buffers */

    rvoices: Vec<usize>, /*< Read-only: Voices array, sorted so that all nulls are last */
    polyphony: i32,      /*< Read-only: Length of voices array */
    active_voices: i32,  /*< Read-only: Number of non-null voices */
    current_blockcount: i32, /*< Read-only: how many blocks to process this time */
    fx_units: i32,
    with_reverb: i32,   /*< Should the synth use the built-in reverb unit? */
    with_chorus: i32,   /*< Should the synth use the built-in chorus unit? */
    mix_fx_to_out: i32, /*< Should the effects be mixed in with the primary output? */

    /// The rvoices (upstream's voices' `rvoice` and `overflow_rvoice`).
    pub rvoice_pool: Vec<FluidRvoice>,
}

/// An entry of the `fluid_real_t **dest_bufs` array
/// `fluid_mixer_buffers_prepare()` makes (a pointer to one of the mixer's
/// buffers, or NULL).
#[derive(Clone, Copy, Debug)]
enum DestBuf {
    Null,
    DryLeft(usize),
    DryRight(usize),
    FxLeft(usize),
}

impl FluidRvoiceMixer {
    /// Translation of `fluid_rvoice_mixer_process_fx()`.
    #[inline]
    fn process_fx(&mut self, current_blockcount: i32) {
        let fx_channels_per_unit = self.buffers.fx_buf_count / self.fx_units;
        let dry_count = self.buffers.buf_count; /* dry buffers count */
        let mix_fx_to_out = self.mix_fx_to_out; /* get mix_fx_to_out mode */

        // all dry unprocessed mono input is stored in the left channel
        // (in_rev = in_ch = fx_left_buf)

        if self.with_reverb != 0 || self.with_chorus != 0 {
            let mut dry_idx: usize = 0; /* dry buffer index */

            let bufs = &mut self.buffers;
            if self.with_reverb != 0 {
                for f in 0..self.fx_units as usize {
                    if self.fx[f].reverb_on == 0 {
                        continue; /* this reverb unit is disabled */
                    }

                    let buf_idx = f * fx_channels_per_unit as usize + SYNTH_REVERB_CHANNEL; /* buffer index */
                    let mut samp_idx = buf_idx * SAMPLECOUNT; /* sample index in buffer */
                    let sample_count = current_blockcount as usize * FLUID_BUFSIZE; /* sample count to process */

                    /* in mix mode, map fx out_rev at index f to a dry buffer at index dry_idx */
                    if mix_fx_to_out != 0 {
                        /* dry buffer mapping, should be done more flexible in the future */
                        dry_idx = (f % dry_count as usize) * SAMPLECOUNT;
                    }

                    let mut i = 0;
                    while i < sample_count {
                        let in_rev = &bufs.fx_left_buf[samp_idx..samp_idx + FLUID_BUFSIZE];
                        if mix_fx_to_out != 0 {
                            let o = dry_idx + i;
                            self.fx[f].reverb.processmix(
                                in_rev,
                                &mut bufs.left_buf[o..o + FLUID_BUFSIZE],
                                &mut bufs.right_buf[o..o + FLUID_BUFSIZE],
                            );
                        } else {
                            // (in_rev aliases out_rev_l upstream)
                            let input: Vec<FluidReal> = in_rev.to_vec();
                            let (l, r) = (&mut bufs.fx_left_buf, &mut bufs.fx_right_buf);
                            self.fx[f].reverb.processreplace(
                                &input,
                                &mut l[samp_idx..samp_idx + FLUID_BUFSIZE],
                                &mut r[samp_idx..samp_idx + FLUID_BUFSIZE],
                            );
                        }
                        i += FLUID_BUFSIZE;
                        samp_idx += FLUID_BUFSIZE;
                    }
                } // implicit omp barrier - required, because out_rev_l aliases with out_ch_l
            }

            if self.with_chorus != 0 {
                for f in 0..self.fx_units as usize {
                    if self.fx[f].chorus_on == 0 {
                        continue; /* this chorus unit is disabled */
                    }

                    let buf_idx = f * fx_channels_per_unit as usize + SYNTH_CHORUS_CHANNEL;
                    let mut samp_idx = buf_idx * SAMPLECOUNT;
                    let sample_count = current_blockcount as usize * FLUID_BUFSIZE;

                    /* in mix mode, map fx out_ch at index f to a dry buffer at index dry_idx */
                    if mix_fx_to_out != 0 {
                        /* dry buffer mapping, should be done more flexible in the future */
                        dry_idx = (f % dry_count as usize) * SAMPLECOUNT;
                    }

                    let mut i = 0;
                    while i < sample_count {
                        let in_ch = &bufs.fx_left_buf[samp_idx..samp_idx + FLUID_BUFSIZE];
                        if mix_fx_to_out != 0 {
                            let o = dry_idx + i;
                            self.fx[f].chorus.processmix(
                                in_ch,
                                &mut bufs.left_buf[o..o + FLUID_BUFSIZE],
                                &mut bufs.right_buf[o..o + FLUID_BUFSIZE],
                            );
                        } else {
                            // (in_ch aliases out_ch_l upstream)
                            let input: Vec<FluidReal> = in_ch.to_vec();
                            let (l, r) = (&mut bufs.fx_left_buf, &mut bufs.fx_right_buf);
                            self.fx[f].chorus.processreplace(
                                &input,
                                &mut l[samp_idx..samp_idx + FLUID_BUFSIZE],
                                &mut r[samp_idx..samp_idx + FLUID_BUFSIZE],
                            );
                        }
                        i += FLUID_BUFSIZE;
                        samp_idx += FLUID_BUFSIZE;
                    }
                }
            }
        }
    }

    /// Translation of `fluid_mixer_buffers_prepare()`: glue to get
    /// fluid_rvoice_buffers_mix what it wants.
    #[inline]
    fn buffers_prepare(&self, outbufs: &mut Vec<DestBuf>) -> i32 {
        let fx_channels_per_unit = (self.buffers.fx_buf_count / self.fx_units) as usize;
        let offset = self.buffers.buf_count as usize * 2;
        let with_reverb = self.with_reverb;
        let with_chorus = self.with_chorus;

        outbufs.clear();
        outbufs.resize(
            2 * (self.buffers.buf_count + self.buffers.fx_buf_count) as usize,
            DestBuf::Null,
        );

        /* Set up the reverb and chorus buffers only when the effect is enabled or
         * when LADSPA is active. Nonexisting buffers are detected in the DSP loop.
         * Not sending the effect signals saves some time in that case. */

        // all the dry, non-processed mono audio for effects is to be stored in the left buffers
        for i in 0..self.fx_units as usize {
            let fx_idx = i * fx_channels_per_unit;

            outbufs[offset + fx_idx + SYNTH_REVERB_CHANNEL] = if with_reverb != 0 {
                DestBuf::FxLeft(fx_idx + SYNTH_REVERB_CHANNEL)
            } else {
                DestBuf::Null
            };

            outbufs[offset + fx_idx + SYNTH_CHORUS_CHANNEL] = if with_chorus != 0 {
                DestBuf::FxLeft(fx_idx + SYNTH_CHORUS_CHANNEL)
            } else {
                DestBuf::Null
            };
        }

        /* The output associated with a MIDI channel is wrapped around
         * using the number of audio groups as modulo divider.  This is
         * typically the number of output channels on the 'sound card',
         * as long as the LADSPA Fx unit is not used. In case of LADSPA
         * unit, think of it as subgroups on a mixer.
         *
         * For example: Assume that the number of groups is set to 2.
         * Then MIDI channel 1, 3, 5, 7 etc. go to output 1, channels 2,
         * 4, 6, 8 etc to output 2.  Or assume 3 groups: Then MIDI
         * channels 1, 4, 7, 10 etc go to output 1; 2, 5, 8, 11 etc to
         * output 2, 3, 6, 9, 12 etc to output 3.
         */
        for i in 0..self.buffers.buf_count as usize {
            outbufs[i * 2] = DestBuf::DryLeft(i);
        }

        for i in 0..self.buffers.buf_count as usize {
            outbufs[i * 2 + 1] = DestBuf::DryRight(i);
        }

        (offset as i32) + self.buffers.fx_buf_count
    }
}

/// Translation of `fluid_finish_rvoice()`.
#[inline]
fn fluid_finish_rvoice(buffers: &mut FluidMixerBuffers, polyphony: i32, rvoice: usize) {
    if (buffers.finished_voices.len() as i32) < polyphony {
        buffers.finished_voices.push(rvoice);
    } else {
        fluid_log(
            FLUID_ERR,
            "Exceeded finished voices array, try increasing polyphony",
        );
    }
}

/// Translation of `fluid_rvoice_buffers_mix()`: mix samples down from
/// internal dsp_buf to output buffers.
///
/// @param buffers Destination buffer(s)
/// @param dsp_buf Mono sample source
/// @param start_block starting sample in dsp_buf
/// @param sample_count number of samples to mix following \c start_block
/// @param dest_bufs Array of buffers to mixdown to
/// @param dest_bufcount Length of dest_bufs (i.e count of buffers)
fn fluid_rvoice_buffers_mix(
    buffers: &mut FluidRvoiceBuffers,
    dsp_buf: &[FluidReal],
    start_block: i32,
    sample_count: i32,
    dest_bufs: &[DestBuf],
    dest_bufcount: i32,
    out: &mut FluidMixerBuffers,
) {
    /* buffers count to mixdown to */
    let bufcount = buffers.count;

    /* if there is nothing to mix, return immediately */
    if sample_count <= 0 || dest_bufcount <= 0 {
        return;
    }

    let start = start_block as usize * FLUID_BUFSIZE;
    let sample_count = sample_count as usize;

    /* mixdown for each buffer */
    for i in 0..bufcount as usize {
        let target_amp = buffers.bufs[i].target_amp;
        let mut current_amp = buffers.bufs[i].current_amp;

        // get_dest_buf()
        let j = buffers.bufs[i].mapping;
        let dest = if j >= dest_bufcount || j < 0 {
            DestBuf::Null
        } else {
            dest_bufs[j as usize]
        };
        let buf: &mut [FluidReal] = match dest {
            DestBuf::Null => continue,
            DestBuf::DryLeft(g) => &mut out.left_buf[g * SAMPLECOUNT..(g + 1) * SAMPLECOUNT],
            DestBuf::DryRight(g) => &mut out.right_buf[g * SAMPLECOUNT..(g + 1) * SAMPLECOUNT],
            DestBuf::FxLeft(g) => &mut out.fx_left_buf[g * SAMPLECOUNT..(g + 1) * SAMPLECOUNT],
        };

        if current_amp == 0.0 && target_amp == 0.0 {
            continue;
        }

        let amp_incr = (target_amp - current_amp) / FLUID_BUFSIZE as FluidReal;

        /* Mixdown sample_count samples in the current buffer buf
         *
         * For the first FLUID_BUFSIZE samples, we linearly interpolate the buffers amplitude to
         * avoid clicks/pops when rapidly changing the channels panning (issue 768).
         *
         * We could have squashed this into one single loop by using an if clause within the loop body.
         * But it seems like having two separate loops is easier for compilers to understand, and therefore
         * auto-vectorizing the loops.
         */
        if sample_count < FLUID_BUFSIZE {
            // scalar loop variant, the voice will have finished afterwards
            for dsp_i in 0..sample_count {
                buf[start + dsp_i] += current_amp * dsp_buf[start + dsp_i];
                current_amp += amp_incr;
            }
        } else {
            // here goes the vectorizable loop
            for dsp_i in 0..FLUID_BUFSIZE {
                // We cannot simply increment current_amp by amp_incr during every iteration, as this would create a dependency and prevent vectorization.
                buf[start + dsp_i] +=
                    (current_amp + amp_incr * dsp_i as FluidReal) * dsp_buf[start + dsp_i];
            }

            // we have reached the target_amp
            if target_amp > 0.0 {
                /* Note, that this loop could be unrolled by FLUID_BUFSIZE elements */
                for dsp_i in FLUID_BUFSIZE..sample_count {
                    // Index by blocks (not by samples) to let the compiler know that we always start accessing
                    // buf and dsp_buf at the FLUID_BUFSIZE*sizeof(fluid_real_t) byte boundary and never somewhere
                    // in between.
                    // A good compiler should understand: Aha, so I don't need to add a peel loop when vectorizing
                    // this loop. Great.
                    buf[start + dsp_i] += target_amp * dsp_buf[start + dsp_i];
                }
            }
        }

        buffers.bufs[i].current_amp = target_amp;
    }
}

/// Translation of `fluid_mixer_buffers_render_one()`: synthesize one voice
/// and add to buffer.
///
/// NOTE: If return value is less than blockcount*FLUID_BUFSIZE, that means
/// voice has been finished, removed and possibly replaced with another voice.
#[inline]
fn fluid_mixer_buffers_render_one(
    buffers: &mut FluidMixerBuffers,
    polyphony: i32,
    rvoice_idx: usize,
    rvoice: &mut FluidRvoice,
    dest_bufs: &[DestBuf],
    dest_bufcount: i32,
    blockcount: i32,
) {
    let mut total_samples: i32 = 0;
    let mut last_block_mixed: i32 = 0;
    let mut src_buf = std::mem::take(&mut buffers.local_buf);

    for i in 0..blockcount {
        /* render one block in src_buf */
        let o = FLUID_BUFSIZE * i as usize;
        let s = fluid_rvoice_write(rvoice, &mut src_buf[o..o + FLUID_BUFSIZE]);

        if s == -1 {
            /* the voice is silent, mix back all the previously rendered sound */
            fluid_rvoice_buffers_mix(
                &mut rvoice.buffers,
                &src_buf,
                last_block_mixed,
                total_samples - (last_block_mixed * FLUID_BUFSIZE as i32),
                dest_bufs,
                dest_bufcount,
                buffers,
            );

            last_block_mixed = i + 1; /* future block start index to mix from */
            total_samples += FLUID_BUFSIZE as i32; /* accumulate samples count rendered */
        } else {
            /* the voice wasn't quiet. Some samples have been rendered [0..FLUID_BUFSIZE] */
            total_samples += s;

            if s < FLUID_BUFSIZE as i32 {
                /* voice has finished */
                break;
            }
        }
    }

    /* Now mix the remaining blocks from last_block_mixed to total_sample */
    fluid_rvoice_buffers_mix(
        &mut rvoice.buffers,
        &src_buf,
        last_block_mixed,
        total_samples - (last_block_mixed * FLUID_BUFSIZE as i32),
        dest_bufs,
        dest_bufcount,
        buffers,
    );

    buffers.local_buf = src_buf;

    if total_samples < blockcount * FLUID_BUFSIZE as i32 {
        /* voice has finished */
        fluid_finish_rvoice(buffers, polyphony, rvoice_idx);
    }
}

/// Translation of `fluid_rvoice_mixer_add_voice()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_mixer_add_voice(
    mixer: &mut FluidRvoiceMixer,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let RvoicePtr::Rvoice(voice) = param[0].ptr else {
        return;
    };

    if mixer.active_voices < mixer.polyphony {
        let n = mixer.active_voices as usize;
        mixer.rvoices[n] = voice;
        mixer.active_voices += 1;
        return; // success
    }

    /* See if any voices just finished, if so, take its place.
    This can happen in voice overflow conditions. */
    for i in 0..mixer.active_voices as usize {
        if mixer.rvoices[i] == voice {
            fluid_log(
                FLUID_ERR,
                "Internal error: Trying to replace an existing rvoice in fluid_rvoice_mixer_add_voice?!",
            );
            return;
        }

        if mixer.rvoice_pool[mixer.rvoices[i]].envlfo.volenv.section == FLUID_VOICE_ENVFINISHED {
            let finished = mixer.rvoices[i];
            fluid_finish_rvoice(&mut mixer.buffers, mixer.polyphony, finished);
            mixer.rvoices[i] = voice;
            return; // success
        }
    }

    /* This should never happen */
    fluid_log(
        FLUID_ERR,
        "Trying to exceed polyphony in fluid_rvoice_mixer_add_voice",
    );
}

/// Translation of `fluid_mixer_buffers_update_polyphony()`.
fn fluid_mixer_buffers_update_polyphony(buffers: &mut FluidMixerBuffers, value: i32) -> bool {
    if buffers.finished_voices.len() as i32 > value {
        return false;
    }

    let value = value.max(0) as usize;
    if buffers.finished_voices.capacity() < value
        && buffers
            .finished_voices
            .try_reserve_exact(value - buffers.finished_voices.len())
            .is_err()
    {
        return false;
    }
    true
}

/// Translation of `fluid_rvoice_mixer_set_polyphony()` (an rvoice event
/// function): update polyphony - max number of voices (NOTE: not hard
/// realtime capable).
pub(crate) fn fluid_rvoice_mixer_set_polyphony(
    handler: &mut FluidRvoiceMixer,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let value = param[0].i;

    if handler.active_voices > value {
        return /*FLUID_FAILED*/;
    }

    let n = value.max(0) as usize;
    if n > handler.rvoices.len()
        && handler
            .rvoices
            .try_reserve_exact(n - handler.rvoices.len())
            .is_err()
    {
        return /*FLUID_FAILED*/;
    }
    handler.rvoices.resize(n, 0);

    if !fluid_mixer_buffers_update_polyphony(&mut handler.buffers, value) {
        return /*FLUID_FAILED*/;
    }

    handler.polyphony = value;
    /*return FLUID_OK*/
}

/// Translation of `fluid_render_loop_singlethread()`.
fn fluid_render_loop_singlethread(mixer: &mut FluidRvoiceMixer, blockcount: i32) {
    let mut bufs = Vec::new();
    let bufcount = mixer.buffers_prepare(&mut bufs);

    for i in 0..mixer.active_voices as usize {
        let idx = mixer.rvoices[i];
        let FluidRvoiceMixer {
            buffers,
            rvoice_pool,
            polyphony,
            ..
        } = mixer;
        fluid_mixer_buffers_render_one(
            buffers,
            *polyphony,
            idx,
            &mut rvoice_pool[idx],
            &bufs,
            bufcount,
            blockcount,
        );
    }
}

/// Translation of `fluid_mixer_buffers_zero()`.
#[inline]
fn fluid_mixer_buffers_zero(buffers: &mut FluidMixerBuffers, current_blockcount: i32) {
    let size = current_blockcount as usize * FLUID_BUFSIZE;

    /* TODO: Optimize by only zero out the buffers we actually use later on. */
    let buf_count = buffers.buf_count as usize;
    let fx_buf_count = buffers.fx_buf_count as usize;

    for i in 0..buf_count {
        buffers.left_buf[i * SAMPLECOUNT..i * SAMPLECOUNT + size].fill(0.0);
        buffers.right_buf[i * SAMPLECOUNT..i * SAMPLECOUNT + size].fill(0.0);
    }

    for i in 0..fx_buf_count {
        buffers.fx_left_buf[i * SAMPLECOUNT..i * SAMPLECOUNT + size].fill(0.0);
        buffers.fx_right_buf[i * SAMPLECOUNT..i * SAMPLECOUNT + size].fill(0.0);
    }
}

/// An allocation of `n` zeroed samples (`FLUID_ARRAY_ALIGNED()`), or
/// `None` if out of memory.
fn alloc_samples(n: usize) -> Option<Vec<FluidReal>> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    v.resize(n, 0.0);
    Some(v)
}

/// Translation of `fluid_mixer_buffers_init()`.
fn fluid_mixer_buffers_init(buffers: &mut FluidMixerBuffers, polyphony: i32) -> bool {
    /* Local mono voice buf */
    /* Left and right audio buffers */
    let (Some(local), Some(left), Some(right)) = (
        alloc_samples(SAMPLECOUNT),
        alloc_samples(buffers.buf_count as usize * SAMPLECOUNT),
        alloc_samples(buffers.buf_count as usize * SAMPLECOUNT),
    ) else {
        fluid_log(FLUID_ERR, "Out of memory");
        return false;
    };
    buffers.local_buf = local;
    buffers.left_buf = left;
    buffers.right_buf = right;

    /* Effects audio buffers */
    let (Some(fx_left), Some(fx_right)) = (
        alloc_samples(buffers.fx_buf_count as usize * SAMPLECOUNT),
        alloc_samples(buffers.fx_buf_count as usize * SAMPLECOUNT),
    ) else {
        fluid_log(FLUID_ERR, "Out of memory");
        return false;
    };
    buffers.fx_left_buf = fx_left;
    buffers.fx_right_buf = fx_right;

    buffers.finished_voices = Vec::new();

    if !fluid_mixer_buffers_update_polyphony(buffers, polyphony) {
        fluid_log(FLUID_ERR, "Out of memory");
        return false;
    }

    true
}

/// Translation of `fluid_rvoice_mixer_set_samplerate()` (an rvoice event
/// function). Note: Not hard realtime capable (calls malloc)
#[allow(dead_code)]
pub(crate) fn fluid_rvoice_mixer_set_samplerate(
    mixer: &mut FluidRvoiceMixer,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let samplerate = param[1].real; // because fluid_synth_update_mixer() puts real into arg2

    for fx in mixer.fx.iter_mut() {
        fx.chorus.samplerate_change(samplerate);

        fx.reverb.samplerate_change(samplerate);

        /*
          fluid_revmodel_samplerate_change() shouldn't fail if the reverb was created
          with sample_rate_max set to the maximum sample rate indicated in the settings.
          If this condition isn't respected, the reverb will continue to work but with
          lost of quality.
        */
    }
}

impl FluidRvoiceMixer {
    /// Translation of `new_fluid_rvoice_mixer()`.
    ///
    /// @param buf_count number of primary stereo buffers
    /// @param fx_buf_count number of stereo effect buffers
    pub(crate) fn new(
        buf_count: i32,
        fx_buf_count: i32,
        fx_units: i32,
        sample_rate_max: FluidReal,
        sample_rate: FluidReal,
    ) -> Option<FluidRvoiceMixer> {
        let mut mixer = FluidRvoiceMixer {
            fx: Vec::new(),
            buffers: FluidMixerBuffers::default(),
            rvoices: Vec::new(),
            polyphony: 0,
            active_voices: 0,
            current_blockcount: 0,
            fx_units,
            with_reverb: 0,
            with_chorus: 0,
            mix_fx_to_out: 0,
            rvoice_pool: Vec::new(),
        };
        mixer.buffers.buf_count = buf_count;
        mixer.buffers.fx_buf_count = fx_buf_count * fx_units;

        /* allocate the reverb module */
        if mixer
            .fx
            .try_reserve_exact(fx_units.max(0) as usize)
            .is_err()
        {
            fluid_log(FLUID_ERR, "Out of memory");
            return None;
        }

        for _ in 0..fx_units {
            /* create reverb and chorus units */
            let reverb = FluidRevmodel::new(sample_rate_max, sample_rate);
            let chorus = FluidChorus::new(sample_rate);

            let (Some(reverb), Some(chorus)) = (reverb, chorus) else {
                fluid_log(FLUID_ERR, "Out of memory");
                return None;
            };
            mixer.fx.push(FluidMixerFx {
                reverb,
                reverb_param: [0.0; FLUID_REVERB_PARAM_LAST],
                reverb_on: 0,
                chorus,
                chorus_param: [0.0; FLUID_CHORUS_PARAM_LAST],
                chorus_on: 0,
            });
        }

        let polyphony = mixer.polyphony;
        if !fluid_mixer_buffers_init(&mut mixer.buffers, polyphony) {
            return None;
        }

        Some(mixer)
    }

    /// Translation of `fluid_rvoice_mixer_set_reverb_full()`: set one or
    /// more reverb shadow parameters for one fx group.
    ///
    /// These parameters will be returned if queried.
    /// (see fluid_rvoice_mixer_reverb_get_param())
    ///
    /// @param fx_group index of the fx group to which parameters must be set.
    ///  must be in the range [-1..mixer->fx_units[. If -1 the changes are applied to
    ///  all fx units.
    /// @param set Flags indicating which parameters should be set (#fluid_revmodel_set_t)
    /// @param values table of parameters values.
    pub(crate) fn set_reverb_full(&mut self, mut fx_group: i32, set: i32, values: &[f64]) {
        let mut nr_units = self.fx_units;

        if fx_group >= 0 {
            /* apply parameters to this fx group only */
            nr_units = fx_group + 1;
        } else {
            /* apply parameters to all fx groups */
            fx_group = 0;
        }

        while fx_group < nr_units {
            for param in 0..FLUID_REVERB_PARAM_LAST {
                if set & (1 << param) != 0 {
                    self.fx[fx_group as usize].reverb_param[param] = values[param];
                }
            }
            fx_group += 1;
        }
    }

    /// Translation of `fluid_rvoice_mixer_reverb_get_param()`.
    #[allow(dead_code)]
    pub(crate) fn reverb_get_param(&self, fx_group: i32, param: usize) -> f64 {
        self.fx[fx_group as usize].reverb_param[param]
    }

    /// Translation of `fluid_rvoice_mixer_set_chorus_full()`: set one or
    /// more chorus shadow parameters for one fx group.
    ///
    /// These parameters will be returned if queried.
    /// (see fluid_rvoice_mixer_chorus_get_param())
    pub(crate) fn set_chorus_full(&mut self, mut fx_group: i32, set: i32, values: &[f64]) {
        let mut nr_units = self.fx_units;

        if fx_group >= 0 {
            /* apply parameters to this group fx only */
            nr_units = fx_group + 1;
        } else {
            /* apply parameters to all fx units*/
            fx_group = 0;
        }

        while fx_group < nr_units {
            for param in 0..FLUID_CHORUS_PARAM_LAST {
                if set & (1 << param) != 0 {
                    self.fx[fx_group as usize].chorus_param[param] = values[param];
                }
            }
            fx_group += 1;
        }
    }

    /// Translation of `fluid_rvoice_mixer_chorus_get_param()`.
    #[allow(dead_code)]
    pub(crate) fn chorus_get_param(&self, fx_group: i32, param: usize) -> f64 {
        self.fx[fx_group as usize].chorus_param[param]
    }

    /// Translation of `fluid_rvoice_mixer_set_mix_fx()`.
    pub(crate) fn set_mix_fx(&mut self, on: i32) {
        self.mix_fx_to_out = on;
    }

    /// Translation of `fluid_rvoice_mixer_get_bufcount()`.
    pub(crate) fn get_bufcount(&self) -> i32 {
        FLUID_MIXER_MAX_BUFFERS_DEFAULT as i32
    }
}

/// Translation of `fluid_rvoice_mixer_set_reverb_enabled()` (an rvoice
/// event function). @deprecated: use fluid_rvoice_mixer_reverb_enable
/// instead
#[allow(dead_code)]
pub(crate) fn fluid_rvoice_mixer_set_reverb_enabled(
    mixer: &mut FluidRvoiceMixer,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    mixer.with_reverb = param[0].i;
}

/// Translation of `fluid_rvoice_mixer_reverb_enable()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_mixer_reverb_enable(
    mixer: &mut FluidRvoiceMixer,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let mut fx_group = param[0].i; /* reverb fx group index */
    let mut on = param[1].i; /* on/off */

    let nr_units = mixer.fx_units;

    /* does on/off must be applied only to fx group at index fx_group ? */
    if fx_group >= 0 {
        mixer.fx[fx_group as usize].reverb_on = on;
    }
    /* on/off must be applied to all fx groups */
    else {
        fx_group = 0;
        while fx_group < nr_units {
            mixer.fx[fx_group as usize].reverb_on = on;
            fx_group += 1;
        }
    }

    /* set with_reverb if at least one reverb unit is on */
    fx_group = 0;
    while fx_group < nr_units {
        on = mixer.fx[fx_group as usize].reverb_on;

        if on != 0 {
            break;
        }
        fx_group += 1;
    }

    mixer.with_reverb = on;
}

/// Translation of `fluid_rvoice_mixer_set_chorus_enabled()` (an rvoice
/// event function). @deprecated: use fluid_rvoice_mixer_chorus_enable
/// instead
#[allow(dead_code)]
pub(crate) fn fluid_rvoice_mixer_set_chorus_enabled(
    mixer: &mut FluidRvoiceMixer,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    mixer.with_chorus = param[0].i;
}

/// Translation of `fluid_rvoice_mixer_chorus_enable()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_mixer_chorus_enable(
    mixer: &mut FluidRvoiceMixer,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let mut fx_group = param[0].i; /* chorus fx group index */
    let mut on = param[1].i; /* on/off */

    let nr_units = mixer.fx_units;

    /* does on/off must be applied only to fx group at index fx_group ? */
    if fx_group >= 0 {
        mixer.fx[fx_group as usize].chorus_on = on;
    }
    /* on/off must be applied to all fx groups */
    else {
        fx_group = 0;
        while fx_group < nr_units {
            mixer.fx[fx_group as usize].chorus_on = on;
            fx_group += 1;
        }
    }

    /* set with_chorus if at least one chorus unit is on */
    fx_group = 0;
    while fx_group < nr_units {
        on = mixer.fx[fx_group as usize].chorus_on;

        if on != 0 {
            break;
        }
        fx_group += 1;
    }

    mixer.with_chorus = on;
}

/// Translation of `fluid_rvoice_mixer_set_chorus_params()` (an rvoice
/// event function).
pub(crate) fn fluid_rvoice_mixer_set_chorus_params(
    mixer: &mut FluidRvoiceMixer,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let mut i = param[0].i;
    let set = param[1].i;
    let nr = param[2].i;
    let level = param[3].real;
    let speed = param[4].real;
    let depth_ms = param[5].real;
    let type_ = param[6].i;

    let mut nr_units = mixer.fx_units;

    /* does parameters must be applied only to fx group i ? */
    if i >= 0 {
        nr_units = i + 1;
    } else {
        i = 0; /* parameters must be applied to all fx groups */
    }

    while i < nr_units {
        mixer.fx[i as usize]
            .chorus
            .set(set, nr, level, speed, depth_ms, type_);
        i += 1;
    }
}

/// Translation of `fluid_rvoice_mixer_set_reverb_params()` (an rvoice
/// event function).
pub(crate) fn fluid_rvoice_mixer_set_reverb_params(
    mixer: &mut FluidRvoiceMixer,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    let mut i = param[0].i; /* fx group index */
    let set = param[1].i;
    let roomsize = param[2].real;
    let damping = param[3].real;
    let width = param[4].real;
    let level = param[5].real;

    let mut nr_units = mixer.fx_units;

    /* does parameters change should be applied only to fx group i ? */
    if i >= 0 {
        nr_units = i + 1; /* parameters change must be applied to fx groups i */
    } else {
        i = 0; /* parameters change must be applied to all fx groups */
    }

    while i < nr_units {
        mixer.fx[i as usize]
            .reverb
            .set(set, roomsize, damping, width, level);
        i += 1;
    }
}

/// Translation of `fluid_rvoice_mixer_reset_reverb()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_mixer_reset_reverb(
    mixer: &mut FluidRvoiceMixer,
    _param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    for fx in mixer.fx.iter_mut() {
        fx.reverb.reset();
    }
}

/// Translation of `fluid_rvoice_mixer_reset_chorus()` (an rvoice event
/// function).
pub(crate) fn fluid_rvoice_mixer_reset_chorus(
    mixer: &mut FluidRvoiceMixer,
    _param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    for fx in mixer.fx.iter_mut() {
        fx.chorus.reset();
    }
}

/// Translation of `fluid_mixer_buffer_process_finished_voices()` (with
/// `fluid_rvoice_eventhandler_finished_voice_callback()` pushing into the
/// event handler's return queue).
fn fluid_mixer_buffer_process_finished_voices(
    mixer: &mut FluidRvoiceMixer,
    finished_voices_queue: &mut FluidRingbuffer<usize>,
) {
    let finished = std::mem::take(&mut mixer.buffers.finished_voices);
    for &v in finished.iter() {
        let mut av = mixer.active_voices;

        let mut j = 0;
        while j < av {
            if v == mixer.rvoices[j as usize] {
                av -= 1;

                /* Pack the array */
                if j < av {
                    mixer.rvoices[j as usize] = mixer.rvoices[av as usize];
                }
            }
            j += 1;
        }

        mixer.active_voices = av;

        // fluid_rvoice_eventhandler_finished_voice_callback()
        if let Some(vptr) = finished_voices_queue.get_inptr(0) {
            *vptr = v;
            finished_voices_queue.next_inptr(1);
        } // else: Buffer full
    }

    let mut finished = finished;
    finished.clear();
    mixer.buffers.finished_voices = finished;
}

/// Translation of `fluid_rvoice_mixer_render()`: synthesize audio into
/// buffers.
///
/// @param blockcount number of blocks to render, each having FLUID_BUFSIZE samples
/// @return number of blocks rendered
pub(crate) fn fluid_rvoice_mixer_render(
    mixer: &mut FluidRvoiceMixer,
    finished_voices_queue: &mut FluidRingbuffer<usize>,
    blockcount: i32,
) -> i32 {
    mixer.current_blockcount = blockcount;

    // Zero buffers
    fluid_mixer_buffers_zero(&mut mixer.buffers, blockcount);

    fluid_render_loop_singlethread(mixer, blockcount);

    // Process reverb & chorus
    mixer.process_fx(blockcount);

    // Call the callback and pack active voice array
    fluid_mixer_buffer_process_finished_voices(mixer, finished_voices_queue);

    blockcount
}
