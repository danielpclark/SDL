// Rust translation of src/svg/ftsvg.c, ftsvg.h and svgtypes.h (and the
// hook types of include/freetype/otsvg.h and the SVG interface of
// include/freetype/internal/svginterface.h) from FreeType (2.13.2, as
// SDL_ttf's external/freetype pins it).
// Copyright (C) 2022-2023 by David Turner, Robert Wilhelm, Werner Lemberg,
// and Moazin Khatti.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The FreeType SVG renderer interface (body).
//!
//! Translation notes: the renderer's fields (`SVG_RendererRec`) are the
//! module's `props`; the hooks' state (`FT_Pointer state`) is a boxed
//! value of the hooks' choosing. The render hook gets the face of the
//! slot (C's hooks reach it through `slot->face`), or `None` for slots
//! without one (`FT_Glyph_To_Bitmap`'s).

use std::any::Any;

use super::super::base::ftcalc::{ft_matrix_multiply, ft_mul_fix};
use super::super::base::ftmemory::ft_alloc;
use super::super::base::ftobjs::*;
use super::super::fttypes::*;

/// The hooks' state (`FT_Pointer`, which `init_svg` sets).
pub type SvgState = Option<Box<dyn Any + Send>>;

/// `SVG_Lib_Init_Func`
pub type SvgLibInitFunc = fn(data_pointer: &mut SvgState) -> FtResult<()>;
/// `SVG_Lib_Free_Func`
pub type SvgLibFreeFunc = fn(data_pointer: &mut SvgState);
/// `SVG_Lib_Render_Func` (with the slot's face, see the module
/// documentation)
pub type SvgLibRenderFunc = fn(
    slot: &mut FtGlyphSlotRec,
    face: Option<&mut FtFace>,
    data_pointer: &mut SvgState,
) -> FtResult<()>;
/// `SVG_Lib_Preset_Slot_Func`
pub type SvgLibPresetSlotFunc =
    fn(slot: &mut FtGlyphSlotRec, cache: bool, state: &mut SvgState) -> FtResult<()>;

/// `SVG_RendererHooks`: the hooks a library that renders SVG documents
/// (PlutoSVG's [`crate::plutosvg::ft::plutosvg_ft_svg_hooks`]) sets with the
/// `svg-hooks` property of the `ot-svg` module.
#[derive(Debug, Clone, Copy, Default)]
pub struct SvgRendererHooks {
    pub init_svg: Option<SvgLibInitFunc>,
    pub free_svg: Option<SvgLibFreeFunc>,
    pub render_svg: Option<SvgLibRenderFunc>,

    pub preset_slot: Option<SvgLibPresetSlotFunc>,
}

impl PartialEq for SvgRendererHooks {
    fn eq(&self, other: &Self) -> bool {
        fn same<T: Copy>(a: Option<T>, b: Option<T>, addr: fn(T) -> usize) -> bool {
            a.map(addr) == b.map(addr)
        }
        same(self.init_svg, other.init_svg, |f| f as usize)
            && same(self.free_svg, other.free_svg, |f| f as usize)
            && same(self.render_svg, other.render_svg, |f| f as usize)
            && same(self.preset_slot, other.preset_slot, |f| f as usize)
    }
}

impl Eq for SvgRendererHooks {}

/// `SVG_Interface` (the module's `module_interface`)
#[derive(Debug)]
pub struct SvgInterface {
    /// `preset_slot`
    pub preset_slot: fn(
        library: &FtLibraryRec,
        module: usize,
        slot: &mut FtGlyphSlotRec,
        cache: bool,
    ) -> FtResult<()>,
}

/// `SVG_RendererRec` (its fields after `root`)
#[derive(Default)]
pub struct SvgRendererRec {
    pub loaded: bool,
    pub hooks_set: bool,
    pub hooks: SvgRendererHooks, /* this holds hooks for SVG rendering          */
    pub state: SvgState,         /* a place for hooks to store state, if needed */
}

impl std::fmt::Debug for SvgRendererRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgRendererRec")
            .field("loaded", &self.loaded)
            .field("hooks_set", &self.hooks_set)
            .field("hooks", &self.hooks)
            .finish()
    }
}

/* FT_CONFIG_OPTION_SVG */

/// `ft_svg_init`
fn ft_svg_init(library: &mut FtLibraryRec, module: usize) -> FtResult<()> {
    let render = SvgRendererRec {
        loaded: false,
        hooks_set: false,
        ..Default::default()
    };

    let m = &library.modules[module];
    *m.props.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(render));

    Ok(())
}

/// `ft_svg_done`
fn ft_svg_done(module: &FtModuleRec) {
    module.with_props(|render: &mut SvgRendererRec| {
        if render.loaded && render.hooks_set {
            if let Some(free_svg) = render.hooks.free_svg {
                free_svg(&mut render.state);
            }
        }

        render.loaded = false;
    });
}

/// `ft_svg_preset_slot`
fn ft_svg_preset_slot(
    library: &FtLibraryRec,
    module: usize,
    slot: &mut FtGlyphSlotRec,
    cache: bool,
) -> FtResult<()> {
    library.modules[module]
        .with_props(|svg_renderer: &mut SvgRendererRec| {
            let hooks = svg_renderer.hooks;

            if !svg_renderer.hooks_set {
                /* Hooks are NOT set.  Can't render OT-SVG glyphs */
                return Err(FT_ERR_MISSING_SVG_HOOKS);
            }

            if !svg_renderer.loaded {
                /* ft_svg_preset_slot: first presetting call, calling init hook */
                if let Some(init_svg) = hooks.init_svg {
                    let _ = init_svg(&mut svg_renderer.state);
                }

                svg_renderer.loaded = true;
            }

            match hooks.preset_slot {
                Some(preset_slot) => preset_slot(slot, cache, &mut svg_renderer.state),
                None => Err(FT_ERR_MISSING_SVG_HOOKS),
            }
        })
        .unwrap_or(Err(FT_ERR_MISSING_SVG_HOOKS))
}

/// `ft_svg_render`
fn ft_svg_render(
    library: &FtLibraryRec,
    renderer: usize,
    slot: &mut FtGlyphSlotRec,
    mode: FtRenderMode,
    _origin: Option<&FtVector>,
    face: Option<&mut FtFace>,
) -> FtResult<()> {
    if mode != FT_RENDER_MODE_NORMAL {
        return Err(FT_ERR_BAD_ARGUMENT);
    }

    let module = &library.modules[renderer];
    let hooks = module.with_props(|svg_renderer: &mut SvgRendererRec| {
        if !svg_renderer.hooks_set {
            /* Hooks are NOT set.  Can't render OT-SVG glyphs */
            return None;
        }

        if !svg_renderer.loaded {
            /* ft_svg_render: first rendering, calling init hook */
            if let Some(init_svg) = svg_renderer.hooks.init_svg {
                let _ = init_svg(&mut svg_renderer.state);
            }

            svg_renderer.loaded = true;
        }
        Some(svg_renderer.hooks)
    });
    let Some(Some(hooks)) = hooks else {
        return Err(FT_ERR_MISSING_SVG_HOOKS);
    };

    let _ = ft_svg_preset_slot(library, renderer, slot, true);

    let size_image_buffer = (slot.bitmap.pitch as i64 as u64).wrapping_mul(slot.bitmap.rows as u64);
    /* No `FT_QALLOC` here since we need a clean, empty canvas */
    /* to start with.                                          */
    let size = if size_image_buffer > i64::MAX as u64 {
        -1
    } else {
        size_image_buffer as FtLong
    };
    slot.bitmap.buffer = ft_alloc(size)?;

    let error = module
        .with_props(|svg_renderer: &mut SvgRendererRec| match hooks.render_svg {
            Some(render_svg) => render_svg(slot, face, &mut svg_renderer.state),
            None => Err(FT_ERR_MISSING_SVG_HOOKS),
        })
        .unwrap_or(Err(FT_ERR_MISSING_SVG_HOOKS));
    if error.is_err() {
        slot.bitmap.buffer = Vec::new();
    } else {
        slot.internal.flags |= FT_GLYPH_OWN_BITMAP;
    }

    error
}

/// `svg_interface`
pub static SVG_INTERFACE: SvgInterface = SvgInterface {
    preset_slot: ft_svg_preset_slot, /* Preset_Bitmap_Func preset_slot */
};

/// `ft_svg_property_set`
fn ft_svg_property_set(
    module: &FtModuleRec,
    property_name: &str,
    value: &FtPropertyValue,
    value_is_string: bool,
) -> FtResult<()> {
    if property_name == "svg-hooks" {
        if value_is_string {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        let FtPropertyValue::SvgHooks(hooks) = value else {
            return Err(FT_ERR_INVALID_ARGUMENT);
        };

        if hooks.init_svg.is_none()
            || hooks.free_svg.is_none()
            || hooks.render_svg.is_none()
            || hooks.preset_slot.is_none()
        {
            /* ft_svg_property_set: SVG rendering hooks not set because */
            /* at least one function pointer is NULL                    */
            return Err(FT_ERR_INVALID_ARGUMENT);
        }

        module.with_props(|renderer: &mut SvgRendererRec| {
            renderer.hooks = *hooks;
            renderer.hooks_set = true;
        });
        Ok(())
    } else {
        Err(FT_ERR_MISSING_PROPERTY)
    }
}

/// `ft_svg_property_get`
fn ft_svg_property_get(module: &FtModuleRec, property_name: &str) -> FtResult<FtPropertyValue> {
    if property_name == "svg-hooks" {
        let hooks = module
            .with_props(|renderer: &mut SvgRendererRec| renderer.hooks)
            .unwrap_or_default();
        Ok(FtPropertyValue::SvgHooks(hooks))
    } else {
        Err(FT_ERR_MISSING_PROPERTY)
    }
}

/// `ft_svg_service_properties`
static FT_SVG_SERVICE_PROPERTIES: FtServicePropertiesRec = FtServicePropertiesRec {
    set_property: ft_svg_property_set, /* FT_Properties_SetFunc set_property */
    get_property: ft_svg_property_get, /* FT_Properties_GetFunc get_property */
};

/// `ft_svg_get_interface` (with `ft_svg_services`)
fn ft_svg_get_interface(_module: &FtModuleRec, ft_svg_interface: &str) -> Option<FtService> {
    if ft_svg_interface == FT_SERVICE_ID_PROPERTIES {
        return Some(FtService::Properties(&FT_SVG_SERVICE_PROPERTIES));
    }

    None
}

/// `ft_svg_transform`
fn ft_svg_transform(
    slot: &mut FtGlyphSlotRec,
    matrix: Option<&FtMatrix>,
    delta: Option<&FtVector>,
) -> FtResult<()> {
    let Some(doc) = slot.other.as_mut() else {
        return Ok(());
    };

    let tmp_matrix = FtMatrix {
        xx: 0x10000,
        xy: 0,
        yx: 0,
        yy: 0x10000,
    };
    let tmp_delta = FtVector { x: 0, y: 0 };

    let matrix = matrix.unwrap_or(&tmp_matrix);
    let delta = delta.unwrap_or(&tmp_delta);

    let mut a = doc.transform;
    let b = *matrix;
    ft_matrix_multiply(&b, &mut a);

    let x = ft_mul_fix(matrix.xx, doc.delta.x)
        .wrapping_add(ft_mul_fix(matrix.xy, doc.delta.y))
        .wrapping_add(delta.x);
    let y = ft_mul_fix(matrix.yx, doc.delta.x)
        .wrapping_add(ft_mul_fix(matrix.yy, doc.delta.y))
        .wrapping_add(delta.y);

    doc.delta.x = x;
    doc.delta.y = y;
    doc.transform = a;

    Ok(())
}

/// `ft_svg_renderer_class`
pub static FT_SVG_RENDERER_CLASS: FtRendererClass = FtRendererClass {
    root: FtModuleClass {
        module_flags: FT_MODULE_RENDERER,

        module_name: "ot-svg",
        module_version: 0x10000,
        module_requires: 0x20000,

        module_interface: FtModuleInterface::Svg(&SVG_INTERFACE), /* module specific interface */

        module_init: Some(ft_svg_init), /* FT_Module_Constructor module_init   */
        module_done: Some(ft_svg_done), /* FT_Module_Destructor  module_done   */
        get_interface: Some(ft_svg_get_interface), /* FT_Module_Requester   get_interface */
    },

    glyph_format: FT_GLYPH_FORMAT_SVG,

    render_glyph: Some(ft_svg_render), /* FT_Renderer_RenderFunc    render_glyph    */
    transform_glyph: Some(ft_svg_transform), /* FT_Renderer_TransformFunc transform_glyph */
    get_glyph_cbox: None,              /* FT_Renderer_GetCBoxFunc   get_glyph_cbox  */
    set_mode: None,                    /* FT_Renderer_SetModeFunc   set_mode        */
    raster_class: None,                /* FT_Raster_Funcs*          raster_class    */
};
