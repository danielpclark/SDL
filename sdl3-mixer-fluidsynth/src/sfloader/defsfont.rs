/* FluidSynth - A Software Synthesizer
 *
 * Copyright (C) 2003  Peter Hanappe and others.
 *
 * SoundFont file loading code borrowed from Smurf SoundFont Editor
 * Copyright (C) 1999-2001 Josh Green
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
// Modified 2026-10-17: translated into Rust from fluid_defsfont.c and
// fluid_defsfont.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).
// The loaded SoundFont is immutable and shared (`Arc`) with the synth's
// channels and voices, except for the zone ranges' `ignore` flags, which
// are atomic. Dynamic sample loading (synth.dynamic-sample-loading, off by
// default and not set by SDL_mixer) loads all samples up front here, as
// without it: what plays is the same, only when the samples are read
// differs. (Upstream's dynamic loading reopens the SoundFont through the
// file callbacks for each preset selected, which with SDL_mixer's
// callbacks reads an IOStream closed after the first load: FIXME
// (upstream).)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::fluidsynth_h::*;
use crate::fluidsynth_priv::FLUID_NUM_MOD;
use crate::sfloader::samplecache::{fluid_samplecache_load, fluid_samplecache_unload};
use crate::sfloader::sffile::*;
use crate::sfloader::sfont::{
    fluid_sample_sanitize_loop, fluid_sample_validate, new_fluid_sample, FluidFileCallbacks,
    FluidSfont, Sample, SfLoader,
};
use crate::synth::chan::FluidChannel;
use crate::synth::gen::*;
use crate::synth::modulator::*;
use crate::synth::synth::FluidSynthInner;
use crate::synth::voice::{
    fluid_voice_add_mod_local, fluid_voice_gen_incr, fluid_voice_gen_set,
    fluid_voice_optimize_sample,
};
use crate::utils::sys::{flog, fluid_log, FLUID_DBG, FLUID_ERR, FLUID_WARN};
use crate::FluidReal;

/* EMU8k/10k hardware applies this factor to initial attenuation generator values set at preset and
 * instrument level in a soundfont. We apply this factor when loading the generator values to stay
 * compatible as most existing soundfonts expect exactly this (strange, non-standard) behaviour. */
const EMU_ATTENUATION_FACTOR: f32 = 0.4f32;

/*-----------------------------------sfont.h----------------------------*/

#[allow(dead_code)]
pub(crate) const SF_SAMPMODES_LOOP: i32 = 1;
#[allow(dead_code)]
pub(crate) const SF_SAMPMODES_UNROLL: i32 = 2;

#[allow(dead_code)]
pub(crate) const SF_MIN_SAMPLERATE: i32 = 400;
#[allow(dead_code)]
pub(crate) const SF_MAX_SAMPLERATE: i32 = 50000;

#[allow(dead_code)]
pub(crate) const SF_MIN_SAMPLE_LENGTH: i32 = 32;

/// Translation of `fluid_zone_range_t`: defines the velocity and key range
/// for a zone.
#[derive(Debug, Default)]
pub(crate) struct FluidZoneRange {
    pub keylo: i32,
    pub keyhi: i32,
    pub vello: i32,
    pub velhi: i32,
    pub ignore: AtomicBool, /* set to TRUE for legato playing to ignore this range zone */
}

impl Clone for FluidZoneRange {
    fn clone(&self) -> Self {
        FluidZoneRange {
            keylo: self.keylo,
            keyhi: self.keyhi,
            vello: self.vello,
            velhi: self.velhi,
            ignore: AtomicBool::new(self.ignore.load(Ordering::Relaxed)),
        }
    }
}

/// Translation of `fluid_voice_zone_t`: stored on a preset zone to keep
/// track of the inst zones that could start a voice and their combined
/// preset zone/instrument zone ranges.
#[derive(Debug)]
pub(crate) struct FluidVoiceZone {
    pub inst_zone: Arc<FluidInstZone>,
    pub range: Arc<FluidZoneRange>,
}

/*
 * fluid_defsfont_t
 */
/// Translation of `fluid_defsfont_t`.
pub(crate) struct DefSfont {
    pub fcbs: FluidFileCallbacks, /* the file callbacks used to load this Soundfont */
    pub filename: String,         /* the filename of this soundfont */
    pub samplepos: u32,           /* the position in the file at which the sample data starts */
    pub samplesize: u32,          /* the size of the sample data in bytes */
    pub sampledata: Option<Arc<[i16]>>, /* the sample data, loaded in ram */

    pub sample24pos: u32, /* position within sffd of the sm24 chunk, set to zero if no 24 bit sample support */
    pub sample24size: u32, /* length within sffd of the sm24 chunk */
    pub sample24data: Option<Arc<[u8]>>, /* if not NULL, the least significant byte of the 24bit sample data, loaded in ram */

    pub sample: Vec<Arc<Sample>>,    /* the samples in this soundfont */
    pub preset: Vec<Arc<DefPreset>>, /* the presets of this soundfont */
    pub inst: Vec<Arc<FluidInst>>,   /* the instruments of this soundfont */
    pub mlock: bool,                 /* Should we try memlock (avoid swapping)? */
    #[allow(dead_code)]
    pub dynamic_samples: bool, /* Enables dynamic sample loading if set */

    #[allow(dead_code)]
    pub preset_iter_cur: usize, /* the current preset in the iteration */

    /* (the parent sfont's default_mod_list, which the loading fills) */
    pub default_mod_list: Option<Arc<Vec<FluidMod>>>,
}

impl std::fmt::Debug for DefSfont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefSfont")
            .field("filename", &self.filename)
            .finish_non_exhaustive()
    }
}

/*
 * fluid_preset_t
 */
/// Translation of `fluid_defpreset_t`.
#[derive(Debug, Default)]
pub(crate) struct DefPreset {
    pub name: String,                         /* the name of the preset */
    pub bank: u32,                            /* the bank number */
    pub num: u32,                             /* the preset number */
    pub global_zone: Option<FluidPresetZone>, /* the global zone of the preset */
    pub zone: Vec<FluidPresetZone>,           /* the chained list of preset zones */
}

/*
 * fluid_preset_zone
 */
/// Translation of `fluid_preset_zone_t`.
#[derive(Debug)]
pub(crate) struct FluidPresetZone {
    pub name: String,
    pub inst: Option<Arc<FluidInst>>,
    pub voice_zone: Vec<FluidVoiceZone>,
    pub range: FluidZoneRange,
    pub gen: [FluidGen; GEN_LAST],
    pub mod_: Vec<FluidMod>, /* List of modulators */
}

/*
 * fluid_inst_t
 */
/// Translation of `fluid_inst_t`.
#[derive(Debug, Default)]
pub(crate) struct FluidInst {
    pub name: String,
    pub source_idx: i32, /* Index of instrument in source Soundfont */
    pub global_zone: Option<Arc<FluidInstZone>>,
    pub zone: Vec<Arc<FluidInstZone>>,
}

/*
 * fluid_inst_zone_t
 */
/// Translation of `fluid_inst_zone_t`.
#[derive(Debug)]
pub(crate) struct FluidInstZone {
    pub name: String,
    pub sample: Option<Arc<Sample>>,
    pub range: FluidZoneRange,
    pub gen: [FluidGen; GEN_LAST],
    pub mod_: Vec<FluidMod>, /* List of modulators */
}

/***************************************************************
 *
 *                           SFONT LOADER
 */

/// Translation of `fluid_defsfloader_load()`.
pub(crate) fn fluid_defsfloader_load(loader: &SfLoader, filename: &str) -> Option<FluidSfont> {
    let mut defsfont = new_fluid_defsfont(&loader.settings);

    if fluid_defsfont_load(&mut defsfont, &loader.file_callbacks, filename) == FLUID_FAILED {
        return None;
    }

    Some(FluidSfont {
        default_mod_list: defsfont.default_mod_list.clone(),
        data: defsfont,
        id: 0,
        bankofs: 0,
    })
}

/***************************************************************
 *
 *                           SFONT
 */

/// Translation of `new_fluid_defsfont()`.
fn new_fluid_defsfont(settings: &crate::utils::settings::Settings) -> DefSfont {
    let mut mlock = 0;
    let mut dynamic_samples = 0;
    settings.getint("synth.lock-memory", &mut mlock);
    settings.getint("synth.dynamic-sample-loading", &mut dynamic_samples);

    DefSfont {
        fcbs: FluidFileCallbacks {
            fopen: Arc::new(|_| None),
        },
        filename: String::new(),
        samplepos: 0,
        samplesize: 0,
        sampledata: None,
        sample24pos: 0,
        sample24size: 0,
        sample24data: None,
        sample: Vec::new(),
        preset: Vec::new(),
        inst: Vec::new(),
        mlock: mlock != 0,
        dynamic_samples: dynamic_samples != 0,
        preset_iter_cur: 0,
        default_mod_list: None,
    }
}

impl Drop for DefSfont {
    /// The part of `delete_fluid_defsfont()` that is not the `Arc`s: the
    /// sample data goes back to the sample cache.
    fn drop(&mut self) {
        if let Some(sampledata) = self.sampledata.take() {
            fluid_samplecache_unload(&sampledata);
        }
    }
}

impl DefSfont {
    /// Translation of `fluid_defsfont_get_name()`.
    #[allow(dead_code)]
    pub(crate) fn get_name(&self) -> &str {
        &self.filename
    }

    /// Translation of `fluid_defsfont_get_preset()`.
    pub(crate) fn get_preset(&self, bank: i32, num: i32) -> Option<Arc<DefPreset>> {
        for preset in &self.preset {
            if (preset.get_banknum() == bank) && (preset.get_num() == num) {
                return Some(preset.clone());
            }
        }

        None
    }

    /// Translation of `fluid_defsfont_iteration_start()`.
    #[allow(dead_code)]
    pub(crate) fn iteration_start(&mut self) {
        self.preset_iter_cur = 0;
    }

    /// Translation of `fluid_defsfont_iteration_next()`.
    #[allow(dead_code)]
    pub(crate) fn iteration_next(&mut self) -> Option<Arc<DefPreset>> {
        let preset = self.preset.get(self.preset_iter_cur).cloned();

        if preset.is_some() {
            self.preset_iter_cur += 1;
        }

        preset
    }
}

/// Translation of `fluid_defsfont_load_all_sampledata()`: loads the
/// sample data for all samples from the Soundfont file. For SF2 files, it
/// loads the data in one large block. (SF3 files are not supported in this
/// build.)
/// Returns FLUID_OK on success, otherwise FLUID_FAILED
fn fluid_defsfont_load_all_sampledata(
    defsfont: &mut DefSfont,
    samples: &mut [Sample],
    sfdata: &mut SfData,
) -> i32 {
    let sf3_file = sfdata.version.major == 3;
    let sample_parsing_result = FLUID_OK;
    let mut invalid_loops_were_sanitized = false;

    /* For SF2 files, we load the sample data in one large block */
    if !sf3_file {
        let num_samples = (sfdata.samplesize / 2) as i32;

        let loaded = fluid_samplecache_load(sfdata, 0, (num_samples - 1) as u32, 0, defsfont.mlock);
        let read_samples = match &loaded {
            Some((data, _)) => data.len() as i32,
            None => -1,
        };

        if let Some((data, data24)) = loaded {
            defsfont.sampledata = Some(data);
            defsfont.sample24data = data24;
        }

        if read_samples != num_samples {
            flog!(
                FLUID_ERR,
                "Attempted to read {} words of sample data, but got {} instead",
                num_samples,
                read_samples
            );
            return FLUID_FAILED;
        }
    }

    for sample in samples.iter_mut() {
        // (SF3 files are rejected while parsing the INFO chunk, without
        // libsndfile)
        /* Data pointers of SF2 samples point to large sample data block loaded above */
        sample.data = defsfont
            .sampledata
            .clone()
            .unwrap_or_else(|| Arc::from(Vec::new()));
        sample.data24 = defsfont.sample24data.clone();
        let modified = fluid_sample_sanitize_loop(sample, defsfont.samplesize);
        if modified {
            invalid_loops_were_sanitized = true;
        }
        fluid_voice_optimize_sample(sample);
    }

    if invalid_loops_were_sanitized {
        fluid_log(
            FLUID_WARN,
            "Some invalid sample loops were sanitized! If you experience audible glitches, start fluidsynth in verbose mode for detailed information.",
        );
    }

    sample_parsing_result
}

/// Translation of `fluid_defsfont_load()`.
fn fluid_defsfont_load(defsfont: &mut DefSfont, fcbs: &FluidFileCallbacks, file: &str) -> i32 {
    defsfont.filename = file.to_string();

    defsfont.fcbs = fcbs.clone();

    /* The actual loading is done in the sfont and sffile files */
    let Some(mut sfdata) = fluid_sffile_open(file, &defsfont.fcbs) else {
        /* error message already printed */
        return FLUID_FAILED;
    };

    let result = fluid_defsfont_load_sfdata(defsfont, &mut sfdata);
    fluid_sffile_close(sfdata);
    result
}

/// The part of `fluid_defsfont_load()` between opening and closing the
/// file.
fn fluid_defsfont_load_sfdata(defsfont: &mut DefSfont, sfdata: &mut SfData) -> i32 {
    if fluid_sffile_parse_presets(sfdata) == FLUID_FAILED {
        fluid_log(FLUID_ERR, "Couldn't parse presets from soundfont file");
        return FLUID_FAILED;
    }

    if !sfdata.default_mod_list.is_empty() {
        /* Load the default modulators*/
        let mut list = Vec::new();
        let dmod_data: Vec<Option<SfMod>> =
            sfdata.default_mod_list.iter().map(|&m| Some(m)).collect();
        if fluid_mod_import_sfont(&mut list, &dmod_data) != FLUID_OK {
            fluid_log(FLUID_ERR, "Unable to load the default modulators");
            return FLUID_FAILED;
        }
        defsfont.default_mod_list = Some(Arc::new(list));
    }

    /* Keep track of the position and size of the sample data because
    it's loaded separately (and might be unoaded/reloaded in future) */
    defsfont.samplepos = sfdata.samplepos;
    defsfont.samplesize = sfdata.samplesize;
    defsfont.sample24pos = sfdata.sample24pos;
    defsfont.sample24size = sfdata.sample24size;

    /* Create all samples from sample headers */
    // (fluid_defsfont_add_sample() prepends to the list, which reverses
    // sfdata's reversed list back to file order)
    let mut samples: Vec<Sample> = Vec::new();
    let mut sample_of_sfsample: Vec<Option<usize>> = Vec::new();
    for sfsample in &sfdata.sample {
        let mut sample = new_fluid_sample();

        if fluid_sample_import_sfont(&mut sample, sfsample, defsfont) == FLUID_OK {
            samples.push(sample);
            sample_of_sfsample.push(Some(samples.len() - 1));
        } else {
            /* (delete_fluid_sample()) */
            sample_of_sfsample.push(None);
        }
    }

    /* Store reference to FluidSynth sample in SFSample for later IZone fixups */
    for (sfsample, s) in sfdata.sample.iter_mut().zip(&sample_of_sfsample) {
        sfsample.fluid_sample = *s;
    }

    /* If dynamic sample loading is disabled, load all samples in the Soundfont */
    // (and when it is enabled, see the top of this file)
    let mut samples_rev: Vec<Sample> = samples.into_iter().rev().collect();
    if fluid_defsfont_load_all_sampledata(defsfont, &mut samples_rev, sfdata) == FLUID_FAILED {
        fluid_log(FLUID_ERR, "Unable to load all sample data");
        return FLUID_FAILED;
    }
    let n = samples_rev.len();
    defsfont.sample = samples_rev.into_iter().map(Arc::new).collect();
    // (indices into the prepended list)
    for sfsample in sfdata.sample.iter_mut() {
        sfsample.fluid_sample = sfsample.fluid_sample.map(|i| n - 1 - i);
    }

    /* Load all the presets */
    let sfpresets = std::mem::take(&mut sfdata.preset);
    for sfpreset in &sfpresets {
        let mut defpreset = new_fluid_defpreset();

        if fluid_defpreset_import_sfont(&mut defpreset, sfpreset, defsfont, sfdata) != FLUID_OK {
            return FLUID_FAILED;
        }

        if fluid_defsfont_add_preset(defsfont, defpreset) == FLUID_FAILED {
            return FLUID_FAILED;
        }
    }
    sfdata.preset = sfpresets;

    FLUID_OK
}

/// Translation of `fluid_defsfont_add_preset()`: add a preset to the
/// SoundFont.
fn fluid_defsfont_add_preset(defsfont: &mut DefSfont, defpreset: DefPreset) -> i32 {
    defsfont.preset.push(Arc::new(defpreset));

    FLUID_OK
}

/***************************************************************
 *
 *                           PRESET
 */

/// Translation of `new_fluid_defpreset()`.
fn new_fluid_defpreset() -> DefPreset {
    DefPreset::default()
}

impl DefPreset {
    /// Translation of `fluid_defpreset_get_banknum()`.
    pub(crate) fn get_banknum(&self) -> i32 {
        self.bank as i32
    }

    /// Translation of `fluid_defpreset_get_num()`.
    pub(crate) fn get_num(&self) -> i32 {
        self.num as i32
    }

    /// Translation of `fluid_defpreset_get_name()`.
    #[allow(dead_code)]
    pub(crate) fn get_name(&self) -> &str {
        &self.name
    }
}

/// Translation of `fluid_defpreset_noteon_add_mod_to_voice()`: adds
/// global and local modulators list to the voice. This is done in 2
/// steps:
/// - Step 1: Local modulators replace identical global modulators.
/// - Step 2: global + local modulators are added to the voice using mode.
///
/// Instrument zone list (local/global) must be added using FLUID_VOICE_OVERWRITE.
/// Preset zone list (local/global) must be added using FLUID_VOICE_ADD.
///
/// @param voice voice instance.
/// @param global_mod global list of modulators.
/// @param local_mod local list of modulators.
/// @param mode Determines how to handle an existing identical modulator.
///   #FLUID_VOICE_ADD to add (offset) the modulator amounts,
///   #FLUID_VOICE_OVERWRITE to replace the modulator,
fn fluid_defpreset_noteon_add_mod_to_voice(
    voice: &mut crate::synth::voice::FluidVoice,
    global_mod: &[FluidMod],
    local_mod: &[FluidMod],
    mode: i32,
) {
    /* list for 'sorting' global/local modulators */
    let mut mod_list: [FluidMod; FLUID_NUM_MOD] = [FluidMod::default(); FLUID_NUM_MOD];
    let mut mod_list_count: usize;

    /* identity_limit_count is the modulator upper limit number to handle with
     * existing identical modulators.
     * When identity_limit_count is below the actual number of modulators, this
     * will restrict identity check to this upper limit,
     * This is useful when we know by advance that there is no duplicate with
     * modulators at index above this limit. This avoid wasting cpu cycles at
     * noteon.
     */
    let mut identity_limit_count: usize;

    /* Step 1: Local modulators replace identical global modulators. */

    /* local (instrument zone/preset zone), modulators: Put them all into a list. */
    mod_list_count = 0;

    for local in local_mod {
        /* As modulators number in local_mod list was limited to FLUID_NUM_MOD at
        soundfont loading time (fluid_limit_mod_list()), here we don't need
        to check if mod_list is full.
         */
        mod_list[mod_list_count] = *local;
        mod_list_count += 1;
    }

    /* global (instrument zone/preset zone), modulators.
     * Replace modulators with the same definition in the global list:
     * (Instrument zone: SF 2.01 page 69, 'bullet' 8)
     * (Preset zone:     SF 2.01 page 69, second-last bullet).
     *
     * mod_list contains local modulators. Now we know that there
     * is no global modulator identical to another global modulator (this has
     * been checked at soundfont loading time). So global modulators
     * are only checked against local modulators number.
     */

    /* Restrict identity check to the number of local modulators */
    identity_limit_count = mod_list_count;

    for global in global_mod {
        /* 'Identical' global modulators are ignored.
         *  SF2.01 section 9.5.1
         *  page 69, 'bullet' 3 defines 'identical'.  */

        let mut i = 0;
        while i < identity_limit_count {
            if fluid_mod_test_identity(global, &mod_list[i]) {
                break;
            }
            i += 1;
        }

        /* Finally add the new modulator to the list. */
        if i >= identity_limit_count {
            /* Although local_mod and global_mod lists was limited to
            FLUID_NUM_MOD at soundfont loading time, it is possible that
            local + global modulators exceeds FLUID_NUM_MOD.
            So, checks if mod_list_count reaches the limit.
            */
            if mod_list_count >= FLUID_NUM_MOD {
                /* mod_list is full, we silently forget this modulator and
                next global modulators. When mod_list will be added to the
                voice, a warning will be displayed if the voice list is full.
                (see fluid_voice_add_mod_local()).
                */
                break;
            }

            mod_list[mod_list_count] = *global;
            mod_list_count += 1;
        }
    }

    /* Step 2: global + local modulators are added to the voice using mode. */

    /*
     * mod_list contains local and global modulators, we know that:
     * - there is no global modulator identical to another global modulator,
     * - there is no local modulator identical to another local modulator,
     * So these local/global modulators are only checked against
     * actual number of voice modulators.
     */

    /* Restrict identity check to the actual number of voice modulators */
    /* Actual number of voice modulators : defaults + [instruments] */
    identity_limit_count = voice.mod_count as usize;

    for mod_ in &mod_list[..mod_list_count] {
        /* in mode FLUID_VOICE_OVERWRITE disabled instruments modulators CANNOT be skipped. */
        /* in mode FLUID_VOICE_ADD disabled preset modulators can be skipped. */

        if (mode == FLUID_VOICE_OVERWRITE) || (mod_.amount != 0.0) {
            /* Instrument modulators -supersede- existing (default) modulators.
            SF 2.01 page 69, 'bullet' 6 */

            /* Preset modulators -add- to existing instrument modulators.
            SF2.01 page 70 first bullet on page */
            fluid_voice_add_mod_local(voice, mod_, mode, identity_limit_count as i32);
        }
    }
}

impl FluidSynthInner {
    /// Translation of `fluid_defpreset_noteon()` (with
    /// `fluid_defpreset_preset_noteon()`).
    pub(crate) fn fluid_defpreset_noteon(
        &mut self,
        defpreset: &DefPreset,
        chan: i32,
        key: i32,
        vel: i32,
    ) -> i32 {
        /* For detuned channels it might be better to use another key for Soundfont sample selection
         * giving better approximations for the pitch than the original key.
         * Example: play key 60 on 6370 Hz => use tuned key 64 for sample selection
         *
         * This feature is only enabled for melodic channels.
         * For drum channels we always select Soundfont samples by key numbers.
         */
        let channel: &FluidChannel = &self.channel[chan as usize];
        let tuned_key = if channel.channel_type == CHANNEL_TYPE_MELODIC {
            ((channel.get_key_pitch(key) / 100.0f32 as FluidReal) + 0.5f32 as FluidReal) as i32
        } else {
            key
        };

        let global_preset_zone = defpreset.global_zone.as_ref();

        /* run thru all the zones of this preset */
        for preset_zone in &defpreset.zone {
            /* check if the note falls into the key and velocity range of this
            preset */
            if fluid_zone_inside_range(&preset_zone.range, tuned_key, vel) {
                let inst = preset_zone
                    .inst
                    .as_ref()
                    .expect("preset zone with instrument");
                let global_inst_zone = inst.global_zone.as_ref();

                /* run thru all the zones of this instrument that could start a voice */
                for voice_zone in &preset_zone.voice_zone {
                    /* check if the instrument zone is ignored and the note falls into
                    the key and velocity range of this  instrument zone.
                    An instrument zone must be ignored when its voice is already running
                    played by a legato passage (see fluid_synth_noteon_monopoly_legato()) */
                    if fluid_zone_inside_range(&voice_zone.range, tuned_key, vel) {
                        let inst_zone = &voice_zone.inst_zone;

                        /* this is a good zone. allocate a new synthesis process and initialize it */
                        let sample = inst_zone.sample.as_ref().expect("voice zone with sample");
                        let Some(v) = self.fluid_synth_alloc_voice_local(
                            sample,
                            chan,
                            key,
                            vel,
                            Some(voice_zone.range.clone()),
                        ) else {
                            return FLUID_FAILED;
                        };

                        let voice = &mut self.voice[v];

                        /* Instrument level, generators */

                        for i in 0..GEN_LAST {
                            /* SF 2.01 section 9.4 'bullet' 4:
                             *
                             * A generator in a local instrument zone supersedes a
                             * global instrument zone generator.  Both cases supersede
                             * the default generator -> voice_gen_set */

                            if inst_zone.gen[i].flags != 0 {
                                fluid_voice_gen_set(
                                    voice,
                                    &mut self.eventhandler,
                                    i,
                                    inst_zone.gen[i].val as f32,
                                );
                            } else if let Some(g) = global_inst_zone.filter(|g| g.gen[i].flags != 0)
                            {
                                fluid_voice_gen_set(
                                    voice,
                                    &mut self.eventhandler,
                                    i,
                                    g.gen[i].val as f32,
                                );
                            } else {
                                /* The generator has not been defined in this instrument.
                                 * Do nothing, leave it at the default.
                                 */
                            }
                        } /* for all generators */

                        /* Adds instrument zone modulators (global and local) to the voice.*/
                        fluid_defpreset_noteon_add_mod_to_voice(
                            voice,
                            /* global instrument modulators */
                            global_inst_zone.map(|z| &z.mod_[..]).unwrap_or(&[]),
                            &inst_zone.mod_,       /* local instrument modulators */
                            FLUID_VOICE_OVERWRITE, /* mode */
                        );

                        /* Preset level, generators */

                        for i in 0..GEN_LAST {
                            let mut awe_val: FluidReal = 0.0;
                            /* SF 2.01 section 8.5 page 58: If some generators are
                            encountered at preset level, they should be ignored.
                            However this check is not necessary when the soundfont
                            loader has ignored invalid preset generators.
                            Actually load_pgen()has ignored these invalid preset
                            generators:
                              GEN_STARTADDROFS,      GEN_ENDADDROFS,
                              GEN_STARTLOOPADDROFS,  GEN_ENDLOOPADDROFS,
                              GEN_STARTADDRCOARSEOFS,GEN_ENDADDRCOARSEOFS,
                              GEN_STARTLOOPADDRCOARSEOFS,
                              GEN_KEYNUM, GEN_VELOCITY,
                              GEN_ENDLOOPADDRCOARSEOFS,
                              GEN_SAMPLEMODE, GEN_EXCLUSIVECLASS,GEN_OVERRIDEROOTKEY
                            */

                            /* SF 2.01 section 9.4 'bullet' 9: A generator in a
                             * local preset zone supersedes a global preset zone
                             * generator.  The effect is -added- to the destination
                             * summing node -> voice_gen_incr */

                            if preset_zone.gen[i].flags != 0 {
                                fluid_voice_gen_incr(voice, i, preset_zone.gen[i].val as f32);
                            } else if let Some(g) =
                                global_preset_zone.filter(|g| g.gen[i].flags != 0)
                            {
                                fluid_voice_gen_incr(voice, i, g.gen[i].val as f32);
                            } else {
                                /* The generator has not been defined in this preset
                                 * Do nothing, leave it unchanged.
                                 */
                            }

                            /* ...unless the default value has been overridden by an AWE32 NRPN */
                            if self.channel[chan as usize].get_override_gen_default(i, &mut awe_val)
                            {
                                fluid_voice_gen_set(
                                    voice,
                                    &mut self.eventhandler,
                                    i,
                                    awe_val as f32,
                                );
                            }
                        } /* for all generators */

                        /* Adds preset zone modulators (global and local) to the voice.*/
                        fluid_defpreset_noteon_add_mod_to_voice(
                            voice,
                            /* global preset modulators */
                            global_preset_zone.map(|z| &z.mod_[..]).unwrap_or(&[]),
                            &preset_zone.mod_, /* local preset modulators */
                            FLUID_VOICE_ADD,   /* mode */
                        );

                        /* add the synthesis process to the synthesis loop. */
                        self.fluid_synth_start_voice(v);

                        /* Store the ID of the first voice that was created by this noteon event.
                         * Exclusive class may only terminate older voices.
                         * That avoids killing voices, which have just been created.
                         * (a noteon event can create several voice processes with the same exclusive
                         * class - for example when using stereo samples)
                         */
                    }
                }
            }
        }

        FLUID_OK
    }
}

/// Translation of `fluid_defpreset_import_sfont()`.
fn fluid_defpreset_import_sfont(
    defpreset: &mut DefPreset,
    sfpreset: &SfPreset,
    defsfont: &mut DefSfont,
    sfdata: &mut SfData,
) -> i32 {
    if !sfpreset.name.is_empty() {
        defpreset.name = sfpreset.name.clone();
    } else {
        defpreset.name = truncate_str(format!("Bank{},Pre{}", sfpreset.bank, sfpreset.prenum), 20);
        /* (sizeof(defpreset->name) - 1) */
    }

    defpreset.bank = sfpreset.bank as u32;
    defpreset.num = sfpreset.prenum as u32;

    for (count, sfzone) in sfpreset.zone.iter().enumerate() {
        let sfzone = sfzone.as_ref().expect("zone loaded");
        let zone_name = truncate_str(format!("pz:{}/{}", defpreset.name, count), 255);
        let mut zone = new_fluid_preset_zone(&zone_name);

        if fluid_preset_zone_import_sfont(
            &mut zone,
            defpreset.global_zone.as_ref(),
            sfzone,
            defsfont,
            sfdata,
        ) != FLUID_OK
        {
            return FLUID_FAILED;
        }

        if (count == 0) && zone.inst.is_none() {
            defpreset.global_zone = Some(zone);
        } else {
            fluid_defpreset_add_zone(defpreset, zone);
        }
    }

    FLUID_OK
}

/// Translation of `fluid_defpreset_add_zone()` (which prepends).
fn fluid_defpreset_add_zone(defpreset: &mut DefPreset, zone: FluidPresetZone) {
    defpreset.zone.insert(0, zone);
}

/***************************************************************
 *
 *                           PRESET_ZONE
 */

/// The initial range of a zone (`new_fluid_preset_zone()` and
/// `new_fluid_inst_zone()`).
fn new_zone_range() -> FluidZoneRange {
    FluidZoneRange {
        keylo: 0,
        keyhi: 128,
        vello: 0,
        velhi: 128,
        ignore: AtomicBool::new(false),
    }
}

/// A generator array as `fluid_gen_init(&zone->gen[0], NULL)` leaves it.
fn new_zone_gens() -> [FluidGen; GEN_LAST] {
    let mut gen = [FluidGen::default(); GEN_LAST];
    /* Flag all generators as unused (default, they will be set when they are found
     * in the sound font).
     * This also sets the generator values to default, but that is of no concern here.*/
    fluid_gen_init(&mut gen, None);
    gen
}

/// Translation of `new_fluid_preset_zone()`.
fn new_fluid_preset_zone(name: &str) -> FluidPresetZone {
    FluidPresetZone {
        name: name.to_string(),
        inst: None,
        voice_zone: Vec::new(),
        range: new_zone_range(),
        gen: new_zone_gens(),
        mod_: Vec::new(), /* list of modulators */
    }
}

/// Translation of `fluid_preset_zone_create_voice_zones()`.
fn fluid_preset_zone_create_voice_zones(preset_zone: &mut FluidPresetZone) -> i32 {
    let Some(inst) = preset_zone.inst.clone() else {
        return FLUID_FAILED;
    };
    let prange = &preset_zone.range;

    for inst_zone in &inst.zone {
        /* We only create voice ranges for zones that could actually start a voice,
         * i.e. that have a sample and don't point to ROM */
        let Some(sample) = &inst_zone.sample else {
            continue;
        };
        if fluid_sample_in_rom(sample) {
            continue;
        }

        let irange = &inst_zone.range;

        let range = FluidZoneRange {
            keylo: if prange.keylo > irange.keylo {
                prange.keylo
            } else {
                irange.keylo
            },
            keyhi: if prange.keyhi < irange.keyhi {
                prange.keyhi
            } else {
                irange.keyhi
            },
            vello: if prange.vello > irange.vello {
                prange.vello
            } else {
                irange.vello
            },
            velhi: if prange.velhi < irange.velhi {
                prange.velhi
            } else {
                irange.velhi
            },
            ignore: AtomicBool::new(false),
        };

        preset_zone.voice_zone.push(FluidVoiceZone {
            inst_zone: inst_zone.clone(),
            range: Arc::new(range),
        });
    }

    FLUID_OK
}

/// Translation of `fluid_zone_is_mod_identical()`: checks if modulator
/// mod is identical to another modulator in the list (specs SF 2.0X 7.4,
/// 7.8).
/// @param mod, modulator list (the modulator and the ones following it).
/// @param name, if not NULL, pointer on a string displayed as warning.
/// @return TRUE if mod is identical to another modulator, FALSE otherwise.
fn fluid_zone_is_mod_identical(list: &[FluidMod], name: Option<&str>) -> bool {
    let mod_ = &list[0];

    for next in &list[1..] {
        /* is mod identical to next ? */
        if fluid_mod_test_identity(mod_, next) {
            if let Some(name) = name {
                flog!(FLUID_WARN, "Ignoring identical modulator {}", name);
            }

            return true;
        }
    }

    false
}

/// Translation of `fluid_limit_mod_list()`: limits the number of
/// modulators in a modulator list. This is appropriate to internal
/// synthesizer modulators tables which have a fixed size (FLUID_NUM_MOD).
fn fluid_limit_mod_list(zone_name: &str, list_mod: &mut Vec<FluidMod>) {
    if list_mod.len() > FLUID_NUM_MOD {
        /* truncation of list_mod */
        list_mod.truncate(FLUID_NUM_MOD);
        flog!(
            FLUID_WARN,
            "{}, modulators count limited to {}",
            zone_name,
            FLUID_NUM_MOD
        );
    }
}

/// Translation of `fluid_zone_check_mod()`: checks and remove invalid
/// modulators from a zone modulators list.
/// - checks valid modulator sources (specs SF 2.01  7.4, 7.8, 8.2.1).
/// - checks identical modulators in the list (specs SF 2.01  7.4, 7.8).
fn fluid_zone_check_mod(zone_name: &str, list_mod: &mut Vec<FluidMod>) {
    let mut i = 0; /* mod in list_mod */
    let mut mod_idx = 0; /* modulator index */

    while i < list_mod.len() {
        /* prepare modulator name: zonename/#modulator */
        let zone_mod_name = truncate_str(format!("{}/mod{}", zone_name, mod_idx), 255);

        /* has mod invalid sources ? */
        if !fluid_mod_check_sources(&list_mod[i], Some(&zone_mod_name))
            /* or is mod identical to any following modulator ? */
            || fluid_zone_is_mod_identical(&list_mod[i..], Some(&zone_mod_name))
        {
            /* the modulator is useless so we remove it */
            list_mod.remove(i);
        } else {
            i += 1;
        }

        mod_idx += 1;
    }

    /* limits the size of modulators list */
    fluid_limit_mod_list(zone_name, list_mod);
}

/// Translation of `fluid_zone_gen_import_sfont()`: imports generators
/// from sfzone to gen and range.
/// @param gen, pointer on destination generators table.
/// @param range, pointer on destination range generators.
/// @param sfzone, pointer on soundfont zone generators.
fn fluid_zone_gen_import_sfont(
    gen: &mut [FluidGen; GEN_LAST],
    range: &mut FluidZoneRange,
    global_range: Option<&FluidZoneRange>,
    sfzone: &SfZone,
) {
    if let Some(global_range) = global_range {
        // All zones are initialized with the default range of 0-127. However, local zones should be superseded by
        // the range of their global zone in case that local zone lacks a GEN_KEYRANGE or GEN_VELRANGE
        // (see issue #1250).
        range.keylo = global_range.keylo;
        range.keyhi = global_range.keyhi;
        range.vello = global_range.vello;
        range.velhi = global_range.velhi;
    }

    for sfgen in sfzone.gen.iter().flatten() {
        let id = sfgen.id as usize;
        match id {
            GEN_KEYRANGE => {
                range.keylo = sfgen.amount.lo() as i32;
                range.keyhi = sfgen.amount.hi() as i32;
            }

            GEN_VELRANGE => {
                range.vello = sfgen.amount.lo() as i32;
                range.velhi = sfgen.amount.hi() as i32;
            }

            GEN_ATTENUATION => {
                /* EMU8k/10k hardware applies a scale factor to initial attenuation generator values set at
                 * preset and instrument level */
                gen[id].val =
                    sfgen.amount.sword() as FluidReal * EMU_ATTENUATION_FACTOR as FluidReal;
                gen[id].flags = GEN_SET;
            }

            GEN_INSTRUMENT | GEN_SAMPLEID => {
                gen[id].val = sfgen.amount.uword() as FluidReal;
                gen[id].flags = GEN_SET;
            }

            _ => {
                gen[id].val = sfgen.amount.sword() as FluidReal;
                gen[id].flags = GEN_SET;
            }
        }
    }
}

/// Translation of `fluid_zone_mod_source_import_sfont()`: imports source
/// information from sf_source to src and flags.
/// @param src, pointer on destination modulator source.
/// @param flags, pointer on destination modulator flags.
/// @param sf_source, soundfont modulator source.
/// @return return TRUE if success, FALSE if source type is unknown.
fn fluid_zone_mod_source_import_sfont(src: &mut u8, flags: &mut u8, sf_source: u16) -> bool {
    /* sources */
    *src = (sf_source & 127) as u8; /* index of source, seven-bit value, SF2.01 section 8.2, page 50 */

    /* Bit 7: CC flag SF 2.01 section 8.2.1 page 50*/
    let mut flags_dest: u8 = 0; /* destination flags */

    if sf_source & (1 << 7) != 0 {
        flags_dest |= FLUID_MOD_CC;
    } else {
        flags_dest |= FLUID_MOD_GC;
    }

    /* Bit 8: D flag SF 2.01 section 8.2.2 page 51*/
    if sf_source & (1 << 8) != 0 {
        flags_dest |= FLUID_MOD_NEGATIVE;
    } else {
        flags_dest |= FLUID_MOD_POSITIVE;
    }

    /* Bit 9: P flag SF 2.01 section 8.2.3 page 51*/
    if sf_source & (1 << 9) != 0 {
        flags_dest |= FLUID_MOD_BIPOLAR;
    } else {
        flags_dest |= FLUID_MOD_UNIPOLAR;
    }

    /* modulator source types: SF2.01 section 8.2.1 page 52 */
    let mut type_ = (sf_source >> 10) as i32;
    type_ &= 63; /* type is a 6-bit value */

    if type_ == 0 {
        flags_dest |= FLUID_MOD_LINEAR;
    } else if type_ == 1 {
        flags_dest |= FLUID_MOD_CONCAVE;
    } else if type_ == 2 {
        flags_dest |= FLUID_MOD_CONVEX;
    } else if type_ == 3 {
        flags_dest |= FLUID_MOD_SWITCH;
    } else {
        *flags = flags_dest;
        /* This shouldn't happen - unknown type! */
        return false;
    }

    *flags = flags_dest;
    true
}

/// Translation of `fluid_mod_import_sfont()`: imports modulators from
/// sfzone to modulators list mod.
/// @param mod -  address of pointer on modulators list to return.
/// @param sfmod - pointer on the modulator list.
/// @return FLUID_OK if success, FLUID_FAILED otherwise.
fn fluid_mod_import_sfont(mod_: &mut Vec<FluidMod>, sfmod: &[Option<SfMod>]) -> i32 {
    /* Import the modulators (only SF2.1 and higher) */
    for mod_src in sfmod {
        let mut mod_dest = new_fluid_mod();

        let Some(mod_src) = mod_src else {
            // empty list (DMOD case) - nothing to do!
            return FLUID_OK;
        };

        /* *** Amount *** */
        mod_dest.amount = mod_src.amount as f64;

        /* *** Source *** */
        if !fluid_zone_mod_source_import_sfont(
            &mut mod_dest.src1,
            &mut mod_dest.flags1,
            mod_src.src,
        ) {
            /* This shouldn't happen - unknown type!
             * Deactivate the modulator by setting the amount to 0. */
            mod_dest.amount = 0.0;
        }

        /* *** Dest *** */
        mod_dest.dest = mod_src.dest as u8; /* index of controlled generator */

        /* *** Amount source *** */
        if !fluid_zone_mod_source_import_sfont(
            &mut mod_dest.src2,
            &mut mod_dest.flags2,
            mod_src.amtsrc,
        ) {
            /* This shouldn't happen - unknown type!
             * Deactivate the modulator by setting the amount to 0. */
            mod_dest.amount = 0.0;
        }

        /*
         * *** Transform Type ***
         * Only 2 types of transform are defined in the sf2 specification.
         */
        if mod_src.trans != FLUID_MOD_TRANSFORM_LINEAR as u16
            && mod_src.trans != FLUID_MOD_TRANSFORM_ABS as u16
        {
            /* disable the modulator as the transform is invalid */
            mod_dest.amount = 0.0;
            mod_dest.trans = FLUID_MOD_TRANSFORM_LINEAR;
        } else {
            mod_dest.trans = mod_src.trans as u8;
        }

        /* Store the new modulator in the zone The order of modulators
         * will make a difference, at least in an instrument context: The
         * second modulator overwrites the first one, if they only differ
         * in amount. */
        mod_.push(mod_dest);
    } /* foreach modulator */
    FLUID_OK
}

/// Translation of `fluid_zone_mod_import_sfont()`: imports modulators from
/// sfzone to modulators list mod.
/// @param zone_name, zone name.
/// @param mod, address of pointer on modulators list to return.
/// @param sfzone, pointer on soundfont zone.
/// @return FLUID_OK if success, FLUID_FAILED otherwise.
fn fluid_zone_mod_import_sfont(zone_name: &str, mod_: &mut Vec<FluidMod>, sfzone: &SfZone) -> i32 {
    if fluid_mod_import_sfont(mod_, &sfzone.mod_) != FLUID_OK {
        return FLUID_FAILED;
    }

    /* checks and removes invalid modulators in modulators list*/
    fluid_zone_check_mod(zone_name, mod_);
    FLUID_OK
}

/// Translation of `fluid_preset_zone_import_sfont()`.
fn fluid_preset_zone_import_sfont(
    zone: &mut FluidPresetZone,
    global_zone: Option<&FluidPresetZone>,
    sfzone: &SfZone,
    defsfont: &mut DefSfont,
    sfdata: &mut SfData,
) -> i32 {
    /* import the generators */
    fluid_zone_gen_import_sfont(
        &mut zone.gen,
        &mut zone.range,
        global_zone.map(|z| &z.range),
        sfzone,
    );

    if zone.gen[GEN_INSTRUMENT].flags == GEN_SET {
        let inst_idx = zone.gen[GEN_INSTRUMENT].val as i32;

        zone.inst = find_inst_by_idx(defsfont, inst_idx);

        if zone.inst.is_none() {
            zone.inst = fluid_inst_import_sfont(inst_idx, defsfont, sfdata);
        }

        if zone.inst.is_none() {
            flog!(
                FLUID_ERR,
                "Preset zone {}: Invalid instrument reference",
                zone.name
            );
            return FLUID_FAILED;
        }

        if fluid_preset_zone_create_voice_zones(zone) == FLUID_FAILED {
            return FLUID_FAILED;
        }

        /* We don't need this generator anymore */
        zone.gen[GEN_INSTRUMENT].flags = GEN_UNUSED;
    }

    /* Import the modulators (only SF2.1 and higher) */
    let name = zone.name.clone();
    fluid_zone_mod_import_sfont(&name, &mut zone.mod_, sfzone)
}

/***************************************************************
 *
 *                           INST
 */

/// Translation of `fluid_inst_import_sfont()`.
fn fluid_inst_import_sfont(
    inst_idx: i32,
    defsfont: &mut DefSfont,
    sfdata: &mut SfData,
) -> Option<Arc<FluidInst>> {
    let sfinst = sfdata.inst.iter().find(|sfinst| sfinst.idx == inst_idx)?;

    let mut inst = FluidInst::default();

    inst.source_idx = sfinst.idx;

    if !sfinst.name.is_empty() {
        inst.name = sfinst.name.clone();
    } else {
        inst.name = "<untitled>".to_string();
    }

    for (count, sfzone) in sfinst.zone.iter().enumerate() {
        let sfzone = sfzone.as_ref().expect("zone loaded");
        /* instrument zone name */
        let zone_name = truncate_str(format!("iz:{}/{}", inst.name, count), 255);

        let mut inst_zone = new_fluid_inst_zone(&zone_name);

        if fluid_inst_zone_import_sfont(
            &mut inst_zone,
            inst.global_zone.as_deref(),
            sfzone,
            defsfont,
            sfdata,
        ) != FLUID_OK
        {
            flog!(
                FLUID_ERR,
                "fluid_inst_zone_import_sfont() failed for instrument {}",
                inst.name
            );
            return None;
        }

        if (count == 0) && inst_zone.sample.is_none() {
            inst.global_zone = Some(Arc::new(inst_zone));
        } else {
            fluid_inst_add_zone(&mut inst, inst_zone);
        }
    }

    let inst = Arc::new(inst);
    defsfont.inst.push(inst.clone());
    Some(inst)
}

/// Translation of `fluid_inst_add_zone()` (which prepends).
fn fluid_inst_add_zone(inst: &mut FluidInst, zone: FluidInstZone) {
    inst.zone.insert(0, Arc::new(zone));
}

/***************************************************************
 *
 *                           INST_ZONE
 */

/// Translation of `new_fluid_inst_zone()`.
fn new_fluid_inst_zone(name: &str) -> FluidInstZone {
    FluidInstZone {
        name: name.to_string(),
        sample: None,
        range: new_zone_range(),
        /* Flag the generators as unused.
         * This also sets the generator values to default, but they will be overwritten anyway, if used.*/
        gen: new_zone_gens(),
        mod_: Vec::new(), /* list of modulators */
    }
}

/// Translation of `fluid_inst_zone_import_sfont()`.
fn fluid_inst_zone_import_sfont(
    inst_zone: &mut FluidInstZone,
    global_inst_zone: Option<&FluidInstZone>,
    sfzone: &SfZone,
    defsfont: &DefSfont,
    sfdata: &SfData,
) -> i32 {
    /* import the generators */
    fluid_zone_gen_import_sfont(
        &mut inst_zone.gen,
        &mut inst_zone.range,
        global_inst_zone.map(|z| &z.range),
        sfzone,
    );

    /* FIXME */
    /*    if (zone->gen[GEN_EXCLUSIVECLASS].flags == GEN_SET) { */
    /*      FLUID_LOG(FLUID_DBG, "ExclusiveClass=%d\n", (int) zone->gen[GEN_EXCLUSIVECLASS].val); */
    /*    } */

    if inst_zone.gen[GEN_SAMPLEID].flags == GEN_SET {
        let sample_idx = inst_zone.gen[GEN_SAMPLEID].val as i32;

        /* find the SFSample by index */
        let Some(sfsample) = sfdata.sample.iter().find(|s| s.idx == sample_idx) else {
            flog!(
                FLUID_ERR,
                "Instrument zone '{}': Invalid sample reference",
                inst_zone.name
            );
            return FLUID_FAILED;
        };

        inst_zone.sample = sfsample.fluid_sample.map(|i| defsfont.sample[i].clone());

        /* we don't need this generator anymore, mark it as unused */
        inst_zone.gen[GEN_SAMPLEID].flags = GEN_UNUSED;
    }

    /* Import the modulators (only SF2.1 and higher) */
    let name = inst_zone.name.clone();
    fluid_zone_mod_import_sfont(&name, &mut inst_zone.mod_, sfzone)
}

/// Translation of `fluid_zone_inside_range()`.
pub(crate) fn fluid_zone_inside_range(range: &FluidZoneRange, key: i32, vel: i32) -> bool {
    /* ignoreInstrumentZone is set in mono legato playing */
    /* Reset the 'ignore' request */
    let ignore_zone = range.ignore.swap(false, Ordering::Relaxed);

    !ignore_zone
        && ((range.keylo <= key)
            && (range.keyhi >= key)
            && (range.vello <= vel)
            && (range.velhi >= vel))
}

/***************************************************************
 *
 *                           SAMPLE
 */

/// Translation of `fluid_sample_in_rom()`.
pub(crate) fn fluid_sample_in_rom(sample: &Sample) -> bool {
    sample.sampletype & FLUID_SAMPLETYPE_ROM != 0
}

/// Translation of `fluid_sample_import_sfont()`.
fn fluid_sample_import_sfont(sample: &mut Sample, sfsample: &SfSample, defsfont: &DefSfont) -> i32 {
    sample.name = sfsample.name.clone();

    sample.source_start = sfsample.start;
    sample.source_end = if sfsample.end > 0 {
        sfsample.end - 1
    } else {
        0
    }; /* marks last sample, contrary to SF spec. */
    sample.source_loopstart = sfsample.loopstart;
    sample.source_loopend = sfsample.loopend;

    sample.start = sample.source_start;
    sample.end = sample.source_end;
    sample.loopstart = sample.source_loopstart;
    sample.loopend = sample.source_loopend;
    sample.samplerate = sfsample.samplerate;
    sample.origpitch = sfsample.origpitch as i32;
    sample.pitchadj = sfsample.pitchadj as i32;
    sample.sampletype = sfsample.sampletype as i32;
    sample.default_modulators = defsfont.default_mod_list.clone();

    flog!(
        FLUID_DBG,
        "Discovering sample '{}', src_start {}, loop_start {}, loop_end {}, src_end {}",
        sample.name,
        sample.source_start as i32,
        sample.loopstart as i32,
        sample.loopend as i32,
        sample.source_end as i32
    );

    if fluid_sample_validate(sample, defsfont.samplesize) == FLUID_FAILED {
        return FLUID_FAILED;
    }

    FLUID_OK
}

/// A string cut to at most `max` bytes, as snprintf() cuts it (here at a
/// character boundary).
fn truncate_str(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// Translation of `find_inst_by_idx()`.
fn find_inst_by_idx(defsfont: &DefSfont, idx: i32) -> Option<Arc<FluidInst>> {
    defsfont
        .inst
        .iter()
        .find(|inst| inst.source_idx == idx)
        .cloned()
}
