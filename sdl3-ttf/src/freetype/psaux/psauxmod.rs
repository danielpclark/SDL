// Rust translation of src/psaux/psauxmod.c from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2000-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType auxiliary PostScript module implementation (body).
//!
//! The module's interface (`PSAux_Interface`: the table, parser, builder,
//! decoder and charmap functions) is called directly by the drivers that
//! use it; the module records that it is there (`FT_Get_Module_Interface`
//! finds it).

use super::super::base::ftobjs::*;

/// `psaux_module_class`
pub static PSAUX_MODULE_CLASS: FtModuleClass = FtModuleClass {
    module_flags: 0,
    module_name: "psaux",
    module_version: 0x20000,
    module_requires: 0x20000,

    module_interface: FtModuleInterface::PsAux, /* module-specific interface */

    module_init: None,   /* module_init   */
    module_done: None,   /* module_done   */
    get_interface: None, /* get_interface */
};
