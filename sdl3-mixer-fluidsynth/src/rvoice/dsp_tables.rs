// FluidSynth - A Software Synthesizer: src/rvoice/fluid_rvoice_dsp_tables.h
// (which carries no notice of its own; it is under FluidSynth's license,
// the GNU Lesser General Public License version 2.1 or later, see COPYING).
// Modified 2026-10-17: translated into Rust from fluid_rvoice_dsp_tables.h
// in SDL_mixer 3.3.0's FluidSynth 2.5.7 (see NOTICE).

pub(crate) const FLUID_INTERP_MAX: usize = 256;
pub(crate) const SINC_INTERP_ORDER: usize = 7; /* 7th order constant */
pub(crate) const CUBIC_INTERP_ORDER: usize = 4; /* 4th order constant */
pub(crate) const LINEAR_INTERP_ORDER: usize = 2;
