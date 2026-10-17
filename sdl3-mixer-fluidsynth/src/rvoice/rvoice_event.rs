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
// Modified 2026-10-17: translated into Rust from fluid_rvoice_event.c and
// fluid_rvoice_event.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

use crate::fluidsynth_priv::{RvoiceParam, RvoicePtr, MAX_EVENT_PARAMS};
use crate::rvoice::adsr_env::fluid_adsr_env_set_data;
use crate::rvoice::iir_filter::{
    fluid_iir_filter_init, fluid_iir_filter_set_fres, fluid_iir_filter_set_q,
};
use crate::rvoice::lfo::{fluid_lfo_set_delay, fluid_lfo_set_incr};
use crate::rvoice::rvoice::*;
use crate::rvoice::rvoice_mixer::*;
use crate::utils::ringbuffer::FluidRingbuffer;
use crate::utils::sys::{fluid_log, FLUID_WARN};
use crate::FluidReal;

/// The `fluid_rvoice_function_t`s events call (the C function pointers).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RvoiceFunction {
    /// (an unused queue slot)
    #[default]
    None,
    AdsrEnvSetData,
    LfoSetIncr,
    LfoSetDelay,
    IirFilterInit,
    IirFilterSetFres,
    IirFilterSetQ,
    RvoiceBuffersSetAmp,
    RvoiceBuffersSetMapping,
    RvoiceNoteoff,
    RvoiceVoiceoff,
    RvoiceReset,
    RvoiceMultiRetriggerAttack,
    RvoiceSetPortamento,
    RvoiceSetOutputRate,
    RvoiceSetInterpMethod,
    RvoiceSetRootPitchHz,
    RvoiceSetPitch,
    RvoiceSetAttenuation,
    RvoiceSetMinAttenuationCb,
    RvoiceSetViblfoToPitch,
    RvoiceSetModlfoToPitch,
    RvoiceSetModlfoToVol,
    RvoiceSetModlfoToFc,
    RvoiceSetModenvToFc,
    RvoiceSetModenvToPitch,
    RvoiceSetSynthGain,
    RvoiceSetStart,
    RvoiceSetEnd,
    RvoiceSetLoopstart,
    RvoiceSetLoopend,
    RvoiceSetSamplemode,
    RvoiceSetSample,
    MixerAddVoice,
    MixerSetSamplerate,
    MixerSetPolyphony,
    MixerSetChorusEnabled,
    MixerSetReverbEnabled,
    MixerReverbEnable,
    MixerChorusEnable,
    MixerSetChorusParams,
    MixerSetReverbParams,
    MixerResetReverb,
    MixerResetChorus,
}

/// The objects events act on (the C `void *object`): the mixer, or an
/// rvoice (by its index in the mixer's pool) or one of its parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RvoiceObject {
    #[default]
    Mixer,
    Rvoice(usize),
    Buffers(usize),
    VolEnv(usize),
    ModEnv(usize),
    ModLfo(usize),
    VibLfo(usize),
    ResonantFilter(usize),
    ResonantCustomFilter(usize),
}

/// Translation of `fluid_rvoice_event_t`.
#[derive(Clone, Default)]
pub(crate) struct FluidRvoiceEvent {
    method: RvoiceFunction,
    object: RvoiceObject,
    param: [RvoiceParam; MAX_EVENT_PARAMS],
}

/// Translation of `fluid_rvoice_eventhandler_t`: bridge between the
/// renderer thread and the midi state thread.
pub(crate) struct FluidRvoiceEventhandler {
    queue: FluidRingbuffer<FluidRvoiceEvent>, /*< List of fluid_rvoice_event_t */
    queue_stored: usize,                      /*< Extras pushed but not flushed */
    pub finished_voices: FluidRingbuffer<usize>, /*< return queue from handler, list of fluid_rvoice_t* */
    pub mixer: FluidRvoiceMixer,
}

impl std::fmt::Debug for FluidRvoiceEventhandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FluidRvoiceEventhandler")
            .finish_non_exhaustive()
    }
}

/// Translation of `fluid_rvoice_event_dispatch()`: calls the event's
/// function on its object.
pub(crate) fn fluid_rvoice_event_dispatch(
    mixer: &mut FluidRvoiceMixer,
    method: RvoiceFunction,
    object: RvoiceObject,
    param: &[RvoiceParam; MAX_EVENT_PARAMS],
) {
    use RvoiceFunction as F;
    use RvoiceObject as O;

    match object {
        O::Mixer => match method {
            F::MixerAddVoice => fluid_rvoice_mixer_add_voice(mixer, param),
            F::MixerSetSamplerate => fluid_rvoice_mixer_set_samplerate(mixer, param),
            F::MixerSetPolyphony => fluid_rvoice_mixer_set_polyphony(mixer, param),
            F::MixerSetChorusEnabled => fluid_rvoice_mixer_set_chorus_enabled(mixer, param),
            F::MixerSetReverbEnabled => fluid_rvoice_mixer_set_reverb_enabled(mixer, param),
            F::MixerReverbEnable => fluid_rvoice_mixer_reverb_enable(mixer, param),
            F::MixerChorusEnable => fluid_rvoice_mixer_chorus_enable(mixer, param),
            F::MixerSetChorusParams => fluid_rvoice_mixer_set_chorus_params(mixer, param),
            F::MixerSetReverbParams => fluid_rvoice_mixer_set_reverb_params(mixer, param),
            F::MixerResetReverb => fluid_rvoice_mixer_reset_reverb(mixer, param),
            F::MixerResetChorus => fluid_rvoice_mixer_reset_chorus(mixer, param),
            _ => {}
        },
        O::Rvoice(i) => {
            let Some(voice) = mixer.rvoice_pool.get_mut(i) else {
                return;
            };
            match method {
                F::RvoiceNoteoff => fluid_rvoice_noteoff(voice, param),
                F::RvoiceVoiceoff => fluid_rvoice_voiceoff(voice, param),
                F::RvoiceReset => fluid_rvoice_reset(voice, param),
                F::RvoiceMultiRetriggerAttack => fluid_rvoice_multi_retrigger_attack(voice, param),
                F::RvoiceSetPortamento => fluid_rvoice_set_portamento(voice, param),
                F::RvoiceSetOutputRate => fluid_rvoice_set_output_rate(voice, param),
                F::RvoiceSetInterpMethod => fluid_rvoice_set_interp_method(voice, param),
                F::RvoiceSetRootPitchHz => fluid_rvoice_set_root_pitch_hz(voice, param),
                F::RvoiceSetPitch => fluid_rvoice_set_pitch(voice, param),
                F::RvoiceSetAttenuation => fluid_rvoice_set_attenuation(voice, param),
                F::RvoiceSetMinAttenuationCb => fluid_rvoice_set_min_attenuation_cb(voice, param),
                F::RvoiceSetViblfoToPitch => fluid_rvoice_set_viblfo_to_pitch(voice, param),
                F::RvoiceSetModlfoToPitch => fluid_rvoice_set_modlfo_to_pitch(voice, param),
                F::RvoiceSetModlfoToVol => fluid_rvoice_set_modlfo_to_vol(voice, param),
                F::RvoiceSetModlfoToFc => fluid_rvoice_set_modlfo_to_fc(voice, param),
                F::RvoiceSetModenvToFc => fluid_rvoice_set_modenv_to_fc(voice, param),
                F::RvoiceSetModenvToPitch => fluid_rvoice_set_modenv_to_pitch(voice, param),
                F::RvoiceSetSynthGain => fluid_rvoice_set_synth_gain(voice, param),
                F::RvoiceSetStart => fluid_rvoice_set_start(voice, param),
                F::RvoiceSetEnd => fluid_rvoice_set_end(voice, param),
                F::RvoiceSetLoopstart => fluid_rvoice_set_loopstart(voice, param),
                F::RvoiceSetLoopend => fluid_rvoice_set_loopend(voice, param),
                F::RvoiceSetSamplemode => fluid_rvoice_set_samplemode(voice, param),
                F::RvoiceSetSample => fluid_rvoice_set_sample(voice, param),
                _ => {}
            }
        }
        O::Buffers(i) => {
            let Some(voice) = mixer.rvoice_pool.get_mut(i) else {
                return;
            };
            match method {
                F::RvoiceBuffersSetAmp => fluid_rvoice_buffers_set_amp(&mut voice.buffers, param),
                F::RvoiceBuffersSetMapping => {
                    fluid_rvoice_buffers_set_mapping(&mut voice.buffers, param)
                }
                _ => {}
            }
        }
        O::VolEnv(i) | O::ModEnv(i) => {
            let Some(voice) = mixer.rvoice_pool.get_mut(i) else {
                return;
            };
            let env = if let O::VolEnv(_) = object {
                &mut voice.envlfo.volenv
            } else {
                &mut voice.envlfo.modenv
            };
            if method == F::AdsrEnvSetData {
                fluid_adsr_env_set_data(env, param);
            }
        }
        O::ModLfo(i) | O::VibLfo(i) => {
            let Some(voice) = mixer.rvoice_pool.get_mut(i) else {
                return;
            };
            let lfo = if let O::ModLfo(_) = object {
                &mut voice.envlfo.modlfo
            } else {
                &mut voice.envlfo.viblfo
            };
            match method {
                F::LfoSetIncr => fluid_lfo_set_incr(lfo, param),
                F::LfoSetDelay => fluid_lfo_set_delay(lfo, param),
                _ => {}
            }
        }
        O::ResonantFilter(i) | O::ResonantCustomFilter(i) => {
            let Some(voice) = mixer.rvoice_pool.get_mut(i) else {
                return;
            };
            let filter = if let O::ResonantFilter(_) = object {
                &mut voice.resonant_filter
            } else {
                &mut voice.resonant_custom_filter
            };
            match method {
                F::IirFilterInit => fluid_iir_filter_init(filter, param),
                F::IirFilterSetFres => fluid_iir_filter_set_fres(filter, param),
                F::IirFilterSetQ => fluid_iir_filter_set_q(filter, param),
                _ => {}
            }
        }
    }
}

impl FluidRvoiceEventhandler {
    /// Translation of `fluid_rvoice_eventhandler_flush()`.
    #[inline]
    pub(crate) fn flush(&mut self) {
        let queue_stored = self.queue_stored;
        if queue_stored > 0 {
            self.queue_stored = 0;
            self.queue.next_inptr(queue_stored);
        }
    }

    /// Translation of `fluid_rvoice_eventhandler_get_finished_voice()`:
    /// next finished voice, or NULL if nothing in queue.
    #[inline]
    pub(crate) fn get_finished_voice(&mut self) -> Option<usize> {
        let result = *self.finished_voices.get_outptr()?;
        self.finished_voices.next_outptr();
        Some(result)
    }

    /// Translation of `fluid_rvoice_eventhandler_push_int_real()`.
    ///
    /// In order to be able to push more than one event atomically,
    /// use push for all events, then use flush to commit them to the
    /// queue. If threadsafe is false, all events are processed immediately.
    pub(crate) fn push_int_real(
        &mut self,
        method: RvoiceFunction,
        object: RvoiceObject,
        intparam: i32,
        realparam: FluidReal,
    ) -> bool {
        let mut param: [RvoiceParam; MAX_EVENT_PARAMS] = Default::default();
        param[0].i = intparam;
        param[1].real = realparam;
        self.push_local(method, object, param)
    }

    /// Translation of `fluid_rvoice_eventhandler_push()`.
    pub(crate) fn push(
        &mut self,
        method: RvoiceFunction,
        object: RvoiceObject,
        param: [RvoiceParam; MAX_EVENT_PARAMS],
    ) -> bool {
        self.push_local(method, object, param)
    }

    /// Translation of `fluid_rvoice_eventhandler_push_ptr()`.
    pub(crate) fn push_ptr(
        &mut self,
        method: RvoiceFunction,
        object: RvoiceObject,
        ptr: RvoicePtr,
    ) -> bool {
        let mut param: [RvoiceParam; MAX_EVENT_PARAMS] = Default::default();
        param[0].ptr = ptr;
        self.push_local(method, object, param)
    }

    /// Translation of `fluid_rvoice_eventhandler_push_LOCAL()`.
    fn push_local(
        &mut self,
        method: RvoiceFunction,
        object: RvoiceObject,
        param: [RvoiceParam; MAX_EVENT_PARAMS],
    ) -> bool {
        let old_queue_stored = self.queue_stored;
        self.queue_stored += 1;
        let Some(event) = self.queue.get_inptr(old_queue_stored) else {
            self.queue_stored -= 1;
            fluid_log(
                FLUID_WARN,
                "Ringbuffer full, try increasing synth.polyphony!",
            );
            return false; // Buffer full...
        };

        *event = FluidRvoiceEvent {
            method,
            object,
            param,
        };
        true
    }

    /// Translation of `fluid_rvoice_eventhandler_add_rvoice()`.
    #[inline]
    pub(crate) fn add_rvoice(&mut self, rvoice: usize) {
        self.push_ptr(
            RvoiceFunction::MixerAddVoice,
            RvoiceObject::Mixer,
            RvoicePtr::Rvoice(rvoice),
        );
    }

    /// Translation of `new_fluid_rvoice_eventhandler()`.
    pub(crate) fn new(
        queuesize: i32,
        finished_voices_size: i32,
        bufs: i32,
        fx_bufs: i32,
        fx_units: i32,
        sample_rate_max: FluidReal,
        sample_rate: FluidReal,
    ) -> Option<FluidRvoiceEventhandler> {
        let finished_voices = FluidRingbuffer::new(finished_voices_size.max(0) as usize);
        let Some(finished_voices) = finished_voices else {
            crate::utils::sys::fluid_log(crate::utils::sys::FLUID_ERR, "Out of memory");
            return None;
        };

        let queue = FluidRingbuffer::new(queuesize.max(0) as usize);
        let Some(queue) = queue else {
            crate::utils::sys::fluid_log(crate::utils::sys::FLUID_ERR, "Out of memory");
            return None;
        };

        let mixer = FluidRvoiceMixer::new(bufs, fx_bufs, fx_units, sample_rate_max, sample_rate)?;

        Some(FluidRvoiceEventhandler {
            queue,
            queue_stored: 0,
            finished_voices,
            mixer,
        })
    }

    /// Translation of `fluid_rvoice_eventhandler_dispatch_count()`.
    pub(crate) fn dispatch_count(&self) -> usize {
        self.queue.get_count()
    }

    /// Translation of `fluid_rvoice_eventhandler_dispatch_all()`: call
    /// fluid_rvoice_event_dispatch for all events in queue.
    ///
    /// @return number of events dispatched
    pub(crate) fn dispatch_all(&mut self) -> i32 {
        let mut result = 0;
        while let Some(event) = self.queue.get_outptr() {
            let method = event.method;
            let object = event.object;
            let param = std::mem::take(&mut event.param);
            fluid_rvoice_event_dispatch(&mut self.mixer, method, object, &param);
            result += 1;
            self.queue.next_outptr();
        }
        result
    }
}
