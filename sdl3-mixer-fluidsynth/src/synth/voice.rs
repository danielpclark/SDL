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
// Modified 2026-10-17: translated into Rust from fluid_voice.c and
// fluid_voice.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE). A
// voice's rvoices are indices into the mixer's rvoice pool, its channel
// an index into the synth's channels; what the voice functions reach
// through `voice->eventhandler` and `voice->channel->synth` is a
// `VoiceEnv`. The sample reference counts (for unloading SoundFonts and
// dynamic sample loading) are the `Arc`s.

use std::sync::Arc;

use crate::fluidsynth_h::*;
use crate::fluidsynth_priv::{fluid_clip, params, RvoicePtr, FLUID_BUFSIZE, FLUID_NUM_MOD};
use crate::rvoice::adsr_env::*;
use crate::rvoice::iir_filter::{fluid_iir_filter_init, FluidIirSincos, FRES_MAX, FRES_MIN};
use crate::rvoice::rvoice::{
    fluid_rvoice_get_sample, fluid_rvoice_set_output_rate, FluidRvoice, FLUID_NOISE_FLOOR,
};
use crate::rvoice::rvoice_event::{
    FluidRvoiceEventhandler, RvoiceFunction as F, RvoiceObject as O,
};
use crate::sfloader::defsfont::FluidZoneRange;
use crate::sfloader::sfont::Sample;
use crate::synth::chan::{fluid_channel_is_valid_note, FluidChannel};
use crate::synth::gen::*;
use crate::synth::modulator::*;
use crate::synth::synth::{SYNTH_CHORUS_CHANNEL, SYNTH_REVERB_CHANNEL};
use crate::utils::conv::*;
use crate::utils::sys::{flog, fluid_log, FLUID_ERR, FLUID_WARN};
use crate::FluidReal;

pub(crate) const NO_CHANNEL: u8 = 0xff;

/// Translation of `fluid_overflow_prio_t`.
#[derive(Clone, Debug, Default)]
pub(crate) struct FluidOverflowPrio {
    pub percussion: f32, /* Is this voice on the drum channel? Then add this score */
    pub released: f32, /* Is this voice in release stage? Then add this score (usually negative) */
    pub sustained: f32, /* Is this voice sustained? Then add this score (usually negative) */
    pub volume: f32,   /* Multiply current (or future) volume (a value between 0 and 1) */
    pub age: f32, /* This score will be divided by the number of seconds the voice has lasted */
    pub important: f32, /* This score will be added to all important channels */
    pub important_channels: Vec<u8>, /* "important" flags indexed by MIDI channel number */
    pub num_important_channels: i32, /* Number of elements in the important_channels array */
}

/* enum fluid_voice_status */
pub(crate) const FLUID_VOICE_CLEAN: u8 = 0;
pub(crate) const FLUID_VOICE_ON: u8 = 1;
pub(crate) const FLUID_VOICE_SUSTAINED: u8 = 2; /* Sustained by Sustain pedal */
pub(crate) const FLUID_VOICE_HELD_BY_SOSTENUTO: u8 = 3; /* Sustained by Sostenuto pedal */
pub(crate) const FLUID_VOICE_OFF: u8 = 4;

/// Translation of `fluid_voice_t`.
#[derive(Clone)]
pub(crate) struct FluidVoice {
    pub id: u32, /* the id is incremented for every new noteon.
                 it's used for noteoff's  */
    pub status: u8,
    pub chan: u8, /* the channel number, quick access for channel messages */
    pub key: u8,  /* the key of the noteon event, quick access for noteoff */
    pub vel: u8,  /* the velocity of the noteon event */
    pub channel: Option<usize>, /* (its index in the synth's channels) */
    pub zone_range: Option<Arc<FluidZoneRange>>, /* instrument zone range*/
    pub sample: Option<Arc<Sample>>, /* Pointer to sample (dupe in rvoice) */
    pub overflow_sample: Option<Arc<Sample>>, /* Pointer to sample (dupe in overflow_rvoice) */

    pub start_time: u32,
    pub mod_count: i32,
    pub mod_: [FluidMod; FLUID_NUM_MOD],
    pub gen: [FluidGen; GEN_LAST],

    /* basic parameters */
    pub output_rate: FluidReal, /* the sample rate of the synthesizer (dupe in rvoice) */

    /* basic parameters */
    pub pitch: FluidReal,       /* the pitch in midicents (dupe in rvoice) */
    pub attenuation: FluidReal, /* the attenuation in centibels (dupe in rvoice) */
    pub root_pitch: FluidReal,

    /* master gain (dupe in rvoice) */
    pub synth_gain: FluidReal,

    /* pan */
    pub pan: FluidReal,

    /* balance */
    pub balance: FluidReal,

    /* reverb */
    pub reverb_send: FluidReal,

    /* chorus */
    pub chorus_send: FluidReal,

    /* rvoice control */
    pub rvoice: usize,
    pub overflow_rvoice: usize, /* Used temporarily and only in overflow situations */
    pub can_access_rvoice: bool, /* False if rvoice is being rendered in separate thread */
    pub can_access_overflow_rvoice: bool, /* False if overflow_rvoice is being rendered in separate thread */
    pub has_noteoff: bool,                /* Flag set when noteoff has been sent */
}

impl std::fmt::Debug for FluidVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FluidVoice")
            .field("id", &self.id)
            .field("status", &self.status)
            .field("chan", &self.chan)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// What the voice functions reach through `voice->eventhandler` and
/// `voice->channel` (with its `synth`).
pub(crate) struct VoiceEnv<'a> {
    pub eh: &'a mut FluidRvoiceEventhandler,
    pub channel: Option<&'a FluidChannel>,
    pub active_voice_count: &'a mut i32,
    pub audio_groups: i32,
    pub effects_groups: i32,
    pub effects_channels: i32,
    pub fromkey_portamento: i32,
    pub portamento_time_mode: i32,
    pub portamento_time_has_seen_lsb: bool,
    pub min_note_length_ticks: u32,
}

impl VoiceEnv<'_> {
    /// `voice->channel` (set by fluid_voice_init(), before the functions
    /// that read it run)
    fn channel(&self) -> &FluidChannel {
        self.channel.expect("voice with a channel")
    }
}

/// Translation of `OVERFLOW_PRIO_CANNOT_KILL`.
pub(crate) const OVERFLOW_PRIO_CANNOT_KILL: f32 = 999999.0;

/* used for filter turn off optimization - if filter cutoff is above the
specified value and filter q is below the other value, turn filter off */
#[allow(dead_code)]
const FLUID_MAX_AUDIBLE_FILTER_FC: f32 = 19000.0f32;
#[allow(dead_code)]
const FLUID_MIN_AUDIBLE_FILTER_Q: f32 = 1.2f32;

/* min vol envelope release (to stop clicks) in SoundFont timecents */
const FLUID_MIN_VOLENVRELEASE: f32 = -7200.0f32; /* ~16ms */

const INT24_MAX: i32 = 1 << (16 + 8 - 1);

/// `UPDATE_RVOICE0(proc)`
fn update_rvoice0(voice: &FluidVoice, eh: &mut FluidRvoiceEventhandler, proc_: F) {
    eh.push(proc_, O::Rvoice(voice.rvoice), params());
}

/// `UPDATE_RVOICE_GENERIC_R1(proc, obj, rarg)`
fn update_rvoice_generic_r1(eh: &mut FluidRvoiceEventhandler, proc_: F, obj: O, rarg: FluidReal) {
    let mut param = params();
    param[0].real = rarg;
    eh.push(proc_, obj, param);
}

/// `UPDATE_RVOICE_GENERIC_I1(proc, obj, iarg)`
fn update_rvoice_generic_i1(eh: &mut FluidRvoiceEventhandler, proc_: F, obj: O, iarg: i32) {
    let mut param = params();
    param[0].i = iarg;
    eh.push(proc_, obj, param);
}

/// `UPDATE_RVOICE_GENERIC_I2(proc, obj, iarg1, iarg2)`
fn update_rvoice_generic_i2(
    eh: &mut FluidRvoiceEventhandler,
    proc_: F,
    obj: O,
    iarg1: i32,
    iarg2: i32,
) {
    let mut param = params();
    param[0].i = iarg1;
    param[1].i = iarg2;
    eh.push(proc_, obj, param);
}

/// `UPDATE_RVOICE_GENERIC_IR(proc, obj, iarg, rarg)`
fn update_rvoice_generic_ir(
    eh: &mut FluidRvoiceEventhandler,
    proc_: F,
    obj: O,
    iarg: i32,
    rarg: FluidReal,
) {
    let mut param = params();
    param[0].i = iarg;
    param[1].real = rarg;
    eh.push(proc_, obj, param);
}

/// `UPDATE_RVOICE_R1(proc, arg1)`
fn update_rvoice_r1(
    voice: &FluidVoice,
    eh: &mut FluidRvoiceEventhandler,
    proc_: F,
    arg1: FluidReal,
) {
    update_rvoice_generic_r1(eh, proc_, O::Rvoice(voice.rvoice), arg1);
}

/// `UPDATE_RVOICE_I1(proc, arg1)`
fn update_rvoice_i1(voice: &FluidVoice, eh: &mut FluidRvoiceEventhandler, proc_: F, arg1: i32) {
    update_rvoice_generic_i1(eh, proc_, O::Rvoice(voice.rvoice), arg1);
}

/// `UPDATE_RVOICE_BUFFERS_AMP(proc, iarg, rarg)`
fn update_rvoice_buffers_amp(
    voice: &FluidVoice,
    eh: &mut FluidRvoiceEventhandler,
    iarg: i32,
    rarg: FluidReal,
) {
    update_rvoice_generic_ir(
        eh,
        F::RvoiceBuffersSetAmp,
        O::Buffers(voice.rvoice),
        iarg,
        rarg,
    );
}

/// Translation of `fluid_voice_update_volenv()` and
/// `fluid_voice_update_modenv()` (`volenv` picks the envelope).
#[allow(clippy::too_many_arguments)]
fn fluid_voice_update_env(
    voice: &FluidVoice,
    eh: &mut FluidRvoiceEventhandler,
    volenv: bool,
    enqueue: bool,
    section: u32,
    count: u32,
    coeff: FluidReal,
    increment: FluidReal,
    min: FluidReal,
    max: FluidReal,
) {
    let mut param = params();

    param[0].i = section as i32;
    param[1].i = count as i32;
    param[2].real = coeff;
    param[3].real = increment;
    param[4].real = min;
    param[5].real = max;

    if enqueue {
        let obj = if volenv {
            O::VolEnv(voice.rvoice)
        } else {
            O::ModEnv(voice.rvoice)
        };
        eh.push(F::AdsrEnvSetData, obj, param);
    } else {
        let rvoice = &mut eh.mixer.rvoice_pool[voice.rvoice];
        let env = if volenv {
            &mut rvoice.envlfo.volenv
        } else {
            &mut rvoice.envlfo.modenv
        };
        fluid_adsr_env_set_data(env, &param);
    }
}

/// Translation of `fluid_voice_sample_unref()`.
#[inline]
fn fluid_voice_sample_unref(sample: &mut Option<Arc<Sample>>) {
    *sample = None;
}

/// Translation of `fluid_voice_swap_rvoice()`: swaps the current rvoice
/// with the current overflow_rvoice.
fn fluid_voice_swap_rvoice(voice: &mut FluidVoice) {
    std::mem::swap(&mut voice.rvoice, &mut voice.overflow_rvoice);
    std::mem::swap(
        &mut voice.can_access_rvoice,
        &mut voice.can_access_overflow_rvoice,
    );
    voice.overflow_sample = voice.sample.clone();
}

/// Translation of `fluid_voice_initialize_rvoice()`.
fn fluid_voice_initialize_rvoice(
    voice: &FluidVoice,
    eh: &mut FluidRvoiceEventhandler,
    output_rate: FluidReal,
    sincos_table: &Arc<[FluidIirSincos]>,
) {
    let mut param = params();

    eh.mixer.rvoice_pool[voice.rvoice] = FluidRvoice::default();

    /* The 'sustain' and 'finished' segments of the volume / modulation
     * envelope are constant. They are never affected by any modulator
     * or generator. Therefore it is enough to initialize them once
     * during the lifetime of the synth.
     */
    fluid_voice_update_env(
        voice,
        eh,
        true,
        false,
        FLUID_VOICE_ENVSUSTAIN,
        0xffffffff,
        1.0,
        0.0,
        -1.0,
        2.0,
    );
    fluid_voice_update_env(
        voice,
        eh,
        true,
        false,
        FLUID_VOICE_ENVFINISHED,
        0xffffffff,
        0.0,
        0.0,
        -1.0,
        1.0,
    );
    fluid_voice_update_env(
        voice,
        eh,
        false,
        false,
        FLUID_VOICE_ENVSUSTAIN,
        0xffffffff,
        1.0,
        0.0,
        -1.0,
        2.0,
    );
    fluid_voice_update_env(
        voice,
        eh,
        false,
        false,
        FLUID_VOICE_ENVFINISHED,
        0xffffffff,
        0.0,
        0.0,
        -1.0,
        1.0,
    );

    let rvoice = &mut eh.mixer.rvoice_pool[voice.rvoice];

    param[0].i = FLUID_IIR_LOWPASS;
    param[1].i = 0;
    fluid_iir_filter_init(&mut rvoice.resonant_filter, &param);
    rvoice.resonant_filter.sincos_table = Some(sincos_table.clone());

    param[0].i = FLUID_IIR_DISABLED;
    fluid_iir_filter_init(&mut rvoice.resonant_custom_filter, &param);
    rvoice.resonant_custom_filter.sincos_table = Some(sincos_table.clone());

    param[0].real = output_rate;
    fluid_rvoice_set_output_rate(rvoice, &param);
}

/// Translation of `new_fluid_voice()` (its two rvoices are added to the
/// mixer's pool).
pub(crate) fn new_fluid_voice(
    handler: &mut FluidRvoiceEventhandler,
    output_rate: FluidReal,
    sincos_table: &Arc<[FluidIirSincos]>,
) -> Option<FluidVoice> {
    let pool = &mut handler.mixer.rvoice_pool;
    if pool.try_reserve(2).is_err() {
        fluid_log(FLUID_ERR, "Out of memory");
        return None;
    }
    let rvoice = pool.len();
    pool.push(FluidRvoice::default());
    let overflow_rvoice = pool.len();
    pool.push(FluidRvoice::default());

    let mut voice = FluidVoice {
        id: 0,
        status: FLUID_VOICE_CLEAN,
        chan: NO_CHANNEL,
        key: 0,
        vel: 0,
        channel: None,
        zone_range: None,
        sample: None,
        overflow_sample: None,
        start_time: 0,
        mod_count: 0,
        mod_: [FluidMod::default(); FLUID_NUM_MOD],
        gen: [FluidGen::default(); GEN_LAST],
        output_rate,
        pitch: 0.0,
        attenuation: 0.0,
        root_pitch: 0.0,
        synth_gain: 0.0,
        pan: 0.0,
        balance: 0.0,
        reverb_send: 0.0,
        chorus_send: 0.0,
        rvoice,
        overflow_rvoice,
        can_access_rvoice: true,
        can_access_overflow_rvoice: true,
        has_noteoff: false,
    };

    /* Initialize both the rvoice and overflow_rvoice */
    fluid_voice_initialize_rvoice(&voice, handler, output_rate, sincos_table);
    fluid_voice_swap_rvoice(&mut voice);
    fluid_voice_initialize_rvoice(&voice, handler, output_rate, sincos_table);

    Some(voice)
}

/// Translation of `fluid_voice_init()`: initialize the synthesis process.
/// inst_zone, the Instrument Zone contains the sample, Keyrange,Velrange
/// of the voice.
/// When playing legato (n1,n2) in mono mode, n2 will use n1 voices
/// as far as n2 still enters in Keyrange,Velrange of n1.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fluid_voice_init(
    voice: &mut FluidVoice,
    env: &mut VoiceEnv,
    sample: &Arc<Sample>,
    inst_zone_range: Option<Arc<FluidZoneRange>>,
    channel_idx: usize,
    channel: &FluidChannel,
    key: i32,
    vel: i32,
    id: u32,
    start_time: u32,
    gain: FluidReal,
) -> i32 {
    /* Note: The voice parameters will be initialized later, when the
     * generators have been retrieved from the sound font. Here, only
     * the 'working memory' of the voice (position in envelopes, history
     * of IIR filters, position in sample etc) is initialized. */

    if !voice.can_access_rvoice {
        if voice.can_access_overflow_rvoice {
            fluid_voice_swap_rvoice(voice);
        } else {
            fluid_log(
                FLUID_ERR,
                "Internal error: Cannot access an rvoice in fluid_voice_init!",
            );
            return FLUID_FAILED;
        }
    }

    /* We are now guaranteed to have access to the rvoice */

    if voice.sample.is_some() {
        fluid_voice_off(voice, env.eh);
    }

    voice.zone_range = inst_zone_range; /* Instrument zone range for legato */
    voice.id = id;
    voice.chan = channel.get_num() as u8;
    voice.key = key as u8;
    voice.vel = vel as u8;
    voice.channel = Some(channel_idx);
    voice.mod_count = 0;
    voice.start_time = start_time;
    voice.has_noteoff = false;
    update_rvoice0(voice, env.eh, F::RvoiceReset);

    /*
    We increment the reference count of the sample to indicate that this
    sample is about to be owned by the rvoice. This will prevent the
    unloading of the soundfont while this rvoice is playing.
    */
    env.eh.push_ptr(
        F::RvoiceSetSample,
        O::Rvoice(voice.rvoice),
        RvoicePtr::Sample(sample.clone()),
    );
    voice.sample = Some(sample.clone());

    let mut i = channel.get_interp_method();
    update_rvoice_i1(voice, env.eh, F::RvoiceSetInterpMethod, i);

    /* Set all the generators to their default value, according to SF
     * 2.01 section 8.1.3 (page 48). The value of NRPN messages are
     * copied from the channel to the voice's generators. The sound font
     * loader overwrites them. The generator values are later converted
     * into voice parameters in
     * fluid_voice_calculate_runtime_synthesis_parameters.  */
    fluid_gen_init(&mut voice.gen, Some(channel));
    update_rvoice_i1(voice, env.eh, F::RvoiceSetSamplemode, samplemode(voice));

    voice.synth_gain = gain;

    /* avoid division by zero later*/
    if voice.synth_gain < 0.0000001f32 as FluidReal {
        voice.synth_gain = 0.0000001f32 as FluidReal;
    }

    update_rvoice_r1(voice, env.eh, F::RvoiceSetSynthGain, voice.synth_gain);

    /* Set up buffer mapping, should be done more flexible in the future. */
    i = 2 * env.audio_groups;
    i += (voice.chan as i32 % env.effects_groups) * env.effects_channels;
    update_rvoice_generic_i2(
        env.eh,
        F::RvoiceBuffersSetMapping,
        O::Buffers(voice.rvoice),
        2,
        i + SYNTH_REVERB_CHANNEL as i32,
    );
    update_rvoice_generic_i2(
        env.eh,
        F::RvoiceBuffersSetMapping,
        O::Buffers(voice.rvoice),
        3,
        i + SYNTH_CHORUS_CHANNEL as i32,
    );

    i = 2 * (voice.chan as i32 % env.audio_groups);
    update_rvoice_generic_i2(
        env.eh,
        F::RvoiceBuffersSetMapping,
        O::Buffers(voice.rvoice),
        0,
        i,
    );
    update_rvoice_generic_i2(
        env.eh,
        F::RvoiceBuffersSetMapping,
        O::Buffers(voice.rvoice),
        1,
        i + 1,
    );

    FLUID_OK
}

/// Translation of `fluid_voice_set_output_rate()`: update sample rate.
///
/// @note If the voice is active, it will be turned off.
#[allow(dead_code)]
pub(crate) fn fluid_voice_set_output_rate(
    voice: &mut FluidVoice,
    eh: &mut FluidRvoiceEventhandler,
    value: FluidReal,
) {
    if fluid_voice_is_playing(voice) {
        fluid_voice_off(voice, eh);
    }

    voice.output_rate = value;
    update_rvoice_generic_r1(eh, F::RvoiceSetOutputRate, O::Rvoice(voice.rvoice), value);
    update_rvoice_generic_r1(
        eh,
        F::RvoiceSetOutputRate,
        O::Rvoice(voice.overflow_rvoice),
        value,
    );
}

/// Translation of `fluid_voice_gen_set()`: set the value of a generator
/// (a float upstream).
pub(crate) fn fluid_voice_gen_set(
    voice: &mut FluidVoice,
    eh: &mut FluidRvoiceEventhandler,
    i: usize,
    val: f32,
) {
    voice.gen[i].val = val as f64;
    voice.gen[i].flags = GEN_SET;

    if i == GEN_SAMPLEMODE {
        update_rvoice_i1(voice, eh, F::RvoiceSetSamplemode, val as i32);
    }
}

/// Translation of `fluid_voice_gen_incr()`: offset the value of a
/// generator (by a float upstream).
pub(crate) fn fluid_voice_gen_incr(voice: &mut FluidVoice, i: usize, val: f32) {
    voice.gen[i].val += val as f64;
    voice.gen[i].flags = GEN_SET;
}

/// Translation of `fluid_voice_gen_get()`: get the value of a generator.
#[allow(dead_code)]
pub(crate) fn fluid_voice_gen_get(voice: &FluidVoice, gen: usize) -> f32 {
    voice.gen[gen].val as f32
}

/// Translation of `fluid_voice_gen_value()`.
pub(crate) fn fluid_voice_gen_value(voice: &FluidVoice, num: usize) -> FluidReal {
    (voice.gen[num].val + voice.gen[num].mod_ + voice.gen[num].nrpn) as FluidReal
}

/// `_SAMPLEMODE(voice)`
#[inline]
fn samplemode(voice: &FluidVoice) -> i32 {
    voice.gen[GEN_SAMPLEMODE].val as i32
}

/// `_AVAILABLE(voice)`
#[inline]
pub(crate) fn available(voice: &FluidVoice) -> bool {
    voice.can_access_rvoice
        && ((voice.status == FLUID_VOICE_CLEAN) || (voice.status == FLUID_VOICE_OFF))
}

/// Translation of `fluid_voice_start()`.
pub(crate) fn fluid_voice_start(voice: &mut FluidVoice, env: &mut VoiceEnv) {
    /* The maximum volume of the loop is calculated and cached once for each
     * sample with its nominal loop settings. This happens, when the sample is used
     * for the first time.*/

    fluid_voice_calculate_runtime_synthesis_parameters(voice, env);

    voice.status = FLUID_VOICE_ON;

    /* Increment voice count */
    *env.active_voice_count += 1;
}

/// Translation of `fluid_voice_calculate_gain_amplitude()`: calculate the
/// amplitude of a voice.
///
/// @param gain The gain value in the range [0.0 ; 1.0]
/// @return An amplitude used by rvoice_mixer's buffers
#[inline]
fn fluid_voice_calculate_gain_amplitude(voice: &FluidVoice, gain: FluidReal) -> FluidReal {
    /* we use 24bit samples in fluid_rvoice_dsp. in order to normalize float
     * samples to [0.0;1.0] divide samples by the max. value of an int24 and
     * amplify them with the gain */
    gain * voice.synth_gain / (INT24_MAX as f32 * 1.0f32) as FluidReal
}

/// Translation of `fluid_voice_calculate_pitch()`: useful to return the
/// nominal pitch of a key.
/// The nominal pitch is dependent of voice->root_pitch,tuning, and
/// GEN_SCALETUNE generator.
/// This is useful to set the value of GEN_PITCH generator on noteOn.
/// This is useful to get the beginning/ending pitch for portamento.
pub(crate) fn fluid_voice_calculate_pitch(
    voice: &FluidVoice,
    channel: &FluidChannel,
    key: i32,
) -> FluidReal {
    let pitch: FluidReal;

    /* Now the nominal pitch of the key is returned.
     * Note about SCALETUNE: SF2.01 8.1.3 says, that this generator is a
     * non-realtime parameter. So we don't allow modulation (as opposed
     * to fluid_voice_gen_value(voice, GEN_SCALETUNE) When the scale tuning is varied,
     * one key remains fixed. Here C3 (MIDI number 60) is used.
     */
    if let Some(tuning) = channel.get_tuning() {
        let x = tuning.get_pitch((voice.root_pitch / 100.0f32 as FluidReal) as i32);
        pitch =
            voice.gen[GEN_SCALETUNE].val / 100.0f32 as FluidReal * (tuning.get_pitch(key) - x) + x;
    } else {
        pitch = voice.gen[GEN_SCALETUNE].val
            * (key as FluidReal - voice.root_pitch / 100.0f32 as FluidReal)
            + voice.root_pitch;
    }

    pitch
}

/// Translation of `fluid_voice_calculate_gen_pitch()`.
pub(crate) fn fluid_voice_calculate_gen_pitch(voice: &mut FluidVoice, channel: &FluidChannel) {
    voice.gen[GEN_PITCH].val =
        fluid_voice_calculate_pitch(voice, channel, fluid_voice_get_actual_key(voice));
}

/// Translation of `fluid_voice_calculate_runtime_synthesis_parameters()`.
///
/// in this function we calculate the values of all the parameters. the
/// parameters are converted to their most useful unit for the DSP
/// algorithm, for example, number of samples instead of
/// timecents. Some parameters keep their "perceptual" unit and
/// conversion will be done in the DSP function. This is the case, for
/// example, for the pitch since it is modulated by the controllers in
/// cents.
fn fluid_voice_calculate_runtime_synthesis_parameters(
    voice: &mut FluidVoice,
    env: &mut VoiceEnv,
) -> i32 {
    static LIST_OF_GENERATORS_TO_INITIALIZE: [usize; 37] = [
        GEN_STARTADDROFS,     /* SF2.01 page 48 #0   */
        GEN_ENDADDROFS,       /*                #1   */
        GEN_STARTLOOPADDROFS, /*                #2   */
        GEN_ENDLOOPADDROFS,   /*                #3   */
        /* GEN_STARTADDRCOARSEOFS see comment below [1]        #4   */
        GEN_MODLFOTOPITCH,    /*                #5   */
        GEN_VIBLFOTOPITCH,    /*                #6   */
        GEN_MODENVTOPITCH,    /*                #7   */
        GEN_FILTERFC,         /*                #8   */
        GEN_FILTERQ,          /*                #9   */
        GEN_MODLFOTOFILTERFC, /*                #10  */
        GEN_MODENVTOFILTERFC, /*                #11  */
        /* GEN_ENDADDRCOARSEOFS [1]                            #12  */
        GEN_MODLFOTOVOL, /*                #13  */
        /* not defined                                         #14  */
        GEN_CHORUSSEND, /*                #15  */
        GEN_REVERBSEND, /*                #16  */
        GEN_PAN,        /*                #17  */
        /* not defined                                         #18  */
        /* not defined                                         #19  */
        /* not defined                                         #20  */
        GEN_MODLFODELAY,  /*                #21  */
        GEN_MODLFOFREQ,   /*                #22  */
        GEN_VIBLFODELAY,  /*                #23  */
        GEN_VIBLFOFREQ,   /*                #24  */
        GEN_MODENVDELAY,  /*                #25  */
        GEN_MODENVATTACK, /*                #26  */
        GEN_MODENVHOLD,   /*                #27  */
        GEN_MODENVDECAY,  /*                #28  */
        /* GEN_MODENVSUSTAIN [1]                               #29  */
        GEN_MODENVRELEASE, /*                #30  */
        /* GEN_KEYTOMODENVHOLD [1]                             #31  */
        /* GEN_KEYTOMODENVDECAY [1]                            #32  */
        GEN_VOLENVDELAY,  /*                #33  */
        GEN_VOLENVATTACK, /*                #34  */
        GEN_VOLENVHOLD,   /*                #35  */
        GEN_VOLENVDECAY,  /*                #36  */
        /* GEN_VOLENVSUSTAIN [1]                               #37  */
        GEN_VOLENVRELEASE, /*                #38  */
        /* GEN_KEYTOVOLENVHOLD [1]                             #39  */
        /* GEN_KEYTOVOLENVDECAY [1]                            #40  */
        /* GEN_STARTLOOPADDRCOARSEOFS [1]                      #45  */
        GEN_KEYNUM,      /*                #46  */
        GEN_VELOCITY,    /*                #47  */
        GEN_ATTENUATION, /*                #48  */
        /* GEN_ENDLOOPADDRCOARSEOFS [1]                        #50  */
        /* GEN_COARSETUNE           [1]                        #51  */
        /* GEN_FINETUNE             [1]                        #52  */
        GEN_OVERRIDEROOTKEY, /*                #58  */
        GEN_PITCH,           /*                ---  */
        GEN_CUSTOM_BALANCE,  /*                ---  */
        GEN_CUSTOM_FILTERFC, /*                ---  */
        GEN_CUSTOM_FILTERQ,  /*                ---  */
    ];

    /* When the voice is made ready for the synthesis process, a lot of
     * voice-internal parameters have to be calculated.
     *
     * At this point, the sound font has already set the -nominal- value
     * for all generators (excluding GEN_PITCH). Most generators can be
     * modulated - they include a nominal value and an offset (which
     * changes with velocity, note number, channel parameters like
     * aftertouch, mod wheel...) Now this offset will be calculated as
     * follows:
     *
     *  - Process each modulator once.
     *  - Calculate its output value.
     *  - Find the target generator.
     *  - Add the output value to the modulation value of the generator.
     *
     * Note: The generators have been initialized with
     * fluid_gen_init().
     */

    let channel = env.channel();
    for i in 0..voice.mod_count as usize {
        let mod_ = voice.mod_[i];
        let modval = fluid_mod_get_value(&mod_, voice, channel);
        let dest_gen_index = mod_.dest as usize;
        // Note (upstream): a modulator destination past the generators
        // (from a SoundFont file) writes out of bounds; it is ignored here.
        if let Some(dest_gen) = voice.gen.get_mut(dest_gen_index) {
            dest_gen.mod_ += modval;
        }
        /*      fluid_dump_modulator(mod); */
    }

    /* Now the generators are initialized, nominal and modulation value.
     * The voice parameters (which depend on generators) are calculated
     * with fluid_voice_update_param. Processing the list of generator
     * changes will calculate each voice parameter once.
     *
     * Note [1]: Some voice parameters depend on several generators. For
     * example, the pitch depends on GEN_COARSETUNE, GEN_FINETUNE and
     * GEN_PITCH.  voice->pitch.  Unnecessary recalculation is avoided
     * by removing all but one generator from the list of voice
     * parameters.  Same with GEN_XXX and GEN_XXXCOARSE: the
     * initialisation list contains only GEN_XXX.
     */

    /* Calculate the voice parameter(s) dependent on each generator. */
    for &gen in &LIST_OF_GENERATORS_TO_INITIALIZE {
        fluid_voice_update_param(voice, env, gen);
    }

    /* Start portamento if enabled */
    {
        /* fromkey note comes from "GetFromKeyPortamentoLegato()" detector.
        When fromkey is set to ValidNote , portamento is started */
        /* Return fromkey portamento */
        let fromkey = env.fromkey_portamento;

        if fluid_channel_is_valid_note(fromkey) {
            /* Send portamento parameters to the voice dsp */
            let tokey = fluid_voice_get_actual_key(voice);
            fluid_voice_update_portamento(voice, env, fromkey, tokey);
        }
    }

    /* Make an estimate on how loud this voice can get at any time (attenuation). */
    let lower = fluid_voice_get_lower_boundary_for_attenuation(voice, env.channel());
    update_rvoice_r1(voice, env.eh, F::RvoiceSetMinAttenuationCb, lower);
    FLUID_OK
}

/// Translation of `calculate_hold_decay_buffers()`.
fn calculate_hold_decay_buffers(
    voice: &FluidVoice,
    channel: &FluidChannel,
    gen_base: usize,
    gen_key2base: usize,
    is_decay: bool,
) -> i32 {
    /* Purpose:
     *
     * Returns the number of DSP loops, that correspond to the hold
     * (is_decay=0) or decay (is_decay=1) time.
     * gen_base=GEN_VOLENVHOLD, GEN_VOLENVDECAY, GEN_MODENVHOLD,
     * GEN_MODENVDECAY gen_key2base=GEN_KEYTOVOLENVHOLD,
     * GEN_KEYTOVOLENVDECAY, GEN_KEYTOMODENVHOLD, GEN_KEYTOMODENVDECAY
     */

    /* SF2.01 section 8.4.3 # 31, 32, 39, 40
     * GEN_KEYTOxxxENVxxx uses key 60 as 'origin'.
     * The unit of the generator is timecents per key number.
     * If KEYTOxxxENVxxx is 100, a key one octave over key 60 (72)
     * will cause (60-72)*100=-1200 timecents of time variation.
     * The time is cut in half.
     */

    let keysteps: FluidReal = 60.0f32 as FluidReal
        - channel.get_key_pitch(fluid_voice_get_actual_key(voice)) / 100.0f32 as FluidReal;
    let mut timecents: FluidReal = fluid_voice_gen_value(voice, gen_base)
        + fluid_voice_gen_value(voice, gen_key2base) * keysteps;

    /* Range checking */
    if is_decay {
        /* SF 2.01 section 8.1.3 # 28, 36 */
        if timecents > 8000.0 {
            timecents = 8000.0;
        }
    } else {
        /* SF 2.01 section 8.1.3 # 27, 35 */
        if timecents > 5000.0 {
            timecents = 5000.0;
        }

        /* SF 2.01 section 8.1.2 # 27, 35:
         * The most negative number indicates no hold time
         */
        if timecents <= -32768.0 {
            return 0;
        }
    }

    /* SF 2.01 section 8.1.3 # 27, 28, 35, 36 */
    if timecents < -12000.0 {
        timecents = -12000.0;
    }

    let seconds = fluid_tc2sec(timecents);
    /* Each DSP loop processes FLUID_BUFSIZE samples. */

    /* round to next full number of buffers */
    ((voice.output_rate as FluidReal * seconds) / FLUID_BUFSIZE as FluidReal + 0.5f32 as FluidReal)
        as i32
}

/// `NUM_BUFFERS_DELAY()`, `NUM_BUFFERS_ATTACK()` and
/// `NUM_BUFFERS_RELEASE()`: conversion functions differ in range limit
#[inline]
fn num_buffers(voice: &FluidVoice, seconds: FluidReal) -> u32 {
    (voice.output_rate * seconds / FLUID_BUFSIZE as FluidReal) as u32
}

/// Translation of `fluid_voice_update_param()`: update all the synthesis
/// parameters which depend on generator \a gen.
///
/// The value of a generator (gen) has changed.  (The different
/// generators are listed in fluidsynth.h, or in SF2.01 page 48-49)
/// Now the dependent 'voice' parameters are calculated.
///
/// fluid_voice_update_param can be called during the setup of the
/// voice (to calculate the initial value for a voice parameter), or
/// during its operation (a generator has been changed due to
/// realtime parameter modifications like pitch-bend).
///
/// Note: The generator holds three values: The base value .val, an
/// offset caused by modulators .mod, and an offset caused by the
/// NRPN system. fluid_voice_gen_value(voice, generator_enumerator) returns the sum
/// of all three.
///
/// Calling this function is only necessary after changing a generator of an already playing voice.
pub(crate) fn fluid_voice_update_param(voice: &mut FluidVoice, env: &mut VoiceEnv, gen: usize) {
    let count: u32;
    let z: u32;
    if gen >= GEN_LAST {
        // Note (upstream): a modulator destination past the generators
        // (from a SoundFont file) reads out of bounds here in C; here it
        // returns.
        return;
    }
    let mut x: FluidReal = fluid_voice_gen_value(voice, gen);
    let eh = &mut *env.eh;

    match gen {
        GEN_PAN | GEN_CUSTOM_BALANCE => {
            /* range checking is done in the fluid_pan and fluid_balance functions */
            voice.pan = fluid_voice_gen_value(voice, GEN_PAN);
            voice.balance = fluid_voice_gen_value(voice, GEN_CUSTOM_BALANCE);

            /* left amp */
            let amp = fluid_voice_calculate_gain_amplitude(
                voice,
                fluid_pan(voice.pan, true) * fluid_balance(voice.balance, true),
            );
            update_rvoice_buffers_amp(voice, eh, 0, amp);

            /* right amp */
            let amp = fluid_voice_calculate_gain_amplitude(
                voice,
                fluid_pan(voice.pan, false) * fluid_balance(voice.balance, false),
            );
            update_rvoice_buffers_amp(voice, eh, 1, amp);
        }

        GEN_ATTENUATION => {
            voice.attenuation = x;

            /* Range: SF2.01 section 8.1.3 # 48
             * Motivation for range checking:
             * OHPiano.SF2 sets initial attenuation to a whooping -96 dB */
            voice.attenuation = fluid_clip(voice.attenuation, 0.0, 1440.0);
            update_rvoice_r1(voice, eh, F::RvoiceSetAttenuation, voice.attenuation);
        }

        /* The pitch is calculated from three different generators.
         * Read comment in fluidsynth.h about GEN_PITCH.
         */
        GEN_PITCH | GEN_COARSETUNE | GEN_FINETUNE => {
            /* The testing for allowed range is done in 'fluid_ct2hz' */
            voice.pitch = fluid_voice_gen_value(voice, GEN_PITCH)
                + 100.0f32 as FluidReal * fluid_voice_gen_value(voice, GEN_COARSETUNE)
                + fluid_voice_gen_value(voice, GEN_FINETUNE);
            update_rvoice_r1(voice, eh, F::RvoiceSetPitch, voice.pitch);
        }

        GEN_REVERBSEND => {
            /* The generator unit is 'tenths of a percent'. */
            voice.reverb_send = x / 1000.0f32 as FluidReal;
            voice.reverb_send = fluid_clip(voice.reverb_send, 0.0, 1.0);
            let amp = fluid_voice_calculate_gain_amplitude(voice, voice.reverb_send);
            update_rvoice_buffers_amp(voice, eh, 2, amp);
        }

        GEN_CHORUSSEND => {
            /* The generator unit is 'tenths of a percent'. */
            voice.chorus_send = x / 1000.0f32 as FluidReal;
            voice.chorus_send = fluid_clip(voice.chorus_send, 0.0, 1.0);
            let amp = fluid_voice_calculate_gain_amplitude(voice, voice.chorus_send);
            update_rvoice_buffers_amp(voice, eh, 3, amp);
        }

        GEN_OVERRIDEROOTKEY => {
            /* This is a non-realtime parameter. Therefore the .mod part of the generator
             * can be neglected.
             * NOTE: origpitch sets MIDI root note while pitchadj is a fine tuning amount
             * which offsets the original rate.  This means that the fine tuning is
             * inverted with respect to the root note (so subtract it, not add).
             */
            if let Some(sample) = &voice.sample {
                if voice.gen[GEN_OVERRIDEROOTKEY].val > -1.0 {
                    //FIXME: use flag instead of -1
                    voice.root_pitch = voice.gen[GEN_OVERRIDEROOTKEY].val * 100.0f32 as FluidReal
                        - sample.pitchadj as FluidReal;
                } else {
                    voice.root_pitch = sample.origpitch as FluidReal * 100.0f32 as FluidReal
                        - sample.pitchadj as FluidReal;
                }

                x = fluid_ct2hz_real(voice.root_pitch)
                    * (voice.output_rate as FluidReal / sample.samplerate as FluidReal);
            } else {
                if voice.gen[GEN_OVERRIDEROOTKEY].val > -1.0 {
                    //FIXME: use flag instead of -1
                    voice.root_pitch = voice.gen[GEN_OVERRIDEROOTKEY].val * 100.0f32 as FluidReal;
                } else {
                    voice.root_pitch = 0.0;
                }

                x = fluid_ct2hz_real(voice.root_pitch);
            }

            /* voice->pitch depends on voice->root_pitch, so calculate voice->pitch now */
            fluid_voice_calculate_gen_pitch(voice, env.channel.expect("voice with a channel"));
            update_rvoice_r1(voice, eh, F::RvoiceSetRootPitchHz, x);
        }

        GEN_FILTERFC => {
            x = fluid_clip(x, FRES_MIN as FluidReal, FRES_MAX as FluidReal);
            update_rvoice_generic_r1(eh, F::IirFilterSetFres, O::ResonantFilter(voice.rvoice), x);
        }

        GEN_FILTERQ => {
            x = fluid_clip(x, 0.0, 960.0);
            update_rvoice_generic_r1(eh, F::IirFilterSetQ, O::ResonantFilter(voice.rvoice), x);
        }

        /* same as the two above, only for the custom filter */
        GEN_CUSTOM_FILTERFC => {
            update_rvoice_generic_r1(
                eh,
                F::IirFilterSetFres,
                O::ResonantCustomFilter(voice.rvoice),
                x,
            );
        }

        GEN_CUSTOM_FILTERQ => {
            update_rvoice_generic_r1(
                eh,
                F::IirFilterSetQ,
                O::ResonantCustomFilter(voice.rvoice),
                x,
            );
        }

        GEN_MODLFOTOPITCH => {
            x = fluid_clip(x, -12000.0, 12000.0);
            update_rvoice_r1(voice, eh, F::RvoiceSetModlfoToPitch, x);
        }

        GEN_MODLFOTOVOL => {
            x = fluid_clip(x, -960.0, 960.0);
            update_rvoice_r1(voice, eh, F::RvoiceSetModlfoToVol, x);
        }

        GEN_MODLFOTOFILTERFC => {
            x = fluid_clip(x, -12000.0, 12000.0);
            update_rvoice_r1(voice, eh, F::RvoiceSetModlfoToFc, x);
        }

        GEN_MODLFODELAY => {
            x = fluid_clip(x, -12000.0, 5000.0);
            z = (voice.output_rate * fluid_tc2sec_delay(x)) as u32;
            update_rvoice_generic_i1(eh, F::LfoSetDelay, O::ModLfo(voice.rvoice), z as i32);
        }

        GEN_MODLFOFREQ => {
            /* - the frequency is converted into a delta value, per buffer of FLUID_BUFSIZE samples
             * - the delay into a sample delay
             */
            x = fluid_clip(x, -16000.0, 4500.0);
            x = (4.0f32 * FLUID_BUFSIZE as f32) as FluidReal * fluid_ct2hz_real(x)
                / voice.output_rate;
            update_rvoice_generic_r1(eh, F::LfoSetIncr, O::ModLfo(voice.rvoice), x);
        }

        GEN_VIBLFOFREQ => {
            /* vib lfo
             *
             * - the frequency is converted into a delta value, per buffer of FLUID_BUFSIZE samples
             * - the delay into a sample delay
             */
            x = fluid_clip(x, -16000.0, 4500.0);
            x = (4.0f32 * FLUID_BUFSIZE as f32) as FluidReal * fluid_ct2hz_real(x)
                / voice.output_rate;
            update_rvoice_generic_r1(eh, F::LfoSetIncr, O::VibLfo(voice.rvoice), x);
        }

        GEN_VIBLFODELAY => {
            x = fluid_clip(x, -12000.0, 5000.0);
            z = (voice.output_rate * fluid_tc2sec_delay(x)) as u32;
            update_rvoice_generic_i1(eh, F::LfoSetDelay, O::VibLfo(voice.rvoice), z as i32);
        }

        GEN_VIBLFOTOPITCH => {
            x = fluid_clip(x, -12000.0, 12000.0);
            update_rvoice_r1(voice, eh, F::RvoiceSetViblfoToPitch, x);
        }

        GEN_KEYNUM => {
            /* GEN_KEYNUM: SF2.01 page 46, item 46
             *
             * If this generator is active, it forces the key number to its
             * value.  Non-realtime controller.
             *
             * There is a flag, which should indicate, whether a generator is
             * enabled or not.  But here we rely on the default value of -1.
             */

            /* 2017-09-02: do not change the voice's key here, otherwise it will
             * never be released on a noteoff event
             */
        }

        GEN_VELOCITY => {
            /* GEN_VELOCITY: SF2.01 page 46, item 47
             *
             * If this generator is active, it forces the velocity to its
             * value. Non-realtime controller.
             *
             * There is a flag, which should indicate, whether a generator is
             * enabled or not. But here we rely on the default value of -1.
             */
            /* 2017-09-02: do not change the voice's velocity here, use
             * fluid_voice_get_actual_velocity() to get the value of this generator
             * if active.
             */
        }

        GEN_MODENVTOPITCH => {
            x = fluid_clip(x, -12000.0, 12000.0);
            update_rvoice_r1(voice, eh, F::RvoiceSetModenvToPitch, x);
        }

        GEN_MODENVTOFILTERFC => {
            /* Range: SF2.01 section 8.1.3 # 1
             * Motivation for range checking:
             * Filter is reported to make funny noises now and then
             */
            x = fluid_clip(x, -12000.0, 12000.0);
            update_rvoice_r1(voice, eh, F::RvoiceSetModenvToFc, x);
        }

        /* sample start and ends points
         *
         * Range checking is initiated via the
         * voice->check_sample_sanity flag,
         * because it is impossible to check here:
         * During the voice setup, all modulators are processed, while
         * the voice is inactive. Therefore, illegal settings may
         * occur during the setup (for example: First move the loop
         * end point ahead of the loop start point => invalid, then
         * move the loop start point forward => valid again.
         */
        GEN_STARTADDROFS | GEN_STARTADDRCOARSEOFS => {
            /* SF2.01 section 8.1.3 # 0 */
            /* SF2.01 section 8.1.3 # 4 */
            if let Some(sample) = &voice.sample {
                let start_fine = fluid_voice_gen_value(voice, GEN_STARTADDROFS);
                let start_coar = fluid_voice_gen_value(voice, GEN_STARTADDRCOARSEOFS);

                z = sample
                    .start
                    .wrapping_add(start_fine as i32 as u32)
                    .wrapping_add(32768u32.wrapping_mul(start_coar as i32 as u32));
                update_rvoice_i1(voice, eh, F::RvoiceSetStart, z as i32);
            }
        }

        GEN_ENDADDROFS | GEN_ENDADDRCOARSEOFS => {
            /* SF2.01 section 8.1.3 # 1 */
            /* SF2.01 section 8.1.3 # 12 */
            if let Some(sample) = &voice.sample {
                let end_fine = fluid_voice_gen_value(voice, GEN_ENDADDROFS);
                let end_coar = fluid_voice_gen_value(voice, GEN_ENDADDRCOARSEOFS);

                z = sample
                    .end
                    .wrapping_add(end_fine as i32 as u32)
                    .wrapping_add(32768u32.wrapping_mul(end_coar as i32 as u32));
                update_rvoice_i1(voice, eh, F::RvoiceSetEnd, z as i32);
            }
        }

        GEN_STARTLOOPADDROFS | GEN_STARTLOOPADDRCOARSEOFS => {
            /* SF2.01 section 8.1.3 # 2 */
            /* SF2.01 section 8.1.3 # 45 */
            if let Some(sample) = &voice.sample {
                let lstart_fine = fluid_voice_gen_value(voice, GEN_STARTLOOPADDROFS);
                let lstart_coar = fluid_voice_gen_value(voice, GEN_STARTLOOPADDRCOARSEOFS);

                z = sample
                    .loopstart
                    .wrapping_add(lstart_fine as i32 as u32)
                    .wrapping_add(32768u32.wrapping_mul(lstart_coar as i32 as u32));
                update_rvoice_i1(voice, eh, F::RvoiceSetLoopstart, z as i32);
            }
        }

        GEN_ENDLOOPADDROFS | GEN_ENDLOOPADDRCOARSEOFS => {
            /* SF2.01 section 8.1.3 # 3 */
            /* SF2.01 section 8.1.3 # 50 */
            if let Some(sample) = &voice.sample {
                let lend_fine = fluid_voice_gen_value(voice, GEN_ENDLOOPADDROFS);
                let lend_coar = fluid_voice_gen_value(voice, GEN_ENDLOOPADDRCOARSEOFS);

                z = sample
                    .loopend
                    .wrapping_add(lend_fine as i32 as u32)
                    .wrapping_add(32768u32.wrapping_mul(lend_coar as i32 as u32));
                update_rvoice_i1(voice, eh, F::RvoiceSetLoopend, z as i32);
            }
        }

        /* volume envelope
         *
         * - delay and hold times are converted to absolute number of samples
         * - sustain is converted to its absolute value
         * - attack, decay and release are converted to their increment per sample
         */
        GEN_VOLENVDELAY => {
            /* SF2.01 section 8.1.3 # 33 */
            x = fluid_clip(x, -12000.0, 5000.0);
            count = num_buffers(voice, fluid_tc2sec_delay(x));
            fluid_voice_update_env(
                voice,
                eh,
                true,
                true,
                FLUID_VOICE_ENVDELAY,
                count,
                0.0,
                0.0,
                -1.0,
                1.0,
            );
        }

        GEN_VOLENVATTACK => {
            /* SF2.01 section 8.1.3 # 34 */
            x = fluid_clip(x, -12000.0, 8000.0);
            count = 1u32.wrapping_add(num_buffers(voice, fluid_tc2sec_attack(x)));
            fluid_voice_update_env(
                voice,
                eh,
                true,
                true,
                FLUID_VOICE_ENVATTACK,
                count,
                1.0,
                (1.0f32 / count as f32) as FluidReal,
                -1.0,
                1.0,
            );
        }

        GEN_VOLENVHOLD | GEN_KEYTOVOLENVHOLD => {
            /* SF2.01 section 8.1.3 # 35 */
            /* SF2.01 section 8.1.3 # 39 */
            count = calculate_hold_decay_buffers(
                voice,
                env.channel.expect("voice with a channel"),
                GEN_VOLENVHOLD,
                GEN_KEYTOVOLENVHOLD,
                false,
            ) as u32; /* 0 means: hold */
            fluid_voice_update_env(
                voice,
                eh,
                true,
                true,
                FLUID_VOICE_ENVHOLD,
                count,
                1.0,
                0.0,
                -1.0,
                2.0,
            );
        }

        GEN_VOLENVDECAY | GEN_VOLENVSUSTAIN | GEN_KEYTOVOLENVDECAY => {
            /* SF2.01 section 8.1.3 # 36 */
            /* SF2.01 section 8.1.3 # 37 */
            /* SF2.01 section 8.1.3 # 40 */
            x = 1.0f32 as FluidReal
                - 0.001f32 as FluidReal * fluid_voice_gen_value(voice, GEN_VOLENVSUSTAIN);
            x = fluid_clip(x, 0.0, 1.0);
            count = calculate_hold_decay_buffers(
                voice,
                env.channel.expect("voice with a channel"),
                GEN_VOLENVDECAY,
                GEN_KEYTOVOLENVDECAY,
                true,
            ) as u32; /* 1 for decay */
            fluid_voice_update_env(
                voice,
                eh,
                true,
                true,
                FLUID_VOICE_ENVDECAY,
                count,
                1.0,
                if count != 0 {
                    (-1.0f32 / count as f32) as FluidReal
                } else {
                    0.0
                },
                x,
                2.0,
            );
        }

        GEN_VOLENVRELEASE => {
            /* SF2.01 section 8.1.3 # 38 */
            x = fluid_clip(x, FLUID_MIN_VOLENVRELEASE as FluidReal, 8000.0);
            count = 1u32.wrapping_add(num_buffers(voice, fluid_tc2sec_release(x)));
            fluid_voice_update_env(
                voice,
                eh,
                true,
                true,
                FLUID_VOICE_ENVRELEASE,
                count,
                1.0,
                (-1.0f32 / count as f32) as FluidReal,
                0.0,
                1.0,
            );
        }

        /* Modulation envelope */
        GEN_MODENVDELAY => {
            /* SF2.01 section 8.1.3 # 25 */
            x = fluid_clip(x, -12000.0, 5000.0);
            count = num_buffers(voice, fluid_tc2sec_delay(x));
            fluid_voice_update_env(
                voice,
                eh,
                false,
                true,
                FLUID_VOICE_ENVDELAY,
                count,
                0.0,
                0.0,
                -1.0,
                1.0,
            );
        }

        GEN_MODENVATTACK => {
            /* SF2.01 section 8.1.3 # 26 */
            x = fluid_clip(x, -12000.0, 8000.0);
            count = 1u32.wrapping_add(num_buffers(voice, fluid_tc2sec_attack(x)));
            fluid_voice_update_env(
                voice,
                eh,
                false,
                true,
                FLUID_VOICE_ENVATTACK,
                count,
                1.0,
                (1.0f32 / count as f32) as FluidReal,
                -1.0,
                1.0,
            );
        }

        GEN_MODENVHOLD | GEN_KEYTOMODENVHOLD => {
            /* SF2.01 section 8.1.3 # 27 */
            /* SF2.01 section 8.1.3 # 31 */
            count = calculate_hold_decay_buffers(
                voice,
                env.channel.expect("voice with a channel"),
                GEN_MODENVHOLD,
                GEN_KEYTOMODENVHOLD,
                false,
            ) as u32; /* 1 means: hold */
            fluid_voice_update_env(
                voice,
                eh,
                false,
                true,
                FLUID_VOICE_ENVHOLD,
                count,
                1.0,
                0.0,
                -1.0,
                2.0,
            );
        }

        GEN_MODENVDECAY | GEN_MODENVSUSTAIN | GEN_KEYTOMODENVDECAY => {
            /* SF 2.01 section 8.1.3 # 28 */
            /* SF 2.01 section 8.1.3 # 29 */
            /* SF 2.01 section 8.1.3 # 32 */
            count = calculate_hold_decay_buffers(
                voice,
                env.channel.expect("voice with a channel"),
                GEN_MODENVDECAY,
                GEN_KEYTOMODENVDECAY,
                true,
            ) as u32; /* 1 for decay */
            x = 1.0f32 as FluidReal
                - 0.001f32 as FluidReal * fluid_voice_gen_value(voice, GEN_MODENVSUSTAIN);
            x = fluid_clip(x, 0.0, 1.0);
            fluid_voice_update_env(
                voice,
                eh,
                false,
                true,
                FLUID_VOICE_ENVDECAY,
                count,
                1.0,
                if count != 0 {
                    (-1.0f32 / count as f32) as FluidReal
                } else {
                    0.0
                },
                x,
                2.0,
            );
        }

        GEN_MODENVRELEASE => {
            /* SF 2.01 section 8.1.3 # 30 */
            x = fluid_clip(x, -12000.0, 8000.0);
            count = 1u32.wrapping_add(num_buffers(voice, fluid_tc2sec_release(x)));
            fluid_voice_update_env(
                voice,
                eh,
                false,
                true,
                FLUID_VOICE_ENVRELEASE,
                count,
                1.0,
                (-1.0f32 / count as f32) as FluidReal,
                0.0,
                2.0,
            );
        }

        _ => {}
    } /* switch gen */
}

/* bit table for each generator being updated. The bits are packed in variables
 Each variable have NBR_BIT_BY_VAR bits represented by NBR_BIT_BY_VAR_LN2.
 The size of the table is the number of variables: SIZE_UPDATED_GEN_BIT.

 Note: In this implementation NBR_BIT_BY_VAR_LN2 is set to 5 (convenient for 32 bits cpu)
 but this could be set to 6 for 64 bits cpu.
*/

const NBR_BIT_BY_VAR_LN2: u32 = 5; /* for 32 bits variables */
const NBR_BIT_BY_VAR: u32 = 1 << NBR_BIT_BY_VAR_LN2;
const NBR_BIT_BY_VAR_ANDMASK: u32 = NBR_BIT_BY_VAR - 1;
const SIZE_UPDATED_GEN_BIT: usize =
    ((GEN_LAST as u32 + NBR_BIT_BY_VAR_ANDMASK) / NBR_BIT_BY_VAR) as usize;

/// `is_gen_updated()`
#[inline]
fn is_gen_updated(bit: &[u32; SIZE_UPDATED_GEN_BIT], gen: u32) -> bool {
    bit[(gen >> NBR_BIT_BY_VAR_LN2) as usize] & (1 << (gen & NBR_BIT_BY_VAR_ANDMASK)) != 0
}

/// `set_gen_updated()`
#[inline]
fn set_gen_updated(bit: &mut [u32; SIZE_UPDATED_GEN_BIT], gen: u32) {
    bit[(gen >> NBR_BIT_BY_VAR_LN2) as usize] |= 1 << (gen & NBR_BIT_BY_VAR_ANDMASK);
}

/// Translation of `fluid_voice_modulate()`: recalculate voice parameters
/// for a given control.
///
/// @param cc flag to distinguish between a continuous control and a channel control (pitch bend, ...)
/// @param ctrl the control number:
///   when >=0, only modulators's destination having ctrl as source are updated.
///   when -1, all modulators's destination are updated (regardless of ctrl).
///
/// In this implementation, I want to make sure that all controllers
/// are event based: the parameter values of the DSP algorithm should
/// only be updates when a controller event arrived and not at every
/// iteration of the audio cycle (which would probably be feasible if
/// the synth was made in silicon).
///
/// The update is done in two steps:
///
/// - step 1: first, we look for all the modulators that have the changed
///   controller as a source. This will yield a generator that will be changed
///   because of the controller event.
///
/// - step 2: For this generator, calculate its new value. This is the
///   sum of its original value plus the values of all the attached modulators.
///   The generator flag is set to indicate the parameters must be updated.
///   This avoid the risk to call 'fluid_voice_update_param' several
///   times for the same generator if several modulators have that generator as
///   destination. So every changed generators are updated only once.
pub(crate) fn fluid_voice_modulate(
    voice: &mut FluidVoice,
    env: &mut VoiceEnv,
    cc: bool,
    ctrl: i32,
) -> i32 {
    /* Clears registered bits table of updated generators */
    let mut updated_gen_bit = [0u32; SIZE_UPDATED_GEN_BIT];

    /*    printf("Chan=%d, CC=%d, Src=%d, Val=%d\n", voice->channel->channum, cc, ctrl, val); */

    for i in 0..voice.mod_count as usize {
        let mod_ = voice.mod_[i];

        /* step 1: find all the modulators that have the changed controller
        as input source. When ctrl is -1 all modulators destination
        are updated */
        if ctrl < 0 || fluid_mod_has_source(&mod_, cc, ctrl) {
            let gen = mod_.dest as u32;

            // Note (upstream): a modulator destination past the
            // generators (from a SoundFont file) reads and writes out of
            // bounds here in C; it is ignored here.
            if gen as usize >= GEN_LAST {
                continue;
            }

            /* Skip if this generator has already been updated */
            if !is_gen_updated(&updated_gen_bit, gen) {
                let mut modval: FluidReal = 0.0;

                /* step 2: for every attached modulator, calculate the modulation
                 * value for the generator gen */
                let channel = env.channel();
                for k in 0..voice.mod_count as usize {
                    if fluid_mod_has_dest(&voice.mod_[k], gen as i32) {
                        modval += fluid_mod_get_value(&voice.mod_[k], voice, channel);
                    }
                }

                fluid_gen_set_mod(&mut voice.gen[gen as usize], modval);

                /* now recalculate the parameter values that are derived from the
                generator */
                fluid_voice_update_param(voice, env, gen as usize);

                /* set the bit that indicates this generator is updated */
                set_gen_updated(&mut updated_gen_bit, gen);
            }
        }
    }

    FLUID_OK
}

/// Translation of `fluid_voice_modulate_all()`: update all the
/// modulators.
///
/// This function is called after a ALL_CTRL_OFF MIDI message has been received (CC 121).
/// All destinations of all modulators will be updated.
pub(crate) fn fluid_voice_modulate_all(voice: &mut FluidVoice, env: &mut VoiceEnv) -> i32 {
    fluid_voice_modulate(voice, env, false, -1)
}

/* legato update functions --------------------------------------------------*/

/// Translation of `fluid_voice_update_portamento()`: updates voice
/// portamento parameters.
///
/// @voice voice the synthesis voice
/// @fromkey the beginning pitch of portamento.
/// @tokey the ending pitch of portamento.
///
/// The function calculates pitch offset and increment, then these parameters
/// are send to the dsp.
pub(crate) fn fluid_voice_update_portamento(
    voice: &mut FluidVoice,
    env: &mut VoiceEnv,
    fromkey: i32,
    tokey: i32,
) {
    let channel = env.channel();

    /* calculates pitch offset */
    let pitch_beg = fluid_voice_calculate_pitch(voice, channel, fromkey);
    let pitch_end = fluid_voice_calculate_pitch(voice, channel, tokey);
    let pitchoffset = pitch_beg - pitch_end;
    let tm = env.portamento_time_mode;

    /* Calculates increment countinc */
    /* Increment is function of PortamentoTime (ms)*/
    let ms: FluidReal =
        channel.portamentotime_with_mode(tm, env.portamento_time_has_seen_lsb, fromkey, tokey)
            as FluidReal;

    let countinc = ((voice.output_rate as FluidReal * 0.001f32 as FluidReal * ms)
        / FLUID_BUFSIZE as FluidReal
        + 0.5f32 as FluidReal) as u32;

    /* Send portamento parameters to the voice dsp */
    update_rvoice_generic_ir(
        env.eh,
        F::RvoiceSetPortamento,
        O::Rvoice(voice.rvoice),
        countinc as i32,
        pitchoffset,
    );
}

/*---------------------------------------------------------------*/
/// Translation of `fluid_voice_update_multi_retrigger_attack()`: legato
/// mode 1: multi_retrigger
///
/// Modulates all generators dependent of key,vel.
/// Forces the voice envelopes in the attack section (legato mode 1).
///
/// @voice voice the synthesis voice
/// @tokey the new key to be applied to this voice.
/// @vel the new velocity to be applied to this voice.
pub(crate) fn fluid_voice_update_multi_retrigger_attack(
    voice: &mut FluidVoice,
    env: &mut VoiceEnv,
    tokey: i32,
    vel: i32,
) {
    voice.key = tokey as u8; /* new note */
    voice.vel = vel as u8; /* new velocity */
    /* Updates generators dependent of velocity */
    /* Modulates GEN_ATTENUATION (and others ) before calling
    fluid_rvoice_multi_retrigger_attack().*/
    fluid_voice_modulate(voice, env, false, FLUID_MOD_VELOCITY as i32);

    /* Updates generator dependent of voice->key */
    fluid_voice_update_param(voice, env, GEN_KEYTOMODENVHOLD);
    fluid_voice_update_param(voice, env, GEN_KEYTOMODENVDECAY);
    fluid_voice_update_param(voice, env, GEN_KEYTOVOLENVHOLD);
    fluid_voice_update_param(voice, env, GEN_KEYTOVOLENVDECAY);

    /* Updates pitch generator  */
    fluid_voice_calculate_gen_pitch(voice, env.channel());
    fluid_voice_update_param(voice, env, GEN_PITCH);

    /* updates adsr generator */
    update_rvoice0(voice, env.eh, F::RvoiceMultiRetriggerAttack);
}
/* end of legato update functions */

/// Translation of `fluid_voice_release()`: force the voice into release
/// stage. Useful anywhere a voice needs to be damped even if pedals
/// (sustain sostenuto) are depressed.
/// See fluid_synth_damp_voices_by_sustain_LOCAL(),
/// fluid_synth_damp_voices_by_sostenuto_LOCAL,
/// fluid_voice_noteoff().
pub(crate) fn fluid_voice_release(voice: &mut FluidVoice, env: &mut VoiceEnv) {
    let at_tick = env.min_note_length_ticks;
    update_rvoice_i1(voice, env.eh, F::RvoiceNoteoff, at_tick as i32);
    voice.has_noteoff = true; // voice is marked as noteoff occurred
}

/// Translation of `fluid_voice_noteoff()`: sending a noteoff event will
/// advance the envelopes to section 5 (release).
/// The function is convenient for polyphonic or monophonic note
pub(crate) fn fluid_voice_noteoff(voice: &mut FluidVoice, env: &mut VoiceEnv) {
    let channel = env.channel();

    /* Sustain a note under Sostenuto pedal */
    if channel.sostenuto() && channel.sostenuto_orderid > voice.id {
        // Sostenuto depressed after note
        voice.status = FLUID_VOICE_HELD_BY_SOSTENUTO;
    }
    /* Or sustain a note under Sustain pedal */
    else if channel.sustained() {
        voice.status = FLUID_VOICE_SUSTAINED;
    }
    /* Or force the voice to release stage */
    else {
        fluid_voice_release(voice, env);
    }
}

/// Translation of `fluid_voice_kill_excl()`.
///
/// Percussion sounds can be mutually exclusive: for example, a 'closed
/// hihat' sound will terminate an 'open hihat' sound ringing at the
/// same time. This behaviour is modeled using 'exclusive classes',
/// turning on a voice with an exclusive class other than 0 will kill
/// all other voices having that exclusive class within the same preset
/// or channel.  fluid_voice_kill_excl gets called, when 'voice' is to
/// be killed for that reason.
pub(crate) fn fluid_voice_kill_excl(voice: &mut FluidVoice, env: &mut VoiceEnv) -> i32 {
    if !fluid_voice_is_playing(voice) {
        return FLUID_OK;
    }

    /* Turn off the exclusive class information for this voice,
    so that it doesn't get killed twice
    */
    fluid_voice_gen_set(voice, env.eh, GEN_EXCLUSIVECLASS, 0.0);

    /* Speed up the volume envelope */
    /* The previously-used value of "-200" was found through listening tests
    with hi-hat samples. This was changed to "-2000" after "-200" was shown
    to cause too long cut times in most cases. */
    fluid_voice_gen_set(voice, env.eh, GEN_VOLENVRELEASE, -2000.0);
    fluid_voice_update_param(voice, env, GEN_VOLENVRELEASE);

    let at_tick = env.min_note_length_ticks;
    update_rvoice_i1(voice, env.eh, F::RvoiceNoteoff, at_tick as i32);

    FLUID_OK
}

/// Translation of `fluid_voice_overflow_rvoice_finished()`: unlock the
/// overflow rvoice of the voice. Decrement the reference count of the
/// sample owned by this rvoice.
///
/// Called by fluid_synth when the overflow rvoice has finished by itself.
/// Must be called also explicitly at synth destruction to ensure that
/// the soundfont be unloaded successfully.
pub(crate) fn fluid_voice_overflow_rvoice_finished(voice: &mut FluidVoice) {
    voice.can_access_overflow_rvoice = true;

    /* Decrement the reference count of the sample to indicate
    that this sample isn't owned by the rvoice anymore */
    fluid_voice_sample_unref(&mut voice.overflow_sample);
}

/// Translation of `fluid_voice_off()`: force the voice into finished
/// stage. Useful anywhere a voice needs to be cancelled from MIDI API.
pub(crate) fn fluid_voice_off(voice: &FluidVoice, eh: &mut FluidRvoiceEventhandler) {
    update_rvoice0(voice, eh, F::RvoiceVoiceoff); /* request to finish the voice */
}

/// Translation of `fluid_voice_stop()`: turns off a voice, meaning that it
/// is not processed anymore by the DSP loop, i.e. contrary part to
/// fluid_voice_start().
pub(crate) fn fluid_voice_stop(voice: &mut FluidVoice, active_voice_count: &mut i32) {
    voice.chan = NO_CHANNEL;

    /* Decrement the reference count of the sample, to indicate
    that this sample isn't owned by the rvoice anymore.
    */
    fluid_voice_sample_unref(&mut voice.sample);

    voice.status = FLUID_VOICE_OFF;
    voice.has_noteoff = true;

    /* Decrement voice count */
    *active_voice_count -= 1;
}

/// Translation of `fluid_voice_add_mod()`: adds a modulator to the voice
/// if the modulator has valid sources.
///
/// @param mod Modulator info (copied).
/// @param mode Determines how to handle an existing identical modulator.
///   #FLUID_VOICE_ADD to add (offset) the modulator amounts,
///   #FLUID_VOICE_OVERWRITE to replace the modulator,
///   #FLUID_VOICE_DEFAULT when adding a default modulator - no duplicate should
///   exist so don't check.
#[allow(dead_code)]
pub(crate) fn fluid_voice_add_mod(voice: &mut FluidVoice, mod_: &FluidMod, mode: i32) {
    /* Ignore the modulator if its sources inputs are invalid */
    if fluid_mod_check_sources(mod_, Some("api fluid_voice_add_mod mod")) {
        fluid_voice_add_mod_local(voice, mod_, mode, FLUID_NUM_MOD as i32);
    }
}

/// Translation of `fluid_voice_add_mod_local()`: adds a modulator to the
/// voice. local version of fluid_voice_add_mod function. Called at noteon
/// time.
/// @param voice, mod, mode, same as for fluid_voice_add_mod() (see above).
/// @param check_limit_count is the modulator number limit to handle with existing
///   identical modulator(i.e mode FLUID_VOICE_OVERWRITE, FLUID_VOICE_ADD).
///   - When FLUID_NUM_MOD, all the voices modulators (since the previous call)
///     are checked for identity.
///   - When check_count_limit is below the actual number of voices modulators
///     (voice->mod_count), this will restrict identity check to this number,
///     This is useful when we know by advance that there is no duplicate with
///     modulators at index above this limit. This avoid wasting cpu cycles at noteon.
pub(crate) fn fluid_voice_add_mod_local(
    voice: &mut FluidVoice,
    mod_: &FluidMod,
    mode: i32,
    mut check_limit_count: i32,
) {
    /* check_limit_count cannot be above voice->mod_count */
    if check_limit_count > voice.mod_count {
        check_limit_count = voice.mod_count;
    }

    if mode == FLUID_VOICE_ADD {
        /* if identical modulator exists, add them */
        for i in 0..check_limit_count as usize {
            if fluid_mod_test_identity(&voice.mod_[i], mod_) {
                //		printf("Adding modulator...\n");
                voice.mod_[i].amount += mod_.amount;
                return;
            }
        }
    } else if mode == FLUID_VOICE_OVERWRITE {
        /* if identical modulator exists, replace it (only the amount has to be changed) */
        for i in 0..check_limit_count as usize {
            if fluid_mod_test_identity(&voice.mod_[i], mod_) {
                //		printf("Replacing modulator...amount is %f\n",mod->amount);
                voice.mod_[i].amount = mod_.amount;
                return;
            }
        }
    }

    /* Add a new modulator (No existing modulator to add / overwrite).
    Also, default modulators (FLUID_VOICE_DEFAULT) are added without
    checking, if the same modulator already exists. */
    if (voice.mod_count as usize) < FLUID_NUM_MOD {
        let n = voice.mod_count as usize;
        voice.mod_[n].clone_from_mod(mod_);
        voice.mod_count += 1;
    } else {
        flog!(
            FLUID_WARN,
            "Voice {} has more modulators than supported, ignoring.",
            voice.id as i32
        );
    }
}

/// Translation of `fluid_voice_get_id()`: get the unique ID of the
/// noteon-event.
#[allow(dead_code)]
pub(crate) fn fluid_voice_get_id(voice: &FluidVoice) -> u32 {
    voice.id
}

/// Translation of `fluid_voice_is_playing()`: check if a voice is
/// producing sound.
///
/// Like fluid_voice_is_on() this will return TRUE once a call to
/// fluid_synth_start_voice() has been made. Contrary to fluid_voice_is_on(),
/// this might also return TRUE after the voice received a noteoff event, as it may
/// still be playing in release phase, or because it has been sustained or
/// sostenuto'ed.
pub(crate) fn fluid_voice_is_playing(voice: &FluidVoice) -> bool {
    (voice.status == FLUID_VOICE_ON)
        || fluid_voice_is_sustained(voice)
        || fluid_voice_is_sostenuto(voice)
}

/// Translation of `fluid_voice_is_on()`: check if a voice is ON.
///
/// A voice is in ON state as soon as a call to fluid_synth_start_voice() has been made
/// (which is typically done in a fluid_preset_t's noteon function).
/// A voice stays ON as long as it has not received a noteoff event.
pub(crate) fn fluid_voice_is_on(voice: &FluidVoice) -> bool {
    voice.status == FLUID_VOICE_ON && !voice.has_noteoff
}

/// Translation of `fluid_voice_is_sustained()`: check if a voice keeps
/// playing after it has received a noteoff due to being held by sustain.
pub(crate) fn fluid_voice_is_sustained(voice: &FluidVoice) -> bool {
    voice.status == FLUID_VOICE_SUSTAINED
}

/// Translation of `fluid_voice_is_sostenuto()`: check if a voice keeps
/// playing after it has received a noteoff due to being held by
/// sostenuto.
pub(crate) fn fluid_voice_is_sostenuto(voice: &FluidVoice) -> bool {
    voice.status == FLUID_VOICE_HELD_BY_SOSTENUTO
}

/// Translation of `fluid_voice_get_channel()`: return the MIDI channel the
/// voice is playing on.
///
/// @note The result of this function is only valid if the voice is playing.
pub(crate) fn fluid_voice_get_channel(voice: &FluidVoice) -> i32 {
    voice.chan as i32
}

/// Translation of `fluid_voice_get_actual_key()`: return the effective
/// MIDI key of the playing voice.
///
/// If the voice was started from an instrument which uses a fixed key generator, it returns that.
/// Otherwise returns the same value as \c fluid_voice_get_key.
pub(crate) fn fluid_voice_get_actual_key(voice: &FluidVoice) -> i32 {
    let x = fluid_voice_gen_value(voice, GEN_KEYNUM);

    if x >= 0.0 {
        x as i32
    } else {
        fluid_voice_get_key(voice)
    }
}

/// Translation of `fluid_voice_get_key()`: return the MIDI key from the
/// starting noteon event.
pub(crate) fn fluid_voice_get_key(voice: &FluidVoice) -> i32 {
    voice.key as i32
}

/// Translation of `fluid_voice_get_actual_velocity()`: return the
/// effective MIDI velocity of the playing voice.
///
/// If the voice was started from an instrument which uses a fixed velocity generator, it returns that.
/// Otherwise it returns the same value as \c fluid_voice_get_velocity.
pub(crate) fn fluid_voice_get_actual_velocity(voice: &FluidVoice) -> i32 {
    let x = fluid_voice_gen_value(voice, GEN_VELOCITY);

    if x > 0.0 {
        x as i32
    } else {
        fluid_voice_get_velocity(voice)
    }
}

/// Translation of `fluid_voice_get_velocity()`: return the MIDI velocity
/// from the starting noteon event.
pub(crate) fn fluid_voice_get_velocity(voice: &FluidVoice) -> i32 {
    voice.vel as i32
}

/// Translation of `fluid_voice_get_lower_boundary_for_attenuation()`.
///
/// Purpose:
///
/// A lower boundary for the attenuation (as in 'the minimum
/// attenuation of this voice, with volume pedals, modulators
/// etc. resulting in minimum attenuation, cannot fall below x cB) is
/// calculated.  This has to be called during fluid_voice_start, after
/// all modulators have been run on the voice once.  Also,
/// voice->attenuation has to be initialized.
/// (see fluid_voice_calculate_runtime_synthesis_parameters())
fn fluid_voice_get_lower_boundary_for_attenuation(
    voice: &FluidVoice,
    channel: &FluidChannel,
) -> FluidReal {
    let mut possible_att_reduction_cb: FluidReal = 0.0;

    for i in 0..voice.mod_count as usize {
        let mod_ = &voice.mod_[i];

        /* Modulator has attenuation as target and can change over time? */
        if (mod_.dest as usize == GEN_ATTENUATION)
            && ((mod_.flags1 & FLUID_MOD_CC) != 0
                || (mod_.flags2 & FLUID_MOD_CC) != 0
                || (mod_.src1 == FLUID_MOD_CHANNELPRESSURE)
                || (mod_.src1 == FLUID_MOD_KEYPRESSURE)
                || (mod_.src1 == FLUID_MOD_PITCHWHEEL)
                || (mod_.src2 == FLUID_MOD_CHANNELPRESSURE)
                || (mod_.src2 == FLUID_MOD_KEYPRESSURE)
                || (mod_.src2 == FLUID_MOD_PITCHWHEEL))
        {
            let current_val = fluid_mod_get_value(mod_, voice, channel);
            /* min_val is the possible minimum value for this modulator.
            it depends of 3 things :
            1)the minimum values of src1,src2 (i.e -1 if mapping is bipolar
              or 0 if mapping is unipolar).
            2)the sign of amount.
            3)absolute value of amount.

            When at least one source mapping is bipolar:
              min_val is -|amount| regardless the sign of amount.
            When both sources mapping are unipolar:
              min_val is -|amount|, if amount is negative.
              min_val is 0, if amount is positive
             */
            let mut min_val: FluidReal = mod_.amount.abs();

            /* Can this modulator produce a negative contribution? */
            if (mod_.flags1 & FLUID_MOD_BIPOLAR) != 0
                || (mod_.flags2 & FLUID_MOD_BIPOLAR) != 0
                || (mod_.amount < 0.0)
            {
                min_val = -min_val; /* min_val = - |amount|*/
            } else {
                /* No negative value possible. But still, the minimum contribution is 0. */
                min_val = 0.0;
            }

            /* For example:
             * - current_val=100
             * - min_val=-4000
             * - possible reduction contribution of this modulator = current_val - min_val = 4100
             */
            if current_val > min_val {
                possible_att_reduction_cb += current_val - min_val;
            }
        }
    }

    let mut lower_bound = voice.attenuation - possible_att_reduction_cb;

    /* SF2.01 specs do not allow negative attenuation */
    if lower_bound < 0.0 {
        lower_bound = 0.0;
    }

    lower_bound
}

/// Translation of `fluid_voice_set_param()`: set the NRPN value of a
/// generator.
pub(crate) fn fluid_voice_set_param(
    voice: &mut FluidVoice,
    env: &mut VoiceEnv,
    gen: usize,
    nrpn_value: FluidReal,
) -> i32 {
    voice.gen[gen].nrpn = nrpn_value;
    voice.gen[gen].flags = GEN_SET;
    fluid_voice_update_param(voice, env, gen);
    FLUID_OK
}

/// Translation of `fluid_voice_set_gain()`: set the gain.
#[allow(dead_code)]
pub(crate) fn fluid_voice_set_gain(
    voice: &mut FluidVoice,
    eh: &mut FluidRvoiceEventhandler,
    mut gain: FluidReal,
) -> i32 {
    /* avoid division by zero*/
    if gain < 0.0000001f32 as FluidReal {
        gain = 0.0000001f32 as FluidReal;
    }

    voice.synth_gain = gain;
    let left = fluid_voice_calculate_gain_amplitude(
        voice,
        fluid_pan(voice.pan, true) * fluid_balance(voice.balance, true),
    );
    let right = fluid_voice_calculate_gain_amplitude(
        voice,
        fluid_pan(voice.pan, false) * fluid_balance(voice.balance, false),
    );
    let reverb = fluid_voice_calculate_gain_amplitude(voice, voice.reverb_send);
    let chorus = fluid_voice_calculate_gain_amplitude(voice, voice.chorus_send);

    update_rvoice_r1(voice, eh, F::RvoiceSetSynthGain, gain);
    update_rvoice_buffers_amp(voice, eh, 0, left);
    update_rvoice_buffers_amp(voice, eh, 1, right);
    update_rvoice_buffers_amp(voice, eh, 2, reverb);
    update_rvoice_buffers_amp(voice, eh, 3, chorus);

    FLUID_OK
}

/* - Scan the loop
 * - determine the peak level
 * - Calculate, what factor will make the loop inaudible
 * - Store in sample
 */

/// Translation of `fluid_voice_optimize_sample()`: calculate the peak
/// volume of a sample for voice off optimization.
///
/// If the peak volume during the loop is known, then the voice can
/// be released earlier during the release phase. Otherwise, the
/// voice will operate (inaudibly), until the envelope is at the
/// nominal turnoff point.  So it's a good idea to call
/// fluid_voice_optimize_sample() on each sample once.
pub(crate) fn fluid_voice_optimize_sample(s: &mut Sample) -> i32 {
    let mut peak_max: i32 = 0;
    let mut peak_min: i32 = 0;
    let mut peak: i32;

    /* ignore disabled samples */
    if s.start == s.end {
        return FLUID_OK;
    }

    if !s.amplitude_that_reaches_noise_floor_is_valid {
        /* Only once */
        /* Scan the loop */
        let mut i = s.loopstart;
        while i < s.loopend {
            let val = fluid_rvoice_get_sample(&s.data, s.data24.as_deref(), i);

            if val > peak_max {
                peak_max = val;
            } else if val < peak_min {
                peak_min = val;
            }
            i += 1;
        }

        /* Determine the peak level */
        if peak_max > -peak_min {
            peak = peak_max;
        } else {
            peak = -peak_min;
        }

        if peak == 0 {
            /* Avoid division by zero */
            peak = 1;
        }

        /* Calculate what factor will make the loop inaudible
         * For example: Take a peak of 3277 (10 % of 32768).  The
         * normalized amplitude is 0.1 (10 % of 32768).  An amplitude
         * factor of 0.0001 (as opposed to the default 0.00001) will
         * drop this sample to the noise floor.
         */

        /* 16 bits => 96+4=100 dB dynamic range => 0.00001 */
        let normalized_amplitude_during_loop: FluidReal =
            peak as FluidReal / (INT24_MAX as f32 * 1.0f32) as FluidReal;
        let result: f64 = FLUID_NOISE_FLOOR / normalized_amplitude_during_loop;

        /* Store in sample */
        s.amplitude_that_reaches_noise_floor = result;
        s.amplitude_that_reaches_noise_floor_is_valid = true;
    }

    FLUID_OK
}

/// Translation of `fluid_voice_get_overflow_prio()`.
pub(crate) fn fluid_voice_get_overflow_prio(
    voice: &FluidVoice,
    channel: &FluidChannel,
    score: &FluidOverflowPrio,
    mut cur_time: u32,
) -> f32 {
    let mut this_voice_prio: f32 = 0.0;

    /* Are we already overflowing? */
    if !voice.can_access_overflow_rvoice {
        return OVERFLOW_PRIO_CANNOT_KILL;
    }

    /* Is this voice on the drum channel?
     * Then it is very important.
     * Also skip the released and sustained scores.
     */
    if channel.channel_type == CHANNEL_TYPE_DRUM {
        this_voice_prio += score.percussion;
    } else if voice.has_noteoff {
        /* Noteoff has */
        this_voice_prio += score.released;
    } else if fluid_voice_is_sustained(voice) || fluid_voice_is_sostenuto(voice) {
        /* This voice is still active, since the sustain pedal is held down.
         * Consider it less important than non-sustained channels.
         * This decision is somehow subjective. But usually the sustain pedal
         * is used to play 'more-voices-than-fingers', so it shouldn't hurt
         * if we kill one voice.
         */
        this_voice_prio += score.sustained;
    }

    /* We are not enthusiastic about releasing voices, which have just been started.
     * Otherwise hitting a chord may result in killing notes belonging to that very same
     * chord. So give newer voices a higher score. */
    if score.age != 0.0 {
        cur_time = cur_time.wrapping_sub(voice.start_time);

        if cur_time < 1 {
            cur_time = 1; // Avoid div by zero
        }

        // (float += double: the sum in double, stored as float)
        this_voice_prio = (this_voice_prio as f64
            + (score.age as f64 * voice.output_rate) / cur_time as f64)
            as f32;
    }

    /* take a rough estimate of loudness into account. Louder voices are more important. */
    if score.volume != 0.0 {
        let mut a: FluidReal = voice.attenuation;

        if voice.has_noteoff {
            // FIXME: Should take into account where on the envelope we are...?
        }

        if a < 0.1f32 as FluidReal {
            a = 0.1f32 as FluidReal; // Avoid div by zero
        }

        this_voice_prio = (this_voice_prio as f64 + score.volume as f64 / a) as f32;
    }

    /* Check if this voice is on an important channel. If so, then add the
     * score for important channels */
    let channel = fluid_voice_get_channel(voice);

    if channel < score.num_important_channels && score.important_channels[channel as usize] != 0 {
        this_voice_prio += score.important;
    }

    this_voice_prio
}

/// Translation of `fluid_voice_set_custom_filter()`.
#[allow(dead_code)]
pub(crate) fn fluid_voice_set_custom_filter(
    voice: &FluidVoice,
    eh: &mut FluidRvoiceEventhandler,
    type_: i32,
    flags: i32,
) {
    update_rvoice_generic_i2(
        eh,
        F::IirFilterInit,
        O::ResonantCustomFilter(voice.rvoice),
        type_,
        flags,
    );
}
