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
// (fluid_dls.cpp has no license notice of its own; it is part of
// FluidSynth, under the license above: see COPYING.)
// Modified 2026-10-17: translated into Rust from the part of
// fluid_dls.cpp in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE) that
// SDL_mixer reaches: fluid_dls_loader_load() failing to open the file
// (SDL_mixer's SoundFont names only open through its own loader). Reading
// DLS files is left out: one that opens is not loaded.

use crate::sfloader::sfont::{FluidSfont, SfLoader};
use crate::utils::sys::{flog, FLUID_ERR};

/// Translation of `fluid_dls_loader_load()`, as far as the
/// `fluid_dls_font` constructor's opening of the file.
pub(crate) fn fluid_dls_loader_load(loader: &SfLoader, filename: &str) -> Option<FluidSfont> {
    // new_fluid_dls_font()
    match (loader.file_callbacks.fopen)(filename) {
        None => {
            flog!(FLUID_ERR, "Exception thrown in fluid_dls_font constructor");
            /* log_exception() */
            flog!(FLUID_ERR, "exc: Unable to open file '{}'", filename);
        }
        Some(file) => {
            file.close();
            flog!(FLUID_ERR, "Exception thrown in fluid_dls_font constructor");
            flog!(
                FLUID_ERR,
                "exc: DLS files are not supported by this translation"
            );
        }
    }
    None
}
