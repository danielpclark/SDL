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
// Modified 2026-10-17: translated into Rust from fluid_samplecache.c and
// fluid_samplecache.h in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).
// Memory locking (fluid_mlock(), for synth.lock-memory) has no portable
// safe Rust equivalent and is left out; upstream ignores its failure.

/* CACHED SAMPLE DATA LOADER
 *
 * This is a wrapper around fluid_sffile_read_sample_data that attempts to cache the read
 * data across all FluidSynth instances in a global (process-wide) list.
 */

use std::sync::{Arc, Mutex};

use crate::fluidsynth_h::*;
use crate::sfloader::sffile::{fluid_sffile_read_sample_data, SfData};
use crate::utils::sys::{fluid_log, FLUID_ERR};

/// Translation of `fluid_samplecache_entry_t`.
struct FluidSamplecacheEntry {
    /* The following members all form the cache key */
    filename: String,
    modification_time: i64,
    sf_samplepos: u32,
    sf_samplesize: u32,
    sf_sample24pos: u32,
    sf_sample24size: u32,
    sample_start: u32,
    sample_end: u32,
    sample_type: i32,
    /*  End of cache key members */
    sample_data: Arc<[i16]>,
    sample_data24: Option<Arc<[u8]>>,

    num_references: i32,
}

/// A sample's 16-bit data and its 24-bit extension, if any.
pub(crate) type SampleData = (Arc<[i16]>, Option<Arc<[u8]>>);

static SAMPLECACHE_LIST: Mutex<Vec<FluidSamplecacheEntry>> = Mutex::new(Vec::new());

/// Translation of `fluid_samplecache_load()`: the sample data (and
/// 24-bit data) for the samples from `sample_start` to `sample_end`, from
/// the cache or the file; the number of samples is the data's length.
pub(crate) fn fluid_samplecache_load(
    sf: &mut SfData,
    sample_start: u32,
    sample_end: u32,
    sample_type: i32,
    _try_mlock: bool,
) -> Option<SampleData> {
    let mtime = fluid_get_file_modification_time(&sf.fname).unwrap_or(0);

    let mut list = SAMPLECACHE_LIST.lock().unwrap_or_else(|e| e.into_inner());

    let idx = match get_samplecache_entry(&list, sf, sample_start, sample_end, sample_type, mtime) {
        Some(idx) => idx,
        None => {
            drop(list);
            let entry = new_samplecache_entry(sf, sample_start, sample_end, sample_type, mtime)?;

            list = SAMPLECACHE_LIST.lock().unwrap_or_else(|e| e.into_inner());
            list.insert(0, entry);
            0
        }
    };

    let entry = &mut list[idx];
    entry.num_references += 1;
    Some((entry.sample_data.clone(), entry.sample_data24.clone()))
}

/// Translation of `fluid_samplecache_unload()`.
pub(crate) fn fluid_samplecache_unload(sample_data: &Arc<[i16]>) -> i32 {
    let mut list = SAMPLECACHE_LIST.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(i) = list
        .iter()
        .position(|entry| Arc::ptr_eq(sample_data, &entry.sample_data))
    {
        let entry = &mut list[i];
        entry.num_references -= 1;

        if entry.num_references == 0 {
            list.remove(i);
        }

        return FLUID_OK;
    }

    fluid_log(FLUID_ERR, "Trying to free sample data not found in cache.");
    FLUID_FAILED
}

/* Private functions */
/// Translation of `new_samplecache_entry()`.
fn new_samplecache_entry(
    sf: &mut SfData,
    sample_start: u32,
    sample_end: u32,
    sample_type: i32,
    mtime: i64,
) -> Option<FluidSamplecacheEntry> {
    let (data, data24) = fluid_sffile_read_sample_data(sf, sample_start, sample_end, sample_type)?;

    Some(FluidSamplecacheEntry {
        filename: sf.fname.clone(),
        modification_time: mtime,
        sf_samplepos: sf.samplepos,
        sf_samplesize: sf.samplesize,
        sf_sample24pos: sf.sample24pos,
        sf_sample24size: sf.sample24size,
        sample_start,
        sample_end,
        sample_type,
        sample_data: Arc::from(data),
        sample_data24: data24.map(Arc::from),
        num_references: 0,
    })
}

/// Translation of `get_samplecache_entry()`.
fn get_samplecache_entry(
    list: &[FluidSamplecacheEntry],
    sf: &SfData,
    sample_start: u32,
    sample_end: u32,
    sample_type: i32,
    mtime: i64,
) -> Option<usize> {
    list.iter().position(|entry| {
        sf.fname == entry.filename
            && mtime == entry.modification_time
            && sf.samplepos == entry.sf_samplepos
            && sf.samplesize == entry.sf_samplesize
            && sf.sample24pos == entry.sf_sample24pos
            && sf.sample24size == entry.sf_sample24size
            && sample_start == entry.sample_start
            && sample_end == entry.sample_end
            && sample_type == entry.sample_type
    })
}

/// Translation of `fluid_get_file_modification_time()` (`st_mtime`, in
/// seconds).
fn fluid_get_file_modification_time(filename: &str) -> Option<i64> {
    let modified = std::fs::metadata(filename).ok()?.modified().ok()?;
    Some(match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs_f64().ceil() as i64),
    })
}

/// Translation of `fluid_samplecache_count_entries()` (only used for
/// tests).
#[cfg(test)]
pub(crate) fn fluid_samplecache_count_entries() -> i32 {
    SAMPLECACHE_LIST
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .len() as i32
}
