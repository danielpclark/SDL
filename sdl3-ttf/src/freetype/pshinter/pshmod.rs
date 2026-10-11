// Rust translation of src/pshinter/pshmod.c from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2001-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType PostScript hinter module implementation (body).
//!
//! The module's globals functions are [`PSH_GLOBALS_FUNCS`]; its Type 1
//! and Type 2 hints functions (the hints recorder of `pshrec` and the
//! hinting algorithm of `pshalgo`) are only called by the old Type 1 and
//! CFF charstring interpreters, which SDL_ttf's bundled build does not
//! compile, and are not translated: a slot's `glyph_hints` is set to
//! [`T2HintsFuncs`] (or [`T1HintsFuncs`]) to record that the CFF (or
//! Type 1 or CID) driver found them, which is what decides whether the
//! glyph loaders scale hinted outlines again.

use super::super::base::ftobjs::*;

use super::pshglob::{PshGlobalsFuncsRec, PSH_GLOBALS_FUNCS};

/// The Type 2 hints functions (`T2_Hints_FuncsRec`) a glyph slot's
/// `glyph_hints` points to.
#[derive(Debug, Clone, Copy, Default)]
pub struct T2HintsFuncs;

/// The Type 1 hints functions (`T1_Hints_FuncsRec`) a glyph slot's
/// `glyph_hints` points to.
#[derive(Debug, Clone, Copy, Default)]
pub struct T1HintsFuncs;

/// `pshinter_get_globals_funcs`: returns global hints interface
pub fn pshinter_get_globals_funcs(_module: &FtModuleRec) -> &'static PshGlobalsFuncsRec {
    &PSH_GLOBALS_FUNCS
}

/// `pshinter_get_t1_funcs`: return Type 1 hints interface
pub fn pshinter_get_t1_funcs(_module: &FtModuleRec) -> T1HintsFuncs {
    T1HintsFuncs
}

/// `pshinter_get_t2_funcs`: return Type 2 hints interface
pub fn pshinter_get_t2_funcs(_module: &FtModuleRec) -> T2HintsFuncs {
    T2HintsFuncs
}

/// `PSHinter_Interface`
#[derive(Debug)]
pub struct PsHinterInterface {
    pub get_globals_funcs: fn(module: &FtModuleRec) -> &'static PshGlobalsFuncsRec,
    pub get_t1_funcs: fn(module: &FtModuleRec) -> T1HintsFuncs,
    pub get_t2_funcs: fn(module: &FtModuleRec) -> T2HintsFuncs,
}

/// `pshinter_interface`
pub static PSHINTER_INTERFACE: PsHinterInterface = PsHinterInterface {
    get_globals_funcs: pshinter_get_globals_funcs,
    get_t1_funcs: pshinter_get_t1_funcs,
    get_t2_funcs: pshinter_get_t2_funcs,
};

/// `pshinter_module_class`
pub static PSHINTER_MODULE_CLASS: FtModuleClass = FtModuleClass {
    module_flags: 0,
    module_name: "pshinter",
    module_version: 0x10000,
    module_requires: 0x20000,

    module_interface: FtModuleInterface::PsHinter(&PSHINTER_INTERFACE), /* module-specific interface */

    /* (`ps_hinter_init' and `ps_hinter_done' set up and finalize the */
    /* hints recorder, which is not translated)                       */
    module_init: None,
    module_done: None,
    get_interface: None,
};
