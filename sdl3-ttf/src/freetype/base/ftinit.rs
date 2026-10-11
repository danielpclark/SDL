// Rust translation of src/base/ftinit.c and include/freetype/config/ftmodule.h
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType initialization layer (body).
//!
//! The purpose of this file is to implement the following two
//! functions:
//!
//! FT_Add_Default_Modules():
//!   This function is used to add the set of default modules to a
//!   fresh new library object.  The set is taken from the header file
//!   `freetype/config/ftmodule.h'.  See the document `FreeType 2.0
//!   Build System' for more information.
//!
//! FT_Init_FreeType():
//!   This function creates a system object for the current platform,
//!   builds a library out of it, then calls FT_Default_Drivers().
//!
//! The module list is upstream's (`ftmodule.h`, as SDL_ttf's bundled build
//! generates it), in its order, minus the modules not translated yet: the
//! Type 1, CID, PFR, Type 42, PCF and BDF drivers, and the OT-SVG
//! renderer.

use std::sync::Arc;

use super::super::fttypes::*;
use super::ftobjs::*;

/// `ft_default_modules`
fn ft_default_modules() -> Vec<FtModuleClassRef> {
    vec![
        FtModuleClassRef::Module(&crate::freetype::autofit::afmodule::AUTOFIT_MODULE_CLASS),
        FtModuleClassRef::Driver(&crate::freetype::truetype::ttdriver::TT_DRIVER_CLASS),
        /* FT_USE_MODULE( FT_Driver_ClassRec, t1_driver_class ) */
        FtModuleClassRef::Driver(&crate::freetype::cff::cffdrivr::CFF_DRIVER_CLASS),
        /* FT_USE_MODULE( FT_Driver_ClassRec, t1cid_driver_class ) */
        /* FT_USE_MODULE( FT_Driver_ClassRec, pfr_driver_class ) */
        /* FT_USE_MODULE( FT_Driver_ClassRec, t42_driver_class ) */
        FtModuleClassRef::Driver(&crate::freetype::winfonts::winfnt::WINFNT_DRIVER_CLASS),
        /* FT_USE_MODULE( FT_Driver_ClassRec, pcf_driver_class ) */
        /* FT_USE_MODULE( FT_Driver_ClassRec, bdf_driver_class ) */
        FtModuleClassRef::Module(&crate::freetype::psaux::psauxmod::PSAUX_MODULE_CLASS),
        FtModuleClassRef::Module(&crate::freetype::psnames::psmodule::PSNAMES_MODULE_CLASS),
        FtModuleClassRef::Module(&crate::freetype::pshinter::pshmod::PSHINTER_MODULE_CLASS),
        FtModuleClassRef::Module(&crate::freetype::sfnt::sfdriver::SFNT_MODULE_CLASS),
        FtModuleClassRef::Renderer(&crate::freetype::smooth::ftsmooth::FT_SMOOTH_RENDERER_CLASS),
        FtModuleClassRef::Renderer(&crate::freetype::raster::ftrend1::FT_RASTER1_RENDERER_CLASS),
        FtModuleClassRef::Renderer(&crate::freetype::sdf::ftsdfrend::FT_SDF_RENDERER_CLASS),
        FtModuleClassRef::Renderer(&crate::freetype::sdf::ftsdfrend::FT_BITMAP_SDF_RENDERER_CLASS),
        /* FT_USE_MODULE( FT_Renderer_Class, ft_svg_renderer_class ) */
        /* (not translated yet) */
    ]
}

/// `FT_Add_Default_Modules`
pub fn ft_add_default_modules(library: &mut FtLibraryRec) {
    for cur in ft_default_modules() {
        /* notify errors, but don't stop */
        let _ = ft_add_module(library, cur);
    }
}

const MAX_LENGTH: usize = 128;

/// `FT_Set_Default_Properties`
pub fn ft_set_default_properties(library: &FtLibraryRec) {
    let env = match std::env::var_os("FREETYPE_PROPERTIES") {
        Some(e) => e,
        None => return,
    };
    let env = env.to_string_lossy().into_owned();
    let env = env.as_bytes();

    let at = |i: usize| -> u8 { env.get(i).copied().unwrap_or(0) };

    let mut p = 0;
    while at(p) != 0 {
        /* skip leading whitespace and separators */
        if at(p) == b' ' || at(p) == b'\t' {
            p += 1;
            continue;
        }

        /* read module name, followed by `:' */
        let mut q = p;
        let mut module_name = Vec::new();
        for _ in 0..MAX_LENGTH {
            if at(p) == 0 || at(p) == b':' {
                break;
            }
            module_name.push(at(p));
            p += 1;
        }

        if at(p) == 0 || at(p) != b':' || p == q {
            break;
        }

        /* read property name, followed by `=' */
        p += 1;
        q = p;
        let mut property_name = Vec::new();
        for _ in 0..MAX_LENGTH {
            if at(p) == 0 || at(p) == b'=' {
                break;
            }
            property_name.push(at(p));
            p += 1;
        }

        if at(p) == 0 || at(p) != b'=' || p == q {
            break;
        }

        /* read property value, followed by whitespace (if any) */
        p += 1;
        q = p;
        let mut property_value = Vec::new();
        for _ in 0..MAX_LENGTH {
            if at(p) == 0 || at(p) == b' ' || at(p) == b'\t' {
                break;
            }
            property_value.push(at(p));
            p += 1;
        }

        if !(at(p) == 0 || at(p) == b' ' || at(p) == b'\t') || p == q {
            break;
        }

        /* we completely ignore errors */
        let _ = ft_property_string_set(
            library,
            &String::from_utf8_lossy(&module_name),
            &String::from_utf8_lossy(&property_name),
            &String::from_utf8_lossy(&property_value),
        );

        if at(p) == 0 {
            break;
        }
        p += 1;
    }
}

/// `FT_Init_FreeType`
pub fn ft_init_freetype() -> FtResult<FtLibrary> {
    /* build a library out of it, then fill it with the set of */
    /* default drivers.                                        */
    let mut library = ft_new_library();
    ft_add_default_modules(&mut library);

    let library = Arc::new(library);
    ft_set_default_properties(&library);

    Ok(library)
}

/// `FT_Done_FreeType`: the library goes when its last reference does.
pub fn ft_done_freetype(_library: FtLibrary) -> FtResult<()> {
    Ok(())
}
