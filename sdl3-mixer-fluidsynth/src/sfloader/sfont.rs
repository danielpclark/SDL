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
// Modified 2026-10-17: translated into Rust from fluid_sfont.c,
// fluid_sfont.h and include/fluidsynth/sfont.h in SDL_mixer 3.3.0's
// FluidSynth 2.5.7 (see NOTICE). SDL_mixer only uses the default
// SoundFont 2 loader (fluid_defsfont.c), so the "virtual" SoundFont,
// preset and loader method tables are that loader's functions here; the
// reference counts and notifications that only serve dynamic sample
// loading and SoundFont unloading are the `Arc`s.

use std::fs::File;
use std::sync::Arc;

use crate::fluidsynth_h::*;
use crate::sfloader::defsfont::{DefPreset, DefSfont};
use crate::synth::modulator::FluidMod;
use crate::utils::settings::Settings;
use crate::utils::sys::{
    flog, fluid_file_open, fluid_file_read, fluid_file_seek, fluid_file_tell, FLUID_DBG, FLUID_ERR,
    FLUID_INFO, FLUID_WARN,
};

/// A file a SoundFont loader reads: what the open callback of
/// `fluid_sfloader_set_callbacks()` returns, with the read, seek, tell and
/// close callbacks for it (`fluid_sfloader_callback_read_t`,
/// `fluid_sfloader_callback_seek_t`, `fluid_sfloader_callback_tell_t` and
/// `fluid_sfloader_callback_close_t`).
pub trait SfFile: Send {
    /// Reads exactly `buf.len()` bytes, returning #FLUID_OK, or
    /// #FLUID_FAILED if that many can't be read.
    fn read(&mut self, buf: &mut [u8]) -> i32;
    /// Seeks to `offset` from `origin` (`SEEK_SET`, `SEEK_CUR` or
    /// `SEEK_END`), returning #FLUID_OK or #FLUID_FAILED.
    fn seek(&mut self, offset: i64, origin: i32) -> i32;
    /// The current position, or #FLUID_FAILED (-1).
    fn tell(&mut self) -> i64;
    /// Closes the file, returning #FLUID_OK or #FLUID_FAILED.
    fn close(self: Box<Self>) -> i32;
}

/// Translation of `fluid_sfloader_callback_open_t`: opens a file by name
/// (or whatever the name stands for), or returns `None`.
pub type SfOpenCallback = Arc<dyn Fn(&str) -> Option<Box<dyn SfFile>> + Send + Sync>;

/// Translation of `fluid_file_callbacks_t`: file callback structure to
/// enable custom soundfont loading (e.g. from memory).
#[derive(Clone)]
pub(crate) struct FluidFileCallbacks {
    pub fopen: SfOpenCallback,
}

impl std::fmt::Debug for FluidFileCallbacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FluidFileCallbacks").finish_non_exhaustive()
    }
}

/// The default file callbacks' file (`FILE *`).
struct DefaultFile(File);

impl SfFile for DefaultFile {
    fn read(&mut self, buf: &mut [u8]) -> i32 {
        /* safe_fread() */
        if fluid_file_read(buf, &mut self.0) {
            FLUID_OK
        } else {
            FLUID_FAILED
        }
    }

    fn seek(&mut self, ofs: i64, whence: i32) -> i32 {
        /* safe_fseek() */
        if fluid_file_seek(&mut self.0, ofs, whence) {
            FLUID_OK
        } else {
            FLUID_FAILED
        }
    }

    fn tell(&mut self) -> i64 {
        /* default_ftell() */
        fluid_file_tell(&mut self.0)
    }

    fn close(self: Box<Self>) -> i32 {
        /* default_fclose() */
        drop(self.0);
        FLUID_OK
    }
}

/// Translation of `default_fopen()`.
fn default_fopen(path: &str) -> Option<Box<dyn SfFile>> {
    match fluid_file_open(path) {
        Ok(handle) => Some(Box::new(DefaultFile(handle))),
        Err(msg) => {
            flog!(
                FLUID_ERR,
                "fluid_sfloader_load(): Failed to open '{}': {}",
                path,
                msg
            );
            None
        }
    }
}

/// Translation of `fluid_sfloader_t`: a SoundFont loader.
///
/// This is the default SoundFont 2 loader (`new_fluid_defsfloader()`),
/// the only one SDL_mixer uses.
#[derive(Clone, Debug)]
pub struct SfLoader {
    pub(crate) settings: Settings, /* User defined data pointer used by _fluid_sfloader_t::load() */

    /* Callback structure specifying file operations used during soundfont loading to allow custom loading, such as from memory */
    pub(crate) file_callbacks: FluidFileCallbacks,

    /// The synth's DLS loader (`new_fluid_dls_loader()`) rather than a
    /// SoundFont 2 loader.
    pub(crate) dls: bool,
}

impl SfLoader {
    /// Translation of `new_fluid_defsfloader()` (with
    /// `new_fluid_sfloader()`): creates a default soundfont2 loader that
    /// can be used with `Synth::add_sfloader()`.
    ///
    /// By default every synth instance has an initial default soundfont
    /// loader instance. Calling this function is usually only necessary to
    /// load a soundfont from memory, by providing a custom open callback
    /// via `SfLoader::set_callbacks()`.
    pub fn new_defsfloader(settings: &Settings) -> SfLoader {
        SfLoader {
            settings: settings.clone(),
            file_callbacks: FluidFileCallbacks {
                fopen: Arc::new(default_fopen),
            },
            dls: false,
        }
    }

    /// Translation of `new_fluid_dls_loader()`: the DLS loader every synth
    /// has (see `crate::sfloader::dls`).
    pub(crate) fn new_fluid_dls_loader(settings: &Settings) -> SfLoader {
        SfLoader {
            dls: true,
            ..SfLoader::new_defsfloader(settings)
        }
    }

    /// Translation of `fluid_sfloader_set_callbacks()`: set a custom
    /// callback to be used upon soundfont loading; the files it opens read,
    /// seek, tell and close themselves (see [`SfFile`]).
    ///
    /// Useful for loading a soundfont from memory.
    pub fn set_callbacks(&mut self, open: SfOpenCallback) -> i32 {
        self.file_callbacks = FluidFileCallbacks { fopen: open };

        // NOTE: if we ever make the instpatch loader public, this may return FLUID_FAILED
        FLUID_OK
    }

    /// Translation of `fluid_sfloader_load()` (the default loader's
    /// `fluid_defsfloader_load()`).
    pub(crate) fn load(&self, filename: &str) -> Option<FluidSfont> {
        if self.dls {
            crate::sfloader::dls::fluid_dls_loader_load(self, filename)
        } else {
            crate::sfloader::defsfont::fluid_defsfloader_load(self, filename)
        }
    }
}

/// Translation of `fluid_sfont_t`: virtual SoundFont instance structure.
#[derive(Debug)]
pub(crate) struct FluidSfont {
    pub data: DefSfont, /* User defined data */
    pub id: i32,        /* SoundFont ID */
    pub bankofs: i32,   /* Bank offset */

    #[allow(dead_code)]
    pub default_mod_list: Option<Arc<Vec<FluidMod>>>, /* If not NULL, a list of default modulators for that soundfont (e.g. read from DMOD, or DLS compatibility default mods) */
}

impl FluidSfont {
    /// Translation of `fluid_sfont_get_id()`: retrieve the unique ID of a
    /// SoundFont instance.
    #[allow(dead_code)]
    pub(crate) fn get_id(&self) -> i32 {
        self.id
    }

    /// Translation of `fluid_sfont_get_name()`: retrieve the name of a
    /// SoundFont instance.
    #[allow(dead_code)]
    pub(crate) fn get_name(&self) -> &str {
        self.data.get_name()
    }

    /// Translation of `fluid_sfont_get_preset()`: retrieve the preset
    /// assigned the a SoundFont instance for the given bank and preset
    /// number.
    pub(crate) fn get_preset(&self, bank: i32, prenum: i32) -> Option<FluidPreset> {
        self.data.get_preset(bank, prenum).map(|data| FluidPreset {
            sfont_id: self.id,
            data,
        })
    }

    /// Translation of `fluid_sfont_iteration_start()`: starts / re-starts
    /// virtual preset iteration in a SoundFont.
    #[allow(dead_code)]
    pub(crate) fn iteration_start(&mut self) {
        self.data.iteration_start();
    }

    /// Translation of `fluid_sfont_iteration_next()`: virtual SoundFont
    /// preset iteration function.
    #[allow(dead_code)]
    pub(crate) fn iteration_next(&mut self) -> Option<FluidPreset> {
        let id = self.id;
        self.data
            .iteration_next()
            .map(|data| FluidPreset { sfont_id: id, data })
    }
}

/// Translation of `fluid_preset_t`: virtual SoundFont preset (here a
/// preset of the default SoundFont loader, with the ID of its SoundFont).
#[derive(Clone, Debug)]
pub(crate) struct FluidPreset {
    pub sfont_id: i32,        /* Parent virtual SoundFont */
    pub data: Arc<DefPreset>, /* User supplied data */
}

impl FluidPreset {
    /// The C pointer comparison of two presets.
    pub(crate) fn ptr_eq(a: &FluidPreset, b: &FluidPreset) -> bool {
        Arc::ptr_eq(&a.data, &b.data)
    }

    /// The C pointer comparison of two presets that may be NULL.
    pub(crate) fn opt_ptr_eq(a: Option<&FluidPreset>, b: Option<&FluidPreset>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => FluidPreset::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Translation of `fluid_preset_get_name()`.
    #[allow(dead_code)]
    pub(crate) fn get_name(&self) -> &str {
        self.data.get_name()
    }

    /// Translation of `fluid_preset_get_banknum()`.
    #[allow(dead_code)]
    pub(crate) fn get_banknum(&self) -> i32 {
        self.data.get_banknum()
    }

    /// Translation of `fluid_preset_get_num()`.
    #[allow(dead_code)]
    pub(crate) fn get_num(&self) -> i32 {
        self.data.get_num()
    }

    /// Translation of `fluid_preset_get_sfont()` (its ID).
    pub(crate) fn get_sfont_id(&self) -> i32 {
        self.sfont_id
    }
}

/// Translation of `fluid_sample_t`: virtual SoundFont sample.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sample {
    pub name: String, /* Sample name */

    /* The following four sample pointers store the original pointers from the Soundfont
     * file. They are never changed after loading and are used to re-create the
     * actual sample pointers after a sample has been unloaded and loaded again. The
     * actual sample pointers get modified during loading for SF3 (compressed) samples
     * and individually loaded SF2 samples. */
    pub source_start: u32,
    pub source_end: u32,
    pub source_loopstart: u32,
    pub source_loopend: u32,

    pub start: u32,     /* Start index */
    pub end: u32,       /* End index, index of last valid sample point (contrary to SF spec) */
    pub loopstart: u32, /* Loop start index */
    pub loopend: u32, /* Loop end index, first point following the loop (superimposed on loopstart) */

    pub data: Arc<[i16]>,          /* Pointer to the sample's 16 bit PCM data */
    pub data24: Option<Arc<[u8]>>, /* If not NULL, pointer to the least significant byte counterparts of each sample data point in order to create 24 bit audio samples */
    pub samplerate: u32,           /* Sample rate */
    pub origpitch: i32,            /* Original pitch (MIDI note number, 0-127) */
    pub pitchadj: i32,             /* Fine pitch adjustment (+/- 99 cents) */
    pub sampletype: i32, /* Specifies the type of this sample as indicated by the #fluid_sample_type enum */

    pub amplitude_that_reaches_noise_floor_is_valid: bool, /* Indicates if \a amplitude_that_reaches_noise_floor is valid (TRUE), set to FALSE initially to calculate. */
    pub amplitude_that_reaches_noise_floor: f64, /* The amplitude at which the sample's loop will be below the noise floor.  For voice off optimization, calculated automatically. */

    pub default_modulators: Option<Arc<Vec<FluidMod>>>, /* Default soundfont modulators for this sample to allocate the voice for it. NULL will use the synth's defaults. */
}

/// Translation of `new_fluid_sample()`: create a new sample instance.
pub(crate) fn new_fluid_sample() -> Sample {
    Sample {
        data: Arc::from(Vec::new()),
        ..Default::default()
    }
}

const EXCLUSIVE_FLAGS: i32 = FLUID_SAMPLETYPE_MONO | FLUID_SAMPLETYPE_RIGHT | FLUID_SAMPLETYPE_LEFT;

/// Translation of `fluid_sample_validate()`: validate parameters of a
/// sample.
pub(crate) fn fluid_sample_validate(sample: &mut Sample, mut buffer_size: u32) -> i32 {
    const SUPPORTED_FLAGS: i32 = EXCLUSIVE_FLAGS
        | FLUID_SAMPLETYPE_LINKED
        | FLUID_SAMPLETYPE_OGG_VORBIS
        | FLUID_SAMPLETYPE_ROM;

    /* ROM samples are unusable for us by definition */
    if sample.sampletype & FLUID_SAMPLETYPE_ROM != 0 {
        flog!(FLUID_WARN, "Sample '{}': ROM sample ignored", sample.name);
        return FLUID_FAILED;
    }

    if sample.sampletype & !SUPPORTED_FLAGS != 0 {
        flog!(
            FLUID_WARN,
            "Sample '{}' has unknown flags, possibly using an unsupported compression; sample ignored",
            sample.name
        );
        return FLUID_FAILED;
    }

    if (sample.sampletype & EXCLUSIVE_FLAGS) & ((sample.sampletype & EXCLUSIVE_FLAGS) - 1) != 0 {
        flog!(
            FLUID_INFO,
            "Sample '{}' should be either mono or left or right; using it anyway",
            sample.name
        );
    }

    if (sample.sampletype & FLUID_SAMPLETYPE_LINKED != 0)
        && (sample.sampletype & EXCLUSIVE_FLAGS != 0)
    {
        flog!(
            FLUID_INFO,
            "Linked sample '{}' should not be mono, left or right at the same time; using it anyway",
            sample.name
        );
    }

    if (sample.sampletype & EXCLUSIVE_FLAGS) == 0 {
        flog!(
            FLUID_INFO,
            "Sample '{}' has no flags set, assuming mono",
            sample.name
        );
        sample.sampletype = FLUID_SAMPLETYPE_MONO;
    }

    /* Ogg vorbis compressed samples in the SF3 format use byte indices for
     * sample start and end pointers before decompression. Standard SF2 samples
     * use sample word indices for all pointers, so use half the buffer_size
     * for validation. */
    if sample.sampletype & FLUID_SAMPLETYPE_OGG_VORBIS == 0 {
        if !buffer_size.is_multiple_of(2) {
            flog!(FLUID_WARN, "Sample '{}': invalid buffer size", sample.name);
            return FLUID_FAILED;
        }

        buffer_size /= 2;
    }

    if (sample.end > buffer_size) || (sample.start >= sample.end) {
        flog!(
            FLUID_WARN,
            "Sample '{}': invalid start/end file positions",
            sample.name
        );
        return FLUID_FAILED;
    }

    FLUID_OK
}

/// Translation of `fluid_sample_sanitize_loop()`: check the sample loop
/// pointers and optionally convert them to something usable in case they
/// are broken. Return a boolean indicating if the pointers have been
/// modified, so the user can be notified of possible audio glitches.
pub(crate) fn fluid_sample_sanitize_loop(sample: &mut Sample, buffer_size: u32) -> bool {
    let mut modified = false;
    let max_end = buffer_size / 2;
    /* In fluid_sample_t the sample end pointer points to the last sample, not
     * to the data word after the last sample. FIXME: why? */
    let sample_end = sample.end.wrapping_add(1);

    if sample.loopstart == sample.loopend {
        /* Some SoundFonts disable loops by setting loopstart = loopend. While
         * technically invalid, we decided to accept those samples anyway.
         * Before fluidsynth 2.2.5 we've set those indices to zero, as this
         * change was believed to be inaudible. This turned out to be an
         * incorrect assumption, as the loop points may still be modified by
         * loop offset modulators afterwards.
         */
        if sample.loopstart != sample.start {
            // Many soundfonts set loopstart == loopend == sample->start to disabled to loop.
            // Only report cases where it's not equal to the sample->start, to avoid spam.
            flog!(
                FLUID_DBG,
                "Sample '{}': zero length loop detected: loopstart == loopend == '{}', sample start '{}', using it anyway",
                sample.name,
                sample.loopstart as i32,
                sample.start as i32
            );
        }
    } else if sample.loopstart > sample.loopend {
        /* If loop start and end are reversed, try to swap them around and
         * continue validation */
        flog!(
            FLUID_DBG,
            "Sample '{}': reversed loop pointers '{}' - '{}', trying to fix",
            sample.name,
            sample.loopstart as i32,
            sample.loopend as i32
        );
        std::mem::swap(&mut sample.loopstart, &mut sample.loopend);
        modified = true;
    }

    /* The SoundFont 2.4 spec defines the loopstart index as the first sample
     * point of the loop while loopend is the first point AFTER the last sample
     * of the loop. However we cannot be sure whether any of loopend or end is
     * correct. Hours of thinking through this have concluded that it would be
     * best practice to mangle with loops as little as necessary by only making
     * sure the pointers are within sample->start to max_end. Incorrect
     * soundfont shall preferably fail loudly. */
    if (sample.loopstart < sample.start) || (sample.loopstart > max_end) {
        flog!(
            FLUID_DBG,
            "Sample '{}': invalid loop start '{}', setting to sample start '{}'",
            sample.name,
            sample.loopstart as i32,
            sample.start as i32
        );
        sample.loopstart = sample.start;
        modified = true;
    }

    if (sample.loopend < sample.start) || (sample.loopend > max_end) {
        flog!(
            FLUID_DBG,
            "Sample '{}': invalid loop end '{}', setting to sample end '{}'",
            sample.name,
            sample.loopend as i32,
            sample_end as i32
        );
        sample.loopend = sample_end;
        modified = true;
    }

    if (sample.loopstart > sample_end) || (sample.loopend > sample_end) {
        flog!(
            FLUID_DBG,
            "Sample '{}': loop range '{} - {}' after sample end '{}', using it anyway",
            sample.name,
            sample.loopstart as i32,
            sample.loopend as i32,
            sample_end as i32
        );
    }

    modified
}
