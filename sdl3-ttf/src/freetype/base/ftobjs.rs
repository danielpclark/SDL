// Rust translation of src/base/ftobjs.c, include/freetype/internal/ftobjs.h,
// ftdrv.h, ftmodapi.h, ftrender.h and the object types of freetype.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! The FreeType private base classes (body).
//!
//! How the object model is carried over:
//!
//! * The library ([`FtLibraryRec`]) is shared (`Arc`) and immutable once
//!   built, except for the module properties and the LCD geometry, which
//!   sit behind mutexes. Modules live in its table; a face names its driver
//!   by its index there, and keeps a reference to the library (C's
//!   `face->driver->root.library`). The drivers' face lists are gone:
//!   faces are owned by their users and the library outlives them.
//! * A face is an [`FtFace`]: the driver's face record (`TT_FaceRec` for
//!   the TrueType driver), which starts with the [`FtFaceRec`] root as in
//!   C, and dereferences to it.
//! * A face has one glyph slot and one size, both in the face record
//!   (`face->glyph`, `face->size`); `FT_New_Size`/`FT_Done_Size`/
//!   `FT_Activate_Size` for further sizes aren't provided. The driver's
//!   functions that take a slot and a size take the face instead.
//! * The class records of modules, drivers, renderers and charmaps are
//!   `static` tables of function pointers, as in C; the services a module
//!   exports through `get_interface` are an enum of such tables.

use std::any::Any;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};

use super::super::ftimage::{FtRasterFuncs, FtRasterParams, FtRasterSource};
use super::super::fttypes::*;
use super::super::tttables::*;
use super::super::tttypes::TtFaceRec;
use super::ftcalc::*;
use super::ftgloadr::{FtGlyphLoaderRec, FtSubGlyphRec};
use super::ftlcdfil::ft_lcd_padding;
use super::ftmemory::ft_qalloc;
use super::ftoutln::{
    ft_outline_check, ft_outline_get_cbox, ft_outline_transform, ft_outline_translate,
    ft_vector_transform,
};
use super::ftrfork::{
    ft_raccess_get_data_offsets, ft_raccess_get_header_info, FT_MAC_RFORK_MAX_LEN,
};
use super::ftstream::{FtStream, FtStreamRec};

/*************************************************************************/
/*                                                                       */
/*                    M O D U L E   C L A S S E S                        */
/*                                                                       */
/*************************************************************************/

pub const FT_MODULE_FONT_DRIVER: FtULong = 1; /* this module is a font driver  */
pub const FT_MODULE_RENDERER: FtULong = 2; /* this module is a renderer     */
pub const FT_MODULE_HINTER: FtULong = 4; /* this module is a glyph hinter */
pub const FT_MODULE_STYLER: FtULong = 8; /* this module is a styler       */

pub const FT_MODULE_DRIVER_SCALABLE: FtULong = 0x100; /* the driver supports      */
/* scalable fonts           */
pub const FT_MODULE_DRIVER_NO_OUTLINES: FtULong = 0x200; /* the driver does not      */
/* support vector outlines  */
pub const FT_MODULE_DRIVER_HAS_HINTER: FtULong = 0x400; /* the driver provides its  */
/* own hinter               */
pub const FT_MODULE_DRIVER_HINTS_LIGHTLY: FtULong = 0x800; /* the driver's hinter      */
/* produces `light' results */

/// `FT_MAX_MODULES`
pub const FT_MAX_MODULES: usize = 32;

/// A property value for `FT_Property_Set`/`FT_Property_Get` (C's
/// `void*`, typed); `Str` is used by `ft_property_string_set`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FtPropertyValue {
    Int(FtInt),
    UInt(FtUInt),
    Bool(bool),
    IntArray(Vec<FtInt>),
    Str(String),
}

/// `FT_Service_PropertiesRec`
#[derive(Debug)]
pub struct FtServicePropertiesRec {
    pub set_property: fn(
        module: &FtModuleRec,
        property_name: &str,
        value: &FtPropertyValue,
        value_is_string: bool,
    ) -> FtResult<()>,
    pub get_property: fn(module: &FtModuleRec, property_name: &str) -> FtResult<FtPropertyValue>,
}

/// `FT_Service_SFNT_TableRec`
#[derive(Debug)]
pub struct FtServiceSfntTableRec {
    pub load_table: fn(
        face: &mut FtFace,
        tag: FtULong,
        offset: FtLong,
        buffer: Option<&mut [u8]>,
        length: &mut FtULong,
    ) -> FtResult<()>,
    pub get_table: for<'a> fn(face: &'a FtFace, tag: FtSfntTag) -> Option<FtSfntTable<'a>>,
    pub table_info: fn(
        face: &FtFace,
        idx: FtUInt,
        tag: &mut FtULong,
        offset: &mut FtULong,
        length: &mut FtULong,
    ) -> FtResult<()>,
}

/// `FT_Service_GlyphDictRec`
#[derive(Debug)]
pub struct FtServiceGlyphDictRec {
    pub get_name: fn(face: &mut FtFace, glyph_index: FtUInt, buffer: &mut [u8]) -> FtResult<()>,
    pub name_index: fn(face: &mut FtFace, glyph_name: &[u8]) -> FtUInt,
}

/// `FT_Service_PsFontNameRec`
#[derive(Debug)]
pub struct FtServicePsFontNameRec {
    pub get_ps_font_name: fn(face: &mut FtFace) -> Option<String>,
}

/// `TT_CMapInfo`
#[derive(Debug, Clone, Copy, Default)]
pub struct TtCMapInfo {
    pub language: FtULong,
    pub format: FtLong,
}

/// `FT_Service_TTCMapsRec`
#[derive(Debug)]
pub struct FtServiceTtCMapsRec {
    pub get_cmap_info: fn(face: &FtFace, charmap: usize) -> FtResult<TtCMapInfo>,
}

/// `FT_TrueTypeEngineType`
pub type FtTrueTypeEngineType = u32;
pub const FT_TRUETYPE_ENGINE_TYPE_NONE: FtTrueTypeEngineType = 0;
pub const FT_TRUETYPE_ENGINE_TYPE_UNPATENTED: FtTrueTypeEngineType = 1;
pub const FT_TRUETYPE_ENGINE_TYPE_PATENTED: FtTrueTypeEngineType = 2;

/// A service (`ft_module_get_service()` and `get_interface`).
#[derive(Debug, Clone, Copy)]
pub enum FtService {
    Properties(&'static FtServicePropertiesRec),
    SfntTable(&'static FtServiceSfntTableRec),
    GlyphDict(&'static FtServiceGlyphDictRec),
    PsFontName(&'static FtServicePsFontNameRec),
    TtCMaps(&'static FtServiceTtCMapsRec),
    TrueTypeEngine(FtTrueTypeEngineType),
    /// the font format service (`FT_Get_Font_Format`)
    FontFormat(&'static str),
    /// the Windows FNT driver's header service (`FT_Get_WinFNT_Header`)
    WinFnt(&'static super::ftwinfnt::FtServiceWinFntRec),
}

pub const FT_SERVICE_ID_PROPERTIES: &str = "properties";
pub const FT_SERVICE_ID_SFNT_TABLE: &str = "sfnt-table";
pub const FT_SERVICE_ID_GLYPH_DICT: &str = "glyph-dict";
pub const FT_SERVICE_ID_POSTSCRIPT_FONT_NAME: &str = "postscript-font-name";
pub const FT_SERVICE_ID_TT_CMAP: &str = "tt-cmaps";
pub const FT_SERVICE_ID_TRUETYPE_ENGINE: &str = "truetype-engine";
pub const FT_SERVICE_ID_FONT_FORMAT: &str = "font-format";

/// `FT_FONT_FORMAT_TRUETYPE`
pub const FT_FONT_FORMAT_TRUETYPE: &str = "TrueType";

/// `FT_AutoHinter_InterfaceRec` (the interface an auto-hinter module
/// exports as its `module_interface`)
#[derive(Debug)]
pub struct FtAutoHinterInterfaceRec {
    /// `load_glyph`: loads and hints `glyph_index` into `face.glyph`
    pub load_glyph: fn(
        library: &FtLibraryRec,
        hinter: usize,
        face: &mut FtFace,
        glyph_index: FtUInt,
        load_flags: FtInt32,
    ) -> FtResult<()>,
}

/// A module's `module_interface`.
#[derive(Debug, Clone, Copy)]
pub enum FtModuleInterface {
    None,
    AutoHinter(&'static FtAutoHinterInterfaceRec),
    /// the PostScript auxiliary module's (`PSAux_Interface`, whose
    /// functions are called directly)
    PsAux,
    /// the PostScript hinter's (`PSHinter_Interface`)
    PsHinter(&'static super::super::pshinter::pshmod::PsHinterInterface),
}

/// `FT_Module_Class`
#[derive(Debug)]
pub struct FtModuleClass {
    pub module_flags: FtULong,
    pub module_name: &'static str,
    pub module_version: FtFixed,
    pub module_requires: FtFixed,

    pub module_interface: FtModuleInterface,

    /// `module_init`: sets the module's initial properties (and may touch
    /// the library being built, as the smooth renderer's does)
    pub module_init: Option<fn(library: &mut FtLibraryRec, module: usize) -> FtResult<()>>,
    pub module_done: Option<fn(module: &FtModuleRec)>,
    pub get_interface: Option<fn(module: &FtModuleRec, service_id: &str) -> Option<FtService>>,
}

/// `FT_Driver_ClassRec`
#[derive(Debug)]
pub struct FtDriverClassRec {
    pub root: FtModuleClass,

    /// makes the driver's face record around a new root (C allocates
    /// `face_object_size` bytes)
    pub new_face: fn(root: FtFaceRec) -> FtFace,

    pub init_face: Option<
        fn(face: &mut FtFace, typeface_index: FtInt, params: &[FtParameter]) -> FtResult<()>,
    >,
    pub done_face: Option<fn(face: &mut FtFace)>,

    pub init_size: Option<fn(face: &mut FtFace) -> FtResult<()>>,
    pub done_size: Option<fn(face: &mut FtFace)>,

    pub init_slot: Option<fn(slot: &mut FtGlyphSlotRec) -> FtResult<()>>,
    pub done_slot: Option<fn(slot: &mut FtGlyphSlotRec)>,

    /// `load_glyph`, into `face.glyph` at `face.size`
    pub load_glyph:
        Option<fn(face: &mut FtFace, glyph_index: FtUInt, load_flags: FtInt32) -> FtResult<()>>,

    pub get_kerning: Option<
        fn(face: &mut FtFace, left_glyph: FtUInt, right_glyph: FtUInt) -> FtResult<FtVector>,
    >,
    pub attach_file: Option<fn(face: &mut FtFace, stream: &mut FtStreamRec) -> FtResult<()>>,
    pub get_advances: Option<
        fn(
            face: &mut FtFace,
            first: FtUInt,
            count: FtUInt,
            flags: FtInt32,
            advances: &mut [FtFixed],
        ) -> FtResult<()>,
    >,

    /* since version 2.2 */
    pub request_size: Option<fn(face: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()>>,
    pub select_size: Option<fn(face: &mut FtFace, size_index: FtULong) -> FtResult<()>>,
}

/// `FT_Renderer_Class`
#[derive(Debug)]
pub struct FtRendererClass {
    pub root: FtModuleClass,

    pub glyph_format: FtGlyphFormat,

    /// `render_glyph`
    pub render_glyph: Option<
        fn(
            library: &FtLibraryRec,
            renderer: usize,
            slot: &mut FtGlyphSlotRec,
            mode: FtRenderMode,
            origin: Option<&FtVector>,
        ) -> FtResult<()>,
    >,
    /// `transform_glyph`
    pub transform_glyph: Option<
        fn(
            slot: &mut FtGlyphSlotRec,
            matrix: Option<&FtMatrix>,
            delta: Option<&FtVector>,
        ) -> FtResult<()>,
    >,
    /// `get_glyph_cbox`
    pub get_glyph_cbox: Option<fn(slot: &FtGlyphSlotRec) -> FtBBox>,
    /// `set_mode`
    pub set_mode: Option<fn(module: &FtModuleRec, mode_tag: FtULong) -> FtResult<()>>,

    pub raster_class: Option<&'static FtRasterFuncs>,
}

/// A module's class record (C casts `FT_Module_Class*` to the derived
/// class records).
#[derive(Debug, Clone, Copy)]
pub enum FtModuleClassRef {
    Module(&'static FtModuleClass),
    Driver(&'static FtDriverClassRec),
    Renderer(&'static FtRendererClass),
}

impl FtModuleClassRef {
    /// The `FT_Module_Class` root.
    pub fn root(&self) -> &'static FtModuleClass {
        match *self {
            FtModuleClassRef::Module(c) => c,
            FtModuleClassRef::Driver(c) => &c.root,
            FtModuleClassRef::Renderer(c) => &c.root,
        }
    }
}

/// `FT_ModuleRec` (with the derived driver and renderer records: their
/// extra fields are the class's, the raster objects are stateless). The
/// module's own data (`TT_DriverRec`'s interpreter version, the
/// auto-hinter's and SDF renderer's properties, ...) is kept in `props`.
pub struct FtModuleRec {
    pub clazz: FtModuleClassRef,
    pub props: Mutex<Option<Box<dyn Any + Send>>>,
}

impl std::fmt::Debug for FtModuleRec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtModuleRec")
            .field("name", &self.clazz.root().module_name)
            .finish()
    }
}

impl FtModuleRec {
    /// `FT_MODULE_IS_DRIVER`
    pub fn is_driver(&self) -> bool {
        self.clazz.root().module_flags & FT_MODULE_FONT_DRIVER != 0
    }
    /// `FT_MODULE_IS_RENDERER`
    pub fn is_renderer(&self) -> bool {
        self.clazz.root().module_flags & FT_MODULE_RENDERER != 0
    }
    /// `FT_MODULE_IS_HINTER`
    pub fn is_hinter(&self) -> bool {
        self.clazz.root().module_flags & FT_MODULE_HINTER != 0
    }
    /// The driver class of a driver module.
    pub fn driver_class(&self) -> &'static FtDriverClassRec {
        match self.clazz {
            FtModuleClassRef::Driver(c) => c,
            _ => panic!("not a driver module"),
        }
    }
    /// The renderer class of a renderer module.
    pub fn renderer_class(&self) -> &'static FtRendererClass {
        match self.clazz {
            FtModuleClassRef::Renderer(c) => c,
            _ => panic!("not a renderer module"),
        }
    }

    /// Runs `f` on the module's properties, downcast to `T`.
    pub fn with_props<T: 'static, R>(&self, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let mut guard = self.props.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_mut().and_then(|b| b.downcast_mut::<T>()).map(f)
    }
}

/// `FT_LibraryRec`
#[derive(Debug)]
pub struct FtLibraryRec {
    pub version_major: FtInt,
    pub version_minor: FtInt,
    pub version_patch: FtInt,

    pub modules: Vec<FtModuleRec>, /* module objects  */

    /// the renderers list (module indices, in list order)
    pub renderers: Vec<usize>,
    /// current outline renderer
    pub cur_renderer: Option<usize>,
    /// auto-hinter module
    pub auto_hinter: Option<usize>,

    /// `lcd_geometry` (Harmony LCD rendering:
    /// `FT_CONFIG_OPTION_SUBPIXEL_RENDERING` is undefined)
    pub lcd_geometry: Mutex<[FtVector; 3]>,
}

/// `FT_Library`
pub type FtLibrary = Arc<FtLibraryRec>;

impl FtLibraryRec {
    /// `FT_Get_Module` as an index.
    pub fn get_module(&self, module_name: &str) -> Option<usize> {
        self.modules
            .iter()
            .position(|m| m.clazz.root().module_name == module_name)
    }

    /// The LCD geometry (`library->lcd_geometry`).
    pub fn lcd_geometry(&self) -> [FtVector; 3] {
        *self.lcd_geometry.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `renderer->raster_render( renderer->raster, params )` for an
    /// outline.
    pub fn renderer_raster_render(
        &self,
        renderer: usize,
        outline: &FtOutline,
        params: &mut FtRasterParams,
    ) -> FtResult<()> {
        match self.modules[renderer].renderer_class().raster_class {
            Some(rc) => (rc.raster_render)(FtRasterSource::Outline(outline), params),
            None => Err(FT_ERR_CANNOT_RENDER_GLYPH),
        }
    }
}

/*************************************************************************/
/*                                                                       */
/*               FACE, SIZE & GLYPH SLOT OBJECTS                         */
/*                                                                       */
/*************************************************************************/

/// `FT_Parameter` (with its `data` pointer typed)
#[derive(Debug, Clone)]
pub struct FtParameter {
    pub tag: FtULong,
    pub data: FtParameterData,
}

/// The `data` of an [`FtParameter`].
#[derive(Debug, Clone)]
pub enum FtParameterData {
    None,
    Bool(bool),
    Int32(FtInt32),
    Bytes(Vec<u8>),
}

pub const FT_PARAM_TAG_IGNORE_TYPOGRAPHIC_FAMILY: FtULong =
    ft_make_tag(b'i', b'g', b'p', b'f') as FtULong;
pub const FT_PARAM_TAG_IGNORE_TYPOGRAPHIC_SUBFAMILY: FtULong =
    ft_make_tag(b'i', b'g', b'p', b's') as FtULong;
pub const FT_PARAM_TAG_INCREMENTAL: FtULong = ft_make_tag(b'i', b'n', b'c', b'r') as FtULong;
pub const FT_PARAM_TAG_IGNORE_SBIX: FtULong = ft_make_tag(b'i', b's', b'b', b'x') as FtULong;
pub const FT_PARAM_TAG_LCD_FILTER_WEIGHTS: FtULong = ft_make_tag(b'l', b'c', b'd', b'f') as FtULong;
pub const FT_PARAM_TAG_RANDOM_SEED: FtULong = ft_make_tag(b's', b'e', b'e', b'd') as FtULong;
pub const FT_PARAM_TAG_STEM_DARKENING: FtULong = ft_make_tag(b'd', b'a', b'r', b'k') as FtULong;
pub const FT_PARAM_TAG_UNPATENTED_HINTING: FtULong = ft_make_tag(b'u', b'n', b'p', b'a') as FtULong;

/// `FT_Open_Args`: what to open (`flags` selects among the memory, stream
/// and driver fields, as in C).
#[derive(Debug, Default)]
pub struct FtOpenArgs {
    pub flags: FtUInt,
    pub memory_base: Option<Arc<[u8]>>,
    pub stream: Option<FtStream>,
    /// `driver`: a module index
    pub driver: Option<usize>,
    pub params: Vec<FtParameter>,
}

/// `FT_CharMapRec` (its `face` is the face holding it)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FtCharMapRec {
    pub encoding: FtEncoding,
    pub platform_id: FtUShort,
    pub encoding_id: FtUShort,
}

/// `FT_CMap_ClassRec`
#[derive(Debug)]
pub struct FtCMapClassRec {
    pub char_index: fn(cmap: &FtCMapRec, char_code: FtUInt32) -> FtUInt,
    pub char_next: fn(cmap: &mut FtCMapRec, achar_code: &mut FtUInt32) -> FtUInt,

    /* Subsequent entries are special ones for format 14 -- the variant */
    /* selector subtable which behaves like no other                    */
    pub char_var_index: Option<
        fn(
            cmap: &FtCMapRec,
            unicode_cmap: &FtCMapRec,
            char_code: FtUInt32,
            variant_selector: FtUInt32,
        ) -> FtUInt,
    >,
    pub char_var_default:
        Option<fn(cmap: &FtCMapRec, char_code: FtUInt32, variant_selector: FtUInt32) -> FtInt>,
    pub variant_list: Option<fn(cmap: &FtCMapRec) -> Option<Vec<FtUInt32>>>,
    pub charvariant_list:
        Option<fn(cmap: &FtCMapRec, char_code: FtUInt32) -> Option<Vec<FtUInt32>>>,
    pub variantchar_list:
        Option<fn(cmap: &FtCMapRec, variant_selector: FtUInt32) -> Option<Vec<FtUInt32>>>,
}

/// The data of a charmap object (the derived `TT_CMapRec`,
/// `PS_UnicodesRec`, ...).
#[derive(Debug, Clone)]
pub enum FtCMapData {
    None,
    Tt(super::super::sfnt::ttcmap::TtCMapData),
    PsUnicodes(super::super::psnames::psmodule::PsUnicodesRec),
    /// the CFF driver's encoding charmap (`CFF_CMapStdRec`: its `gids`
    /// are the font encoding's `codes`)
    CffEncoding(Box<[FtUShort; 256]>),
    /// the Windows FNT driver's charmap (`FNT_CMapRec`)
    Fnt(super::super::winfonts::winfnt::FntCMapRec),
    /// the BDF driver's charmap (`BDF_CMapRec`)
    Bdf(super::super::bdf::bdfdrivr::BdfCMapRec),
}

/// `FT_CMapRec`
#[derive(Debug, Clone)]
pub struct FtCMapRec {
    pub charmap: FtCharMapRec,
    pub clazz: &'static FtCMapClassRec,
    pub data: FtCMapData,
}

/// `FT_Face_InternalRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct FtFaceInternalRec {
    pub transform_matrix: FtMatrix,
    pub transform_delta: FtVector,
    pub transform_flags: FtInt,

    pub no_stem_darkening: FtChar,
    pub random_seed: FtInt32,

    pub refcount: FtInt,
}

pub const FT_GLYPH_OWN_BITMAP: FtUInt = 0x1;
pub const FT_GLYPH_OWN_GZIP_SVG: FtUInt = 0x2;

/// `FT_Slot_InternalRec`
#[derive(Debug, Default)]
pub struct FtSlotInternalRec {
    pub loader: Option<FtGlyphLoaderRec>,
    pub flags: FtUInt,
    pub glyph_transformed: bool,
    pub glyph_matrix: FtMatrix,
    pub glyph_delta: FtVector,
    pub glyph_hints: Option<Box<dyn Any + Send>>,

    pub load_flags: FtInt32,
}

/// `FT_SVG_DocumentRec` (`slot->other` of faces with SVG glyphs)
#[derive(Debug, Clone, Default)]
pub struct FtSvgDocumentRec {
    pub svg_document: Vec<u8>,
    pub svg_document_length: FtULong,

    pub metrics: FtSizeMetrics,
    pub units_per_EM: FtUShort,

    pub start_glyph_id: FtUShort,
    pub end_glyph_id: FtUShort,

    pub transform: FtMatrix,
    pub delta: FtVector,
}

/// `FT_GlyphSlotRec`
#[derive(Debug, Default)]
#[allow(non_snake_case)]
pub struct FtGlyphSlotRec {
    pub library: Option<FtLibrary>,
    /// the face's driver (module index), for `slot->face->driver`
    pub driver: Option<usize>,
    /// `slot->face->face_flags`, for the slot functions that read it
    pub face_flags: FtLong,

    pub glyph_index: FtUInt,

    pub metrics: FtGlyphMetrics,
    pub linearHoriAdvance: FtFixed,
    pub linearVertAdvance: FtFixed,
    pub advance: FtVector,

    pub format: FtGlyphFormat,

    pub bitmap: FtBitmap,
    pub bitmap_left: FtInt,
    pub bitmap_top: FtInt,

    pub outline: FtOutline,

    pub num_subglyphs: FtUInt,
    pub subglyphs: Vec<FtSubGlyphRec>,

    /// `control_data` (the glyph's instructions)
    pub control_data: Vec<u8>,
    pub control_len: i64,

    pub lsb_delta: FtPos,
    pub rsb_delta: FtPos,

    /// `other` (an SVG document)
    pub other: Option<Box<FtSvgDocumentRec>>,

    pub internal: FtSlotInternalRec,
}

/// `FT_Size_InternalRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct FtSizeInternalRec {
    pub autohint_mode: FtRenderMode,
    pub autohint_metrics: FtSizeMetrics,
}

/// `FT_SizeRec`
#[derive(Debug, Clone, Copy, Default)]
pub struct FtSizeRec {
    pub metrics: FtSizeMetrics, /* size metrics */
    pub internal: FtSizeInternalRec,
}

/// `FT_FaceRec`
#[derive(Debug, Default)]
#[allow(non_snake_case)]
pub struct FtFaceRec {
    pub num_faces: FtLong,
    pub face_index: FtLong,

    pub face_flags: FtLong,
    pub style_flags: FtLong,

    pub num_glyphs: FtLong,

    pub family_name: Option<String>,
    pub style_name: Option<String>,

    pub num_fixed_sizes: FtInt,
    pub available_sizes: Vec<FtBitmapSize>,

    pub num_charmaps: FtInt,
    pub charmaps: Vec<FtCMapRec>,

    /*# The following member variables (down to `underline_thickness`) */
    /*# are only relevant to scalable outlines; cf. @FT_Bitmap_Size    */
    /*# for bitmap fonts.                                              */
    pub bbox: FtBBox,

    pub units_per_EM: FtUShort,
    pub ascender: FtShort,
    pub descender: FtShort,
    pub height: FtShort,

    pub max_advance_width: FtShort,
    pub max_advance_height: FtShort,

    pub underline_position: FtShort,
    pub underline_thickness: FtShort,

    pub glyph: FtGlyphSlotRec,
    pub size: FtSizeRec,
    /// whether the face has its glyph slot and size (C's non-NULL
    /// `face->glyph` and `face->size`: faces opened with a negative index
    /// have neither)
    pub has_slot_and_size: bool,
    /// `charmap`: an index into `charmaps`
    pub charmap: Option<usize>,

    /*@private begin */
    /// `driver`: the module index of the face's driver
    pub driver: usize,
    pub library: Option<FtLibrary>,
    pub stream: Option<FtStream>,

    /// `autohint` (the auto-hinter's face globals)
    pub autohint: Option<Box<dyn Any + Send>>,

    pub internal: FtFaceInternalRec,
    /*@private end */
}

impl FtFaceRec {
    /// `FT_FACE_LIBRARY`
    pub fn library(&self) -> &FtLibrary {
        self.library.as_ref().expect("face without a library")
    }
    /// The face's stream (`face->stream`).
    pub fn stream(&mut self) -> &mut FtStreamRec {
        self.stream.as_mut().expect("face without a stream")
    }
    /// `face->driver->clazz`
    pub fn driver_class(&self) -> &'static FtDriverClassRec {
        self.library().modules[self.driver].driver_class()
    }
    /// The face's driver module.
    pub fn driver_module(&self) -> &FtModuleRec {
        &self.library().modules[self.driver]
    }
}

/// `FT_HAS_HORIZONTAL`
pub fn ft_has_horizontal(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_HORIZONTAL != 0
}
/// `FT_HAS_VERTICAL`
pub fn ft_has_vertical(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_VERTICAL != 0
}
/// `FT_HAS_KERNING`
pub fn ft_has_kerning(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_KERNING != 0
}
/// `FT_IS_SCALABLE`
pub fn ft_is_scalable(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_SCALABLE != 0
}
/// `FT_IS_SFNT`
pub fn ft_is_sfnt(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_SFNT != 0
}
/// `FT_IS_FIXED_WIDTH`
pub fn ft_is_fixed_width(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_FIXED_WIDTH != 0
}
/// `FT_HAS_FIXED_SIZES`
pub fn ft_has_fixed_sizes(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_FIXED_SIZES != 0
}
/// `FT_HAS_GLYPH_NAMES`
pub fn ft_has_glyph_names(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_GLYPH_NAMES != 0
}
/// `FT_HAS_MULTIPLE_MASTERS`
pub fn ft_has_multiple_masters(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_MULTIPLE_MASTERS != 0
}
/// `FT_IS_NAMED_INSTANCE`
pub fn ft_is_named_instance(face: &FtFaceRec) -> bool {
    face.face_index & 0x7FFF0000 != 0
}
/// `FT_IS_VARIATION`
pub fn ft_is_variation(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_VARIATION != 0
}
/// `FT_IS_CID_KEYED`
pub fn ft_is_cid_keyed(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_CID_KEYED != 0
}
/// `FT_IS_TRICKY`
pub fn ft_is_tricky(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_TRICKY != 0
}
/// `FT_HAS_COLOR`
pub fn ft_has_color(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_COLOR != 0
}
/// `FT_HAS_SVG`
pub fn ft_has_svg(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_SVG != 0
}
/// `FT_HAS_SBIX`
pub fn ft_has_sbix(face: &FtFaceRec) -> bool {
    face.face_flags & FT_FACE_FLAG_SBIX != 0
}

/// `FT_Face`: a driver's face record.
#[derive(Debug)]
pub enum FtFace {
    /// a `TT_Face` (the TrueType and CFF drivers')
    Tt(Box<TtFaceRec>),
    /// an `FNT_Face` (the Windows FNT driver's)
    Fnt(Box<super::super::winfonts::winfnt::FntFaceRec>),
    /// a `BDF_Face` (the BDF driver's)
    Bdf(Box<super::super::bdf::bdfdrivr::BdfFaceRec>),
}

impl Deref for FtFace {
    type Target = FtFaceRec;
    fn deref(&self) -> &FtFaceRec {
        match self {
            FtFace::Tt(f) => &f.root,
            FtFace::Fnt(f) => &f.root,
            FtFace::Bdf(f) => &f.root,
        }
    }
}

impl DerefMut for FtFace {
    fn deref_mut(&mut self) -> &mut FtFaceRec {
        match self {
            FtFace::Tt(f) => &mut f.root,
            FtFace::Fnt(f) => &mut f.root,
            FtFace::Bdf(f) => &mut f.root,
        }
    }
}

impl FtFace {
    /// The `TT_Face` of an SFNT-based face.
    pub fn tt(&self) -> Option<&TtFaceRec> {
        match self {
            FtFace::Tt(f) => Some(f),
            _ => None,
        }
    }
    /// The `TT_Face` of an SFNT-based face, mutably.
    pub fn tt_mut(&mut self) -> Option<&mut TtFaceRec> {
        match self {
            FtFace::Tt(f) => Some(f),
            _ => None,
        }
    }
}

/* the remaining macros of ftobjs.h */

/// `FT_REQUEST_WIDTH`
pub fn ft_request_width(req: &FtSizeRequestRec) -> FtLong {
    if req.horiResolution != 0 {
        (req.width.wrapping_mul(req.horiResolution as FtPos) + 36) / 72
    } else {
        req.width
    }
}

/// `FT_REQUEST_HEIGHT`
pub fn ft_request_height(req: &FtSizeRequestRec) -> FtLong {
    if req.vertResolution != 0 {
        (req.height.wrapping_mul(req.vertResolution as FtPos) + 36) / 72
    } else {
        req.height
    }
}

const GRID_FIT_METRICS: bool = true;

/*************************************************************************/
/*                                                                       */
/*                           S T R E A M                                 */
/*                                                                       */
/*************************************************************************/

/// `FT_Stream_New`: create a new input stream from an FT_Open_Args
/// structure; returns the stream and whether it is external (provided by
/// the caller)
pub fn ft_stream_new(args: &mut FtOpenArgs) -> FtResult<(FtStream, bool)> {
    let mode = args.flags & (FT_OPEN_MEMORY | FT_OPEN_STREAM | FT_OPEN_PATHNAME);

    if mode == FT_OPEN_MEMORY {
        /* create a memory-based stream */
        match args.memory_base.clone() {
            Some(base) => Ok((FtStreamRec::open_memory(base), false)),
            None => Err(FT_ERR_INVALID_ARGUMENT),
        }
    } else if mode == FT_OPEN_STREAM && args.stream.is_some() {
        /* use an existing, user-provided stream */

        /* in this case, we do not need to allocate a new stream object */
        /* since the caller is responsible for closing it himself       */
        Ok((args.stream.take().unwrap(), true))
    } else {
        /* (paths aren't supported: SDL_ttf opens streams) */
        if args.flags & FT_OPEN_STREAM != 0 {
            /* FT_Stream_Close */
            args.stream = None;
        }
        Err(FT_ERR_INVALID_ARGUMENT)
    }
}

/*************************************************************************/
/*                                                                       */
/*               FACE, SIZE & GLYPH SLOT OBJECTS                         */
/*                                                                       */
/*************************************************************************/

/// `ft_glyphslot_init`
fn ft_glyphslot_init(
    slot: &mut FtGlyphSlotRec,
    library: &FtLibrary,
    driver: usize,
    face_flags: FtLong,
) -> FtResult<()> {
    let module = &library.modules[driver];
    let clazz = module.driver_class();

    slot.library = Some(library.clone());
    slot.driver = Some(driver);
    slot.face_flags = face_flags;

    slot.internal = FtSlotInternalRec::default();

    if module.clazz.root().module_flags & FT_MODULE_DRIVER_NO_OUTLINES == 0 {
        slot.internal.loader = Some(FtGlyphLoaderRec::new());
    }

    if let Some(init_slot) = clazz.init_slot {
        init_slot(slot)?;
    }

    /* if SVG table exists, allocate the space in `slot->other` */
    if face_flags & FT_FACE_FLAG_SVG != 0 {
        slot.other = Some(Box::default());
    }

    Ok(())
}

/// `ft_glyphslot_free_bitmap`
pub fn ft_glyphslot_free_bitmap(slot: &mut FtGlyphSlotRec) {
    if slot.internal.flags & FT_GLYPH_OWN_BITMAP != 0 {
        slot.bitmap.buffer = Vec::new();
        slot.internal.flags &= !FT_GLYPH_OWN_BITMAP;
    } else {
        /* assume that the bitmap buffer was stolen or not */
        /* allocated from the heap                         */
        slot.bitmap.buffer = Vec::new();
    }
}

/// `ft_glyphslot_preset_bitmap`: overflow-resistant presetting of bitmap
/// position and dimensions; also check whether the size is too large for
/// rendering
pub fn ft_glyphslot_preset_bitmap(
    slot: &mut FtGlyphSlotRec,
    mode: FtRenderMode,
    origin: Option<&FtVector>,
) -> bool {
    let pixel_mode: FtPixelMode;

    let mut x_shift: FtPos = 0;
    let mut y_shift: FtPos = 0;

    if slot.format == FT_GLYPH_FORMAT_SVG {
        /* the `ot-svg` module isn't translated (part 2): SVG glyphs */
        /* can't be rendered                                         */
        return true;
    } else if slot.format != FT_GLYPH_FORMAT_OUTLINE {
        return true;
    }

    if let Some(origin) = origin {
        x_shift = origin.x;
        y_shift = origin.y;
    }

    /* compute the control box, and grid-fit it, */
    /* taking into account the origin shift      */
    let mut cbox = ft_outline_get_cbox(&slot.outline);

    /* rough estimate of pixel box */
    let mut pbox = FtBBox {
        xMin: (cbox.xMin >> 6).wrapping_add(x_shift >> 6),
        yMin: (cbox.yMin >> 6).wrapping_add(y_shift >> 6),
        xMax: (cbox.xMax >> 6).wrapping_add(x_shift >> 6),
        yMax: (cbox.yMax >> 6).wrapping_add(y_shift >> 6),
    };

    /* tiny remainder box */
    cbox.xMin = (cbox.xMin & 63) + (x_shift & 63);
    cbox.yMin = (cbox.yMin & 63) + (y_shift & 63);
    cbox.xMax = (cbox.xMax & 63) + (x_shift & 63);
    cbox.yMax = (cbox.yMax & 63) + (y_shift & 63);

    let mut adjust = false;
    match mode {
        FT_RENDER_MODE_MONO => {
            pixel_mode = FT_PIXEL_MODE_MONO;
            /* x */

            /* undocumented but confirmed: bbox values get rounded;    */
            /* we do asymmetric rounding so that the center of a pixel */
            /* gets always included                                    */

            pbox.xMin += (cbox.xMin + 31) >> 6;
            pbox.xMax += (cbox.xMax + 32) >> 6;

            /* if the bbox collapsed, we add a pixel based on the total */
            /* rounding remainder to cover most of the original cbox    */

            if pbox.xMin == pbox.xMax {
                if ((cbox.xMin + 31) & 63) - 31 + ((cbox.xMax + 32) & 63) - 32 < 0 {
                    pbox.xMin -= 1;
                } else {
                    pbox.xMax += 1;
                }
            }

            /* y */

            pbox.yMin += (cbox.yMin + 31) >> 6;
            pbox.yMax += (cbox.yMax + 32) >> 6;

            if pbox.yMin == pbox.yMax {
                if ((cbox.yMin + 31) & 63) - 31 + ((cbox.yMax + 32) & 63) - 32 < 0 {
                    pbox.yMin -= 1;
                } else {
                    pbox.yMax += 1;
                }
            }
        }

        FT_RENDER_MODE_LCD => {
            pixel_mode = FT_PIXEL_MODE_LCD;
            ft_lcd_padding(&mut cbox, slot, mode);
            adjust = true;
        }

        FT_RENDER_MODE_LCD_V => {
            pixel_mode = FT_PIXEL_MODE_LCD_V;
            ft_lcd_padding(&mut cbox, slot, mode);
            adjust = true;
        }

        _ => {
            /* FT_RENDER_MODE_NORMAL, FT_RENDER_MODE_LIGHT */
            pixel_mode = FT_PIXEL_MODE_GRAY;
            adjust = true;
        }
    }
    if adjust {
        /* Adjust: */
        pbox.xMin += cbox.xMin >> 6;
        pbox.yMin += cbox.yMin >> 6;
        pbox.xMax += (cbox.xMax + 63) >> 6;
        pbox.yMax += (cbox.yMax + 63) >> 6;
    }

    let x_left = pbox.xMin;
    let y_top = pbox.yMax;

    let mut width = pbox.xMax.wrapping_sub(pbox.xMin);
    let mut height = pbox.yMax.wrapping_sub(pbox.yMin);

    let pitch = match pixel_mode {
        FT_PIXEL_MODE_MONO => ((width + 15) >> 4) << 1,

        FT_PIXEL_MODE_LCD => {
            width = width.wrapping_mul(3);
            ft_pad_ceil(width, 4)
        }

        FT_PIXEL_MODE_LCD_V => {
            height = height.wrapping_mul(3);
            width
        }

        _ => width,
    };

    slot.bitmap_left = x_left as FtInt;
    slot.bitmap_top = y_top as FtInt;

    let bitmap = &mut slot.bitmap;
    bitmap.pixel_mode = pixel_mode;
    bitmap.num_grays = 256;
    bitmap.width = width as u32;
    bitmap.rows = height as u32;
    bitmap.pitch = pitch as i32;

    if pbox.xMin < -0x8000 || pbox.xMax > 0x7FFF || pbox.yMin < -0x8000 || pbox.yMax > 0x7FFF {
        return true;
    }

    false
}

/// `ft_glyphslot_set_bitmap`
pub fn ft_glyphslot_set_bitmap(slot: &mut FtGlyphSlotRec, buffer: Vec<u8>) {
    ft_glyphslot_free_bitmap(slot);

    slot.bitmap.buffer = buffer;
}

/// `ft_glyphslot_alloc_bitmap`
pub fn ft_glyphslot_alloc_bitmap(slot: &mut FtGlyphSlotRec, size: FtULong) -> FtResult<()> {
    if slot.internal.flags & FT_GLYPH_OWN_BITMAP != 0 {
        slot.bitmap.buffer = Vec::new();
    } else {
        slot.internal.flags |= FT_GLYPH_OWN_BITMAP;
    }

    slot.bitmap.buffer = ft_qalloc(size as FtLong)?;
    Ok(())
}

/// `ft_glyphslot_clear`
fn ft_glyphslot_clear(slot: &mut FtGlyphSlotRec) {
    /* free bitmap if needed */
    ft_glyphslot_free_bitmap(slot);

    /* clear all public fields in the glyph slot */
    slot.glyph_index = 0;

    slot.metrics = FtGlyphMetrics::default();
    slot.outline = FtOutline::default();

    slot.bitmap.width = 0;
    slot.bitmap.rows = 0;
    slot.bitmap.pitch = 0;
    slot.bitmap.pixel_mode = 0;
    /* `slot->bitmap.buffer' has been handled by ft_glyphslot_free_bitmap */

    slot.bitmap_left = 0;
    slot.bitmap_top = 0;
    slot.num_subglyphs = 0;
    slot.subglyphs = Vec::new();
    slot.control_data = Vec::new();
    slot.control_len = 0;

    if slot.face_flags & FT_FACE_FLAG_SVG == 0 {
        slot.other = None;
    } else if slot.internal.flags & FT_GLYPH_OWN_GZIP_SVG != 0 {
        if let Some(doc) = slot.other.as_mut() {
            doc.svg_document = Vec::new();
        }
        slot.internal.flags &= !FT_GLYPH_OWN_GZIP_SVG;
    }

    slot.format = FT_GLYPH_FORMAT_NONE;

    slot.linearHoriAdvance = 0;
    slot.linearVertAdvance = 0;
    slot.advance.x = 0;
    slot.advance.y = 0;
    slot.lsb_delta = 0;
    slot.rsb_delta = 0;
}

/// `ft_glyphslot_done`
fn ft_glyphslot_done(slot: &mut FtGlyphSlotRec) {
    if let (Some(library), Some(driver)) = (slot.library.clone(), slot.driver) {
        let clazz = library.modules[driver].driver_class();
        if let Some(done_slot) = clazz.done_slot {
            done_slot(slot);
        }
    }

    /* free bitmap buffer if needed */
    ft_glyphslot_free_bitmap(slot);

    slot.other = None;
    slot.internal = FtSlotInternalRec::default();
}

/// `FT_New_GlyphSlot`: a new glyph slot for `face` (made the face's slot
/// by the caller, which keeps the previous one)
pub fn ft_new_glyph_slot(face: &FtFaceRec) -> FtResult<FtGlyphSlotRec> {
    let library = face.library().clone();
    let mut slot = FtGlyphSlotRec::default();

    if let Err(e) = ft_glyphslot_init(&mut slot, &library, face.driver, face.face_flags) {
        ft_glyphslot_done(&mut slot);
        return Err(e);
    }

    Ok(slot)
}

/// `FT_Done_GlyphSlot`
pub fn ft_done_glyph_slot(mut slot: FtGlyphSlotRec) {
    ft_glyphslot_done(&mut slot);
}

/// `FT_Set_Transform`
pub fn ft_set_transform(face: &mut FtFaceRec, matrix: Option<&FtMatrix>, delta: Option<&FtVector>) {
    let internal = &mut face.internal;

    internal.transform_flags = 0;

    let matrix = match matrix {
        None => {
            internal.transform_matrix.xx = 0x10000;
            internal.transform_matrix.xy = 0;
            internal.transform_matrix.yx = 0;
            internal.transform_matrix.yy = 0x10000;

            internal.transform_matrix
        }
        Some(m) => {
            internal.transform_matrix = *m;
            *m
        }
    };

    /* set transform_flags bit flag 0 if `matrix' isn't the identity */
    if (matrix.xy | matrix.yx) != 0 || matrix.xx != 0x10000 || matrix.yy != 0x10000 {
        internal.transform_flags |= 1;
    }

    let delta = match delta {
        None => {
            internal.transform_delta.x = 0;
            internal.transform_delta.y = 0;

            internal.transform_delta
        }
        Some(d) => {
            internal.transform_delta = *d;
            *d
        }
    };

    /* set transform_flags bit flag 1 if `delta' isn't the null vector */
    if (delta.x | delta.y) != 0 {
        internal.transform_flags |= 2;
    }
}

/// `FT_Get_Transform`
pub fn ft_get_transform(face: &FtFaceRec) -> (FtMatrix, FtVector) {
    (
        face.internal.transform_matrix,
        face.internal.transform_delta,
    )
}

/// `ft_glyphslot_grid_fit_metrics`
fn ft_glyphslot_grid_fit_metrics(slot: &mut FtGlyphSlotRec, vertical: bool) {
    let metrics = &mut slot.metrics;
    let right: FtPos;
    let bottom: FtPos;

    if vertical {
        metrics.horiBearingX = ft_pix_floor(metrics.horiBearingX);
        metrics.horiBearingY = ft_pix_ceil_long(metrics.horiBearingY);

        right = ft_pix_ceil_long(add_long(metrics.vertBearingX, metrics.width));
        bottom = ft_pix_ceil_long(add_long(metrics.vertBearingY, metrics.height));

        metrics.vertBearingX = ft_pix_floor(metrics.vertBearingX);
        metrics.vertBearingY = ft_pix_floor(metrics.vertBearingY);

        metrics.width = sub_long(right, metrics.vertBearingX);
        metrics.height = sub_long(bottom, metrics.vertBearingY);
    } else {
        metrics.vertBearingX = ft_pix_floor(metrics.vertBearingX);
        metrics.vertBearingY = ft_pix_floor(metrics.vertBearingY);

        right = ft_pix_ceil_long(add_long(metrics.horiBearingX, metrics.width));
        bottom = ft_pix_floor(sub_long(metrics.horiBearingY, metrics.height));

        metrics.horiBearingX = ft_pix_floor(metrics.horiBearingX);
        metrics.horiBearingY = ft_pix_ceil_long(metrics.horiBearingY);

        metrics.width = sub_long(right, metrics.horiBearingX);
        metrics.height = sub_long(metrics.horiBearingY, bottom);
    }

    metrics.horiAdvance = ft_pix_round_long(metrics.horiAdvance);
    metrics.vertAdvance = ft_pix_round_long(metrics.vertAdvance);
}

/// `FT_Load_Glyph`
pub fn ft_load_glyph(
    face: &mut FtFace,
    glyph_index: FtUInt,
    mut load_flags: FtInt32,
) -> FtResult<()> {
    let mut error: FtResult<()>;
    let mut autohint = false;

    if !face.has_slot_and_size {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    }

    /* The validity test for `glyph_index' is performed by the */
    /* font drivers.                                           */

    ft_glyphslot_clear(&mut face.glyph);

    let library = face.library().clone();
    let driver_module = &library.modules[face.driver];
    let driver = driver_module.driver_class();
    let hinter = library.auto_hinter;

    /* undefined scale means no scale */
    if face.size.metrics.x_ppem == 0 || face.size.metrics.y_ppem == 0 {
        load_flags |= FT_LOAD_NO_SCALE;
    }

    /* resolve load flags dependencies */

    if load_flags & FT_LOAD_NO_RECURSE != 0 {
        load_flags |= FT_LOAD_NO_SCALE | FT_LOAD_IGNORE_TRANSFORM;
    }

    if load_flags & FT_LOAD_NO_SCALE != 0 {
        load_flags |= FT_LOAD_NO_HINTING | FT_LOAD_NO_BITMAP;

        load_flags &= !FT_LOAD_RENDER;
    }

    if load_flags & FT_LOAD_BITMAP_METRICS_ONLY != 0 {
        load_flags &= !FT_LOAD_RENDER;
    }

    /*
     * Determine whether we need to auto-hint or not.
     * The general rules are:
     *
     * - Do only auto-hinting if we have
     *
     *   - a hinter module,
     *   - a scalable font,
     *   - not a tricky font, and
     *   - no transforms except simple slants and/or rotations by
     *     integer multiples of 90 degrees.
     *
     * - Then, auto-hint if FT_LOAD_FORCE_AUTOHINT is set or if we don't
     *   have a native font hinter.
     *
     * - Otherwise, auto-hint for LIGHT hinting mode or if there isn't
     *   any hinting bytecode in the TrueType/OpenType font.
     *
     * - Exception: The font is `tricky' and requires the native hinter to
     *   load properly.
     */

    if hinter.is_some()
        && load_flags & FT_LOAD_NO_HINTING == 0
        && load_flags & FT_LOAD_NO_AUTOHINT == 0
        && ft_is_scalable(face)
        && !ft_is_tricky(face)
        && (load_flags & FT_LOAD_IGNORE_TRANSFORM != 0
            || (face.internal.transform_matrix.yx == 0 && face.internal.transform_matrix.xx != 0)
            || (face.internal.transform_matrix.xx == 0 && face.internal.transform_matrix.yx != 0))
    {
        let driver_flags = driver.root.module_flags;
        if load_flags & FT_LOAD_FORCE_AUTOHINT != 0
            || driver_flags & FT_MODULE_DRIVER_HAS_HINTER == 0
        {
            autohint = true;
        } else {
            let mode = ft_load_target_mode(load_flags);

            /* only the new Adobe engine (for both CFF and Type 1) is `light'; */
            /* we use `strstr' to catch both `Type 1' and `CID Type 1'         */
            /* (no Type 1 driver is translated)                                */
            let is_light_type1 = false;

            /* the check for `num_locations' assures that we actually    */
            /* test for instructions in a TTF and not in a CFF-based OTF */
            /*                                                           */
            /* since `maxSizeOfInstructions' might be unreliable, we     */
            /* check the size of the `fpgm' and `prep' tables, too --    */
            /* the assumption is that there don't exist real TTFs where  */
            /* both `fpgm' and `prep' tables are missing                 */
            let no_bytecode = match face.tt() {
                Some(ttface) => {
                    ft_is_sfnt(face)
                        && ttface.num_locations != 0
                        && ttface.max_profile.maxSizeOfInstructions == 0
                        && ttface.font_program_size == 0
                        && ttface.cvt_program_size == 0
                }
                None => false,
            };
            if (mode == FT_RENDER_MODE_LIGHT
                && (driver_flags & FT_MODULE_DRIVER_HINTS_LIGHTLY == 0 && !is_light_type1))
                || no_bytecode
            {
                autohint = true;
            }
        }
    }

    let load_ok;
    if autohint {
        let mut done = false;

        /* XXX: The use of the `FT_LOAD_XXX_ONLY` flags is not very */
        /*      elegant.                                            */

        /* try to load SVG documents if available */
        if load_flags & FT_LOAD_NO_SVG == 0 && ft_has_svg(face) {
            error = (driver.load_glyph.unwrap())(face, glyph_index, load_flags | FT_LOAD_SVG_ONLY);

            if error.is_ok() && face.glyph.format == FT_GLYPH_FORMAT_SVG {
                done = true;
            }
        }

        /* try to load embedded bitmaps if available */
        if !done && ft_has_fixed_sizes(face) && load_flags & FT_LOAD_NO_BITMAP == 0 {
            error =
                (driver.load_glyph.unwrap())(face, glyph_index, load_flags | FT_LOAD_SBITS_ONLY);

            if error.is_ok() && face.glyph.format == FT_GLYPH_FORMAT_BITMAP {
                done = true;
            }
        }

        if done {
            load_ok = Ok(());
        } else {
            let transform_flags = face.internal.transform_flags;

            /* since the auto-hinter calls FT_Load_Glyph by itself, */
            /* make sure that glyphs aren't transformed             */
            face.internal.transform_flags = 0;

            /* load auto-hinted outline */
            let hinter = hinter.unwrap();
            error = match library.modules[hinter].clazz.root().module_interface {
                FtModuleInterface::AutoHinter(hinting) => {
                    (hinting.load_glyph)(&library, hinter, face, glyph_index, load_flags)
                }
                _ => Err(FT_ERR_UNIMPLEMENTED_FEATURE),
            };

            face.internal.transform_flags = transform_flags;
            load_ok = error;
        }
    } else {
        error = (driver.load_glyph.unwrap())(face, glyph_index, load_flags);
        error?;

        if face.glyph.format == FT_GLYPH_FORMAT_OUTLINE {
            /* check that the loaded outline is correct */
            ft_outline_check(&face.glyph.outline)?;

            if GRID_FIT_METRICS && load_flags & FT_LOAD_NO_HINTING == 0 {
                ft_glyphslot_grid_fit_metrics(
                    &mut face.glyph,
                    load_flags & FT_LOAD_VERTICAL_LAYOUT != 0,
                );
            }
        }
        load_ok = Ok(());
    }

    let mut error = load_ok;

    /* Load_Ok: */
    /* compute the advance */
    if load_flags & FT_LOAD_VERTICAL_LAYOUT != 0 {
        face.glyph.advance.x = 0;
        face.glyph.advance.y = face.glyph.metrics.vertAdvance;
    } else {
        face.glyph.advance.x = face.glyph.metrics.horiAdvance;
        face.glyph.advance.y = 0;
    }

    /* compute the linear advance in 16.16 pixels */
    if load_flags & FT_LOAD_LINEAR_DESIGN == 0 && ft_is_scalable(face) {
        let metrics = face.size.metrics;

        /* it's tricky! */
        face.glyph.linearHoriAdvance =
            ft_mul_div(face.glyph.linearHoriAdvance, metrics.x_scale, 64);

        face.glyph.linearVertAdvance =
            ft_mul_div(face.glyph.linearVertAdvance, metrics.y_scale, 64);
    }

    if load_flags & FT_LOAD_IGNORE_TRANSFORM == 0 {
        let internal = face.internal;

        /* now, transform the glyph image if needed */
        if internal.transform_flags != 0 {
            /* get renderer */
            let renderer = ft_lookup_glyph_renderer(&library, &face.glyph);

            if let Some(renderer) = renderer {
                error = (library.modules[renderer]
                    .renderer_class()
                    .transform_glyph
                    .unwrap())(
                    &mut face.glyph,
                    Some(&internal.transform_matrix),
                    Some(&internal.transform_delta),
                );
            } else if face.glyph.format == FT_GLYPH_FORMAT_OUTLINE {
                /* apply `standard' transformation if no renderer is available */
                if internal.transform_flags & 1 != 0 {
                    ft_outline_transform(&mut face.glyph.outline, &internal.transform_matrix);
                }

                if internal.transform_flags & 2 != 0 {
                    ft_outline_translate(
                        &mut face.glyph.outline,
                        internal.transform_delta.x,
                        internal.transform_delta.y,
                    );
                }
            }

            /* transform advance */
            ft_vector_transform(&mut face.glyph.advance, &internal.transform_matrix);
        }
    }

    face.glyph.glyph_index = glyph_index;
    face.glyph.internal.load_flags = load_flags;

    /* do we need to render the image or preset the bitmap now? */
    if error.is_ok()
        && load_flags & FT_LOAD_NO_SCALE == 0
        && face.glyph.format != FT_GLYPH_FORMAT_BITMAP
        && face.glyph.format != FT_GLYPH_FORMAT_COMPOSITE
    {
        let mut mode = ft_load_target_mode(load_flags);

        if mode == FT_RENDER_MODE_NORMAL && load_flags & FT_LOAD_MONOCHROME != 0 {
            mode = FT_RENDER_MODE_MONO;
        }

        if load_flags & FT_LOAD_RENDER != 0 {
            error = ft_render_glyph(face, mode);
        } else {
            ft_glyphslot_preset_bitmap(&mut face.glyph, mode, None);
        }
    }

    error
}

/// `FT_Load_Char`
pub fn ft_load_char(face: &mut FtFace, char_code: FtULong, load_flags: FtInt32) -> FtResult<()> {
    let mut glyph_index = char_code as FtUInt;
    if face.charmap.is_some() {
        glyph_index = ft_get_char_index(face, char_code);
    }

    ft_load_glyph(face, glyph_index, load_flags)
}

/// `destroy_charmaps`
fn destroy_charmaps(face: &mut FtFaceRec) {
    face.charmaps = Vec::new();
    face.num_charmaps = 0;
}

/// `destroy_face`
fn destroy_face(face: &mut FtFace) {
    /* discard auto-hinting data */
    face.autohint = None;

    /* Discard glyph slots for this face.                           */
    if face.has_slot_and_size {
        let mut slot = std::mem::take(&mut face.glyph);
        ft_glyphslot_done(&mut slot);

        /* discard all sizes for this face */
        let clazz = face.driver_class();
        if let Some(done_size) = clazz.done_size {
            done_size(face);
        }
        face.has_slot_and_size = false;
    }

    /* discard charmaps */
    destroy_charmaps(face);

    /* finalize format-specific stuff */
    let clazz = face.driver_class();
    if let Some(done_face) = clazz.done_face {
        done_face(face);
    }

    /* close the stream for this face if needed */
    face.stream = None;
}

/// `find_unicode_charmap`: This function finds a Unicode charmap, if there
/// is one.  And if there is more than one, it tries to favour the more
/// extensive one, i.e., one that supports UCS-4 against those which are
/// limited to the BMP (said UCS-2 encoding.)
///
/// This function is called from open_face() (just below), and also from
/// FT_Select_Charmap( ..., FT_ENCODING_UNICODE ).
fn find_unicode_charmap(face: &mut FtFaceRec) -> FtResult<()> {
    if face.charmaps.is_empty() {
        return Err(FT_ERR_INVALID_CHARMAP_HANDLE);
    }

    /*
     * The original TrueType specification(s) only specified charmap
     * formats that are capable of mapping 8 or 16 bit character codes to
     * glyph indices.
     *
     * However, recent updates to the Apple and OpenType specifications
     * introduced new formats that are capable of mapping 32-bit character
     * codes as well.  And these are already used on some fonts, mainly to
     * map non-BMP Asian ideographs as defined in Unicode.
     *
     * For compatibility purposes, these fonts generally come with
     * *several* Unicode charmaps:
     *
     * - One of them in the "old" 16-bit format, that cannot access
     *   all glyphs in the font.
     *
     * - Another one in the "new" 32-bit format, that can access all
     *   the glyphs.
     *
     * This function has been written to always favor a 32-bit charmap
     * when found.  Otherwise, a 16-bit one is returned when found.
     */

    /* Since the `interesting' table, with IDs (3,10), is normally the */
    /* last one, we loop backwards.  This loses with type1 fonts with  */
    /* non-BMP characters (<.0001%), this wins with .ttf with non-BMP  */
    /* chars (.01% ?), and this is the same about 99.99% of the time!  */

    let n = face.num_charmaps.max(0) as usize;
    for cur in (0..n).rev() {
        let cm = face.charmaps[cur].charmap;
        if cm.encoding == FT_ENCODING_UNICODE {
            /* XXX If some new encodings to represent UCS-4 are added, */
            /*     they should be added here.                          */
            if (cm.platform_id == TT_PLATFORM_MICROSOFT && cm.encoding_id == TT_MS_ID_UCS_4)
                || (cm.platform_id == TT_PLATFORM_APPLE_UNICODE
                    && cm.encoding_id == TT_APPLE_ID_UNICODE_32)
            {
                face.charmap = Some(cur);
                return Ok(());
            }
        }
    }

    /* We do not have any UCS-4 charmap.                */
    /* Do the loop again and search for UCS-2 charmaps. */
    for cur in (0..n).rev() {
        if face.charmaps[cur].charmap.encoding == FT_ENCODING_UNICODE {
            face.charmap = Some(cur);
            return Ok(());
        }
    }

    Err(FT_ERR_INVALID_CHARMAP_HANDLE)
}

/// `find_variant_selector_charmap`: This function finds the variant
/// selector charmap, if there is one. There can only be one (platform=0,
/// specific=5, format=14).
fn find_variant_selector_charmap(face: &FtFace) -> Option<usize> {
    for cur in 0..face.num_charmaps.max(0) as usize {
        let cm = face.charmaps[cur].charmap;
        if cm.platform_id == TT_PLATFORM_APPLE_UNICODE
            && cm.encoding_id == TT_APPLE_ID_VARIANT_SELECTOR
            && ft_get_cmap_format(face, cur) == 14
        {
            return Some(cur);
        }
    }

    None
}

/// `open_face`: This function does some work for FT_Open_Face().
///
/// On failure, the stream comes back with the error (C's `*astream`).
fn open_face(
    library: &FtLibrary,
    driver: usize,
    stream: FtStream,
    external_stream: bool,
    face_index: FtLong,
    params: &[FtParameter],
) -> Result<FtFace, (FtError, Option<FtStream>, bool)> {
    let clazz = library.modules[driver].driver_class();

    /* allocate the face object and perform basic initialization */
    let mut root = FtFaceRec {
        driver,
        library: Some(library.clone()),
        stream: Some(stream),
        ..Default::default()
    };

    /* set the FT_FACE_FLAG_EXTERNAL_STREAM bit for FT_Done_Face */
    if external_stream {
        root.face_flags |= FT_FACE_FLAG_EXTERNAL_STREAM;
    }

    root.internal.random_seed = -1;

    let mut face = (clazz.new_face)(root);

    let mut error = match clazz.init_face {
        Some(init_face) => init_face(&mut face, face_index as FtInt, params),
        None => Ok(()),
    };

    /* Stream may have been changed. */
    let external_stream = face.face_flags & FT_FACE_FLAG_EXTERNAL_STREAM != 0;

    if error.is_ok() {
        /* select Unicode charmap by default */
        let error2 = find_unicode_charmap(&mut face);

        /* if no Unicode charmap can be found, FT_Err_Invalid_CharMap_Handle */
        /* is returned.                                                      */

        /* no error should happen, but we want to play safe */
        if let Err(e2) = error2 {
            if ft_err_neq(e2, FT_ERR_INVALID_CHARMAP_HANDLE) {
                error = Err(e2);
            }
        }
    }

    match error {
        Ok(()) => Ok(face),
        Err(e) => {
            destroy_charmaps(&mut face);
            if let Some(done_face) = clazz.done_face {
                done_face(&mut face);
            }
            let stream = face.stream.take();
            Err((e, stream, external_stream))
        }
    }
}

/// `FT_New_Memory_Face`
pub fn ft_new_memory_face(
    library: &FtLibrary,
    file_base: Arc<[u8]>,
    face_index: FtLong,
) -> FtResult<FtFace> {
    let args = FtOpenArgs {
        flags: FT_OPEN_MEMORY,
        memory_base: Some(file_base),
        ..Default::default()
    };

    ft_open_face_internal(library, args, face_index, true)
}

/* The behavior here is very similar to that in base/ftmac.c, but it     */
/* is designed to work on non-mac systems, so no mac specific calls.     */
/*                                                                       */
/* We look at the file and determine if it is a mac dfont file or a mac  */
/* resource file, or a macbinary file containing a mac resource file.    */
/*                                                                       */
/* Unlike ftmac I'm not going to look at a `FOND'.  I don't really see   */
/* the point, especially since there may be multiple `FOND' resources.   */
/* Instead I'll just look for `sfnt' and `POST' resources, ordered as    */
/* they occur in the file.                                               */
/*                                                                       */
/* Note that multiple `POST' resources do not mean multiple postscript   */
/* fonts; they all get jammed together to make what is essentially a     */
/* pfb file.                                                             */
/*                                                                       */
/* We aren't interested in `NFNT' or `FONT' bitmap resources.            */
/*                                                                       */
/* As soon as we get an `sfnt' load it into memory and pass it off to    */
/* FT_Open_Face.                                                         */
/*                                                                       */
/* If we have a (set of) `POST' resources, massage them into a (memory)  */
/* pfb file and pass that to FT_Open_Face.  (As with ftmac.c I'm not     */
/* going to try to save the kerning info.  After all that lives in the   */
/* `FOND' which isn't in the file containing the `POST' resources so     */
/* we don't really have access to it.                                    */

/// `open_face_from_buffer`: Create a new FT_Face given a buffer and a
/// driver name. From `ftmac.c'.
fn open_face_from_buffer(
    library: &FtLibrary,
    base: Vec<u8>,
    face_index: FtLong,
    driver_name: Option<&str>,
) -> FtResult<FtFace> {
    let mut args = FtOpenArgs::default();

    if let Some(driver_name) = driver_name {
        match library.get_module(driver_name) {
            Some(d) => args.driver = Some(d),
            None => return Err(FT_ERR_MISSING_MODULE),
        }

        args.flags |= FT_OPEN_DRIVER;
    }

    /* `memory_stream_close` also frees the stream object. */
    args.stream = Some(FtStreamRec::open_memory(Arc::from(base)));

    args.flags |= FT_OPEN_STREAM;

    ft_open_face_internal(library, args, face_index, false)
}

/// `ft_lookup_PS_in_sfnt_stream`: Look up `TYP1' or `CID ' table from sfnt
/// table directory. `offset' and `length' must exclude the binary header
/// in tables.
fn ft_lookup_ps_in_sfnt_stream(
    stream: &mut FtStreamRec,
    face_index: FtLong,
    offset: &mut FtULong,
    length: &mut FtULong,
    is_sfnt_cid: &mut bool,
) -> FtResult<()> {
    /* Type 1 and CID-keyed font drivers should recognize sfnt-wrapped */
    /* format too.  Here, since we can't expect that the TrueType font */
    /* driver is loaded unconditionally, we must parse the font by     */
    /* ourselves.  We are only interested in the name of the table and */
    /* the offset.                                                     */

    *offset = 0;
    *length = 0;
    *is_sfnt_cid = false;

    /* TODO: support for sfnt-wrapped PS/CID in TTC format */

    /* version check for 'typ1' (should be ignored?) */
    let tag = stream.read_ulong()? as FtULong;
    if tag != TTAG_typ1 as FtULong {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    let num_tables = stream.read_ushort()?;
    stream.skip(2 * 3)?; /* skip binary search header */

    let mut pstable_index: FtLong = -1;
    *is_sfnt_cid = false;

    for _ in 0..num_tables {
        let tag = stream.read_ulong()? as FtULong;
        stream.skip(4)?;
        *offset = stream.read_ulong()? as FtULong;
        *length = stream.read_ulong()? as FtULong;

        if tag == TTAG_CID as FtULong {
            pstable_index += 1;
            *offset = offset.wrapping_add(22);
            *length = length.wrapping_sub(22);
            *is_sfnt_cid = true;
            if face_index < 0 {
                return Ok(());
            }
        } else if tag == TTAG_TYP1 as FtULong {
            pstable_index += 1;
            *offset = offset.wrapping_add(24);
            *length = length.wrapping_sub(24);
            *is_sfnt_cid = false;
            if face_index < 0 {
                return Ok(());
            }
        }
        if face_index >= 0 && pstable_index == face_index {
            return Ok(());
        }
    }

    Err(FT_ERR_TABLE_MISSING)
}

/// `open_face_PS_from_sfnt_stream`
fn open_face_ps_from_sfnt_stream(
    library: &FtLibrary,
    stream: &mut FtStreamRec,
    mut face_index: FtLong,
) -> FtResult<FtFace> {
    let mut offset: FtULong = 0;
    let mut length: FtULong = 0;
    let mut is_sfnt_cid = false;

    /* ignore GX stuff */
    if face_index > 0 {
        face_index &= 0xFFFF;
    }

    let pos = stream.pos();

    let error = (|| -> FtResult<FtFace> {
        ft_lookup_ps_in_sfnt_stream(
            stream,
            face_index,
            &mut offset,
            &mut length,
            &mut is_sfnt_cid,
        )?;

        if offset > stream.size {
            return Err(FT_ERR_INVALID_TABLE);
        } else if length > stream.size - offset {
            return Err(FT_ERR_INVALID_TABLE);
        }

        stream.seek(pos + offset)?;

        let mut sfnt_ps = ft_qalloc(length as FtLong)?;

        stream.read(&mut sfnt_ps)?;

        open_face_from_buffer(
            library,
            sfnt_ps,
            face_index.min(0),
            Some(if is_sfnt_cid { "t1cid" } else { "type1" }),
        )
    })();

    /* Exit: */
    match error {
        Err(e) if ft_err_eq(e, FT_ERR_UNKNOWN_FILE_FORMAT) => {
            stream.seek(pos)?;
            Err(e)
        }
        other => other,
    }
}

/// `Mac_Read_POST_Resource`: The resource header says we've got
/// resource_cnt `POST' (type1) resources in this file.  They all need to
/// be coalesced into one lump which gets passed on to the type1 driver.
/// Here can be only one PostScript font in a file so face_index must be 0
/// (or -1).
fn mac_read_post_resource(
    library: &FtLibrary,
    stream: &mut FtStreamRec,
    offsets: &[FtLong],
    mut face_index: FtLong,
) -> FtResult<FtFace> {
    let resource_cnt = offsets.len();
    let mut error: FtError = FT_ERR_OK;

    if face_index == -1 {
        face_index = 0;
    }
    if face_index != 0 {
        return Err(FT_ERR_CANNOT_OPEN_RESOURCE);
    }

    /* Find the length of all the POST resources, concatenated.  Assume */
    /* worst case (each resource in its own section).                   */
    let mut pfb_len: FtULong = 0;
    for &off in offsets {
        stream.seek(off as FtULong)?;
        let temp = stream.read_ulong()? as FtULong; /* actually LONG */

        /* FT2 allocator takes signed long buffer length,
         * too large value causing overflow should be checked
         */
        if FT_MAC_RFORK_MAX_LEN < temp || FT_MAC_RFORK_MAX_LEN - temp < pfb_len + 6 {
            return Err(FT_ERR_INVALID_OFFSET);
        }

        pfb_len += temp + 6;
    }

    if pfb_len + 2 < 6 {
        return Err(FT_ERR_ARRAY_TOO_LARGE);
    }

    let mut pfb_data = ft_qalloc((pfb_len + 2) as FtLong)?;

    pfb_data[0] = 0x80;
    pfb_data[1] = 1; /* Ascii section */
    pfb_data[2] = 0; /* 4-byte length, fill in later */
    pfb_data[3] = 0;
    pfb_data[4] = 0;
    pfb_data[5] = 0;
    let mut pfb_pos: FtULong = 6;
    let mut pfb_lenpos: FtULong = 2;

    let mut len: FtULong = 0;
    let mut type_ = 1;

    let res = (|| -> FtResult<()> {
        for i in 0..resource_cnt {
            stream.seek(offsets[i] as FtULong)?;
            let mut rlen = stream.read_ulong()? as FtULong;

            /* FT2 allocator takes signed long buffer length,
             * too large fragment length causing overflow should be checked
             */
            if 0x7FFFFFFF < rlen {
                return Err(FT_ERR_INVALID_OFFSET);
            }

            let flags = stream.read_ushort()? as i32;

            error = FT_ERR_ARRAY_TOO_LARGE;

            /* postpone the check of `rlen longer than buffer' */
            /* until `FT_Stream_Read'                          */

            if (flags >> 8) == 0 {
                /* Comment, should not be loaded */
                continue;
            }

            /* the flags are part of the resource, so rlen >= 2,  */
            /* but some fonts declare rlen = 0 for empty fragment */
            if rlen > 2 {
                rlen -= 2;
            } else {
                rlen = 0;
            }

            if (flags >> 8) == type_ {
                len += rlen;
            } else {
                if pfb_lenpos + 3 > pfb_len + 2 {
                    return Err(error);
                }

                let p = pfb_lenpos as usize;
                pfb_data[p] = len as u8;
                pfb_data[p + 1] = (len >> 8) as u8;
                pfb_data[p + 2] = (len >> 16) as u8;
                pfb_data[p + 3] = (len >> 24) as u8;

                if (flags >> 8) == 5 {
                    /* End of font mark */
                    break;
                }

                if pfb_pos + 6 > pfb_len + 2 {
                    return Err(error);
                }

                pfb_data[pfb_pos as usize] = 0x80;
                pfb_pos += 1;

                type_ = flags >> 8;
                len = rlen;

                pfb_data[pfb_pos as usize] = type_ as u8;
                pfb_pos += 1;
                pfb_lenpos = pfb_pos;
                for _ in 0..4 {
                    pfb_data[pfb_pos as usize] = 0; /* 4-byte length, fill in later */
                    pfb_pos += 1;
                }
            }

            if pfb_pos > pfb_len || pfb_pos + rlen > pfb_len {
                return Err(error);
            }

            let (a, b) = (pfb_pos as usize, (pfb_pos + rlen) as usize);
            stream.read(&mut pfb_data[a..b])?;

            pfb_pos += rlen;
        }

        error = FT_ERR_ARRAY_TOO_LARGE;

        if pfb_pos + 2 > pfb_len + 2 {
            return Err(error);
        }
        pfb_data[pfb_pos as usize] = 0x80;
        pfb_data[pfb_pos as usize + 1] = 3;
        pfb_pos += 2;

        if pfb_lenpos + 3 > pfb_len + 2 {
            return Err(error);
        }
        let p = pfb_lenpos as usize;
        pfb_data[p] = len as u8;
        pfb_data[p + 1] = (len >> 8) as u8;
        pfb_data[p + 2] = (len >> 16) as u8;
        pfb_data[p + 3] = (len >> 24) as u8;
        Ok(())
    })();

    match res {
        Ok(()) => {
            pfb_data.truncate(pfb_pos as usize);
            open_face_from_buffer(library, pfb_data, face_index, Some("type1"))
        }
        /* Exit2: */
        Err(_) => Err(FT_ERR_CANNOT_OPEN_RESOURCE),
    }
}

/// `Mac_Read_sfnt_Resource`: The resource header says we've got
/// resource_cnt `sfnt' (TrueType/OpenType) resources in this file.  Look
/// through them for the one indicated by face_index, load it into mem,
/// pass it on to the truetype driver, and return it.
fn mac_read_sfnt_resource(
    library: &FtLibrary,
    stream: &mut FtStreamRec,
    offsets: &[FtLong],
    mut face_index: FtLong,
) -> FtResult<FtFace> {
    let resource_cnt = offsets.len() as FtLong;
    let face_index_in_resource: FtLong = 0;

    if face_index < 0 {
        face_index = -face_index - 1;
    }
    if face_index >= resource_cnt {
        return Err(FT_ERR_CANNOT_OPEN_RESOURCE);
    }

    let flag_offset = offsets[face_index as usize] as FtULong;
    stream.seek(flag_offset)?;

    let rlen = stream.read_ulong()? as FtULong;
    if rlen == 0 {
        return Err(FT_ERR_CANNOT_OPEN_RESOURCE);
    }
    if rlen > FT_MAC_RFORK_MAX_LEN {
        return Err(FT_ERR_INVALID_OFFSET);
    }

    if let Ok(face) = open_face_ps_from_sfnt_stream(library, stream, face_index) {
        return Ok(face);
    }

    /* rewind sfnt stream before open_face_PS_from_sfnt_stream() */
    stream.seek(flag_offset + 4)?;

    let mut sfnt_data = ft_qalloc(rlen as FtLong)?;
    stream.read(&mut sfnt_data)?;

    let is_cff = rlen > 4 && &sfnt_data[..4] == b"OTTO";
    open_face_from_buffer(
        library,
        sfnt_data,
        face_index_in_resource,
        Some(if is_cff { "cff" } else { "truetype" }),
    )
}

/// `IsMacResource`: Check for a valid resource fork header, or a valid
/// dfont header.  In a resource fork the first 16 bytes are repeated at
/// the location specified by bytes 4-7.  In a dfont bytes 4-7 point to 16
/// bytes of zeroes instead.
fn is_mac_resource(
    library: &FtLibrary,
    stream: &mut FtStreamRec,
    resource_offset: FtLong,
    face_index: FtLong,
) -> FtResult<FtFace> {
    let (map_offset, rdata_pos) = ft_raccess_get_header_info(stream, resource_offset)?;

    /* POST resources must be sorted to concatenate properly */
    if let Ok(data_offsets) =
        ft_raccess_get_data_offsets(stream, map_offset, rdata_pos, TTAG_POST as FtLong, true)
    {
        let mut face = mac_read_post_resource(library, stream, &data_offsets, face_index)?;
        /* POST exists in an LWFN providing a single face */
        face.num_faces = 1;
        return Ok(face);
    }

    /* sfnt resources should not be sorted to preserve the face order by
    QuickDraw API */
    let data_offsets =
        ft_raccess_get_data_offsets(stream, map_offset, rdata_pos, TTAG_sfnt as FtLong, false)?;
    let count = data_offsets.len() as FtLong;
    let face_index_internal = face_index % count;

    let mut face = mac_read_sfnt_resource(library, stream, &data_offsets, face_index_internal)?;
    face.num_faces = count;
    Ok(face)
}

/// `IsMacBinary`: Check for a valid macbinary header, and if we find one
/// check that the (flattened) resource fork in it is valid.
fn is_mac_binary(
    library: &FtLibrary,
    stream: &mut FtStreamRec,
    face_index: FtLong,
) -> FtResult<FtFace> {
    let mut header = [0u8; 128];

    stream.seek(0)?;

    stream.read(&mut header)?;

    if header[0] != 0
        || header[74] != 0
        || header[82] != 0
        || header[1] == 0
        || header[1] > 33
        || header[63] != 0
        || header[2 + header[1] as usize] != 0
        || header[0x53] > 0x7F
    {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    let dlen: FtLong = ((header[0x53] as FtLong) << 24)
        | ((header[0x54] as FtLong) << 16)
        | ((header[0x55] as FtLong) << 8)
        | header[0x56] as FtLong;

    let offset = 128 + ((dlen + 127) & !127);

    is_mac_resource(library, stream, offset, face_index)
}

/// `load_mac_face`: Check for some macintosh formats without Carbon
/// framework. Is this a macbinary file?  If so look at the resource fork.
/// Is this a mac dfont file? Is this an old style resource fork? (in
/// data) Else call load_face_in_embedded_rfork to try extra rules (defined
/// in `ftrfork.c') -- which only applies to fonts opened by path name,
/// which isn't supported here.
fn load_mac_face(
    library: &FtLibrary,
    stream: &mut FtStreamRec,
    face_index: FtLong,
) -> FtResult<FtFace> {
    match is_mac_binary(library, stream, face_index) {
        Err(e) if ft_err_eq(e, FT_ERR_UNKNOWN_FILE_FORMAT) => {
            is_mac_resource(library, stream, 0, face_index)
        }
        other => other,
    }
}

/// `FT_Open_Face`
pub fn ft_open_face(library: &FtLibrary, args: FtOpenArgs, face_index: FtLong) -> FtResult<FtFace> {
    ft_open_face_internal(library, args, face_index, true)
}

/// `ft_open_face_internal`
fn ft_open_face_internal(
    library: &FtLibrary,
    mut args: FtOpenArgs,
    mut face_index: FtLong,
    test_mac_fonts: bool,
) -> FtResult<FtFace> {
    let mut error: FtError;
    let mut face: Option<FtFace> = None;

    /* only use lower 31 bits together with sign bit */
    if face_index > 0 {
        face_index &= 0x7FFFFFFF;
    } else {
        face_index = face_index.wrapping_neg();
        face_index &= 0x7FFFFFFF;
        face_index = face_index.wrapping_neg();
    }

    /* create input stream */
    let (stream, mut external_stream) = ft_stream_new(&mut args)?;
    let mut stream = Some(stream);

    let params: Vec<FtParameter> = if args.flags & FT_OPEN_PARAMS != 0 {
        args.params.clone()
    } else {
        Vec::new()
    };

    /* If the font driver is specified in the `args' structure, use */
    /* it.  Otherwise, we scan the list of registered drivers.      */
    if args.flags & FT_OPEN_DRIVER != 0 && args.driver.is_some() {
        let driver = args.driver.unwrap();

        /* not all modules are drivers, so check... */
        if library.modules[driver].is_driver() {
            match open_face(
                library,
                driver,
                stream.take().unwrap(),
                external_stream,
                face_index,
                &params,
            ) {
                Ok(f) => face = Some(f),
                Err((e, _, _)) => return Err(e),
            }
        } else {
            return Err(FT_ERR_INVALID_HANDLE);
        }
    } else {
        error = FT_ERR_MISSING_MODULE;

        /* check each font driver for an appropriate format */
        let mut fail3 = false;
        for (cur, module) in library.modules.iter().enumerate() {
            /* not all modules are font drivers, so check... */
            if module.is_driver() {
                match open_face(
                    library,
                    cur,
                    stream.take().unwrap(),
                    external_stream,
                    face_index,
                    &params,
                ) {
                    Ok(f) => {
                        face = Some(f);
                        break;
                    }
                    Err((e, s, ext)) => {
                        error = e;
                        stream = s;
                        external_stream = ext;
                    }
                }

                if test_mac_fonts
                    && module.clazz.root().module_name == "truetype"
                    && ft_err_eq(error, FT_ERR_TABLE_MISSING)
                {
                    let st = stream.as_mut().unwrap();
                    /* TrueType but essential tables are missing */
                    if let Err(e) = st.seek(0) {
                        error = e;
                        fail3 = true;
                        break;
                    }

                    match open_face_ps_from_sfnt_stream(library, st, face_index) {
                        Ok(f) => return Ok(f),
                        Err(e) => error = e,
                    }
                }

                if ft_err_neq(error, FT_ERR_UNKNOWN_FILE_FORMAT) {
                    fail3 = true;
                    break;
                }
            }
        }

        if face.is_none() {
            let _ = fail3;

            /* Fail3: */
            /* If we are on the mac, and we get an                          */
            /* FT_Err_Invalid_Stream_Operation it may be because we have an */
            /* empty data fork, so we need to check the resource fork.      */
            if ft_err_neq(error, FT_ERR_CANNOT_OPEN_STREAM)
                && ft_err_neq(error, FT_ERR_UNKNOWN_FILE_FORMAT)
                && ft_err_neq(error, FT_ERR_INVALID_STREAM_OPERATION)
            {
                return Err(error);
            }

            if test_mac_fonts {
                match load_mac_face(library, stream.as_mut().unwrap(), face_index) {
                    Ok(f) => {
                        /* We don't want to go to Success here.  We've already done   */
                        /* that.  On the other hand, if we succeeded we still need to */
                        /* close this stream (we opened a different stream which      */
                        /* extracted the interesting information out of this stream   */
                        /* here.  That stream will still be open and the face will    */
                        /* point to it).                                              */
                        return Ok(f);
                    }
                    Err(e) => error = e,
                }
            }

            if ft_err_neq(error, FT_ERR_UNKNOWN_FILE_FORMAT) {
                return Err(error);
            }

            /* no driver is able to handle this format */
            return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
        }
    }

    /* Success: */
    let mut face = face.unwrap();

    /* now allocate a glyph slot object for the face */
    if face_index >= 0 {
        let fail = (|| -> FtResult<()> {
            face.glyph = ft_new_glyph_slot(&face)?;

            /* finally, allocate a size object for the face */
            ft_new_size(&mut face)?;
            face.has_slot_and_size = true;
            Ok(())
        })();
        if let Err(e) = fail {
            destroy_face(&mut face);
            return Err(e);
        }
    }

    /* some checks */
    if ft_is_scalable(&face) {
        if face.height < 0 {
            face.height = face.height.wrapping_neg();
        }

        if !ft_has_vertical(&face) {
            face.max_advance_height = face.height;
        }
    }

    if ft_has_fixed_sizes(&face) {
        for bsize in face.available_sizes.iter_mut() {
            if bsize.height < 0 {
                bsize.height = bsize.height.wrapping_neg();
            }
            if bsize.x_ppem < 0 {
                bsize.x_ppem = bsize.x_ppem.wrapping_neg();
            }
            if bsize.y_ppem < 0 {
                bsize.y_ppem = bsize.y_ppem.wrapping_neg();
            }

            /* check whether negation actually has worked */
            if bsize.height < 0 || bsize.x_ppem < 0 || bsize.y_ppem < 0 {
                bsize.width = 0;
                bsize.height = 0;
                bsize.size = 0;
                bsize.x_ppem = 0;
                bsize.y_ppem = 0;
            }
        }
    }

    /* initialize internal face data */
    {
        let internal = &mut face.internal;

        internal.transform_matrix.xx = 0x10000;
        internal.transform_matrix.xy = 0;
        internal.transform_matrix.yx = 0;
        internal.transform_matrix.yy = 0x10000;

        internal.transform_delta.x = 0;
        internal.transform_delta.y = 0;

        internal.refcount = 1;

        internal.no_stem_darkening = -1;
    }

    Ok(face)
}

/// `FT_Attach_Stream`
pub fn ft_attach_stream(face: &mut FtFace, mut parameters: FtOpenArgs) -> FtResult<()> {
    let (mut stream, _) = ft_stream_new(&mut parameters)?;

    /* we implement FT_Attach_Stream in each driver through the */
    /* `attach_file' interface                                  */

    let clazz = face.driver_class();
    match clazz.attach_file {
        Some(attach_file) => attach_file(face, &mut stream),
        None => Err(FT_ERR_UNIMPLEMENTED_FEATURE),
    }

    /* close the attached stream */
}

/// `FT_Done_Face`
pub fn ft_done_face(mut face: FtFace) {
    destroy_face(&mut face);
}

/// `FT_New_Size` (for the face's single size)
fn ft_new_size(face: &mut FtFace) -> FtResult<()> {
    let clazz = face.driver_class();

    face.size = FtSizeRec::default();

    if let Some(init_size) = clazz.init_size {
        init_size(face)?;
    }

    Ok(())
}

/// `FT_Match_Size`
pub fn ft_match_size(
    face: &FtFaceRec,
    req: &FtSizeRequestRec,
    ignore_width: bool,
    size_index: Option<&mut FtULong>,
) -> FtResult<()> {
    if !ft_has_fixed_sizes(face) {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    }

    /* FT_Bitmap_Size doesn't provide enough info... */
    if req.type_ != FT_SIZE_REQUEST_TYPE_NOMINAL {
        return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
    }

    let mut w = ft_request_width(req);
    let mut h = ft_request_height(req);

    if req.width != 0 && req.height == 0 {
        h = w;
    } else if req.width == 0 && req.height != 0 {
        w = h;
    }

    w = ft_pix_round(w);
    h = ft_pix_round(h);

    if w == 0 || h == 0 {
        return Err(FT_ERR_INVALID_PIXEL_SIZE);
    }

    for i in 0..face.num_fixed_sizes.max(0) as usize {
        let bsize = &face.available_sizes[i];

        if h != ft_pix_round(bsize.y_ppem) {
            continue;
        }

        if w == ft_pix_round(bsize.x_ppem) || ignore_width {
            if let Some(size_index) = size_index {
                *size_index = i as FtULong;
            }

            return Ok(());
        }
    }

    Err(FT_ERR_INVALID_PIXEL_SIZE)
}

/// `ft_synthesize_vertical_metrics`
pub fn ft_synthesize_vertical_metrics(metrics: &mut FtGlyphMetrics, mut advance: FtPos) {
    let mut height = metrics.height;

    /* compensate for glyph with bbox above/below the baseline */
    if metrics.horiBearingY < 0 {
        if height < metrics.horiBearingY {
            height = metrics.horiBearingY;
        }
    } else if metrics.horiBearingY > 0 {
        height = height.wrapping_sub(metrics.horiBearingY);
    }

    /* the factor 1.2 is a heuristical value */
    if advance == 0 {
        advance = height.wrapping_mul(12) / 10;
    }

    metrics.vertBearingX = metrics.horiBearingX.wrapping_sub(metrics.horiAdvance / 2);
    metrics.vertBearingY = (advance.wrapping_sub(height)) / 2;
    metrics.vertAdvance = advance;
}

/// `ft_recompute_scaled_metrics`
fn ft_recompute_scaled_metrics(face: &FtFaceRec, metrics: &mut FtSizeMetrics) {
    /* Compute root ascender, descender, test height, and max_advance */

    metrics.ascender = ft_pix_ceil(ft_mul_fix(face.ascender as FtLong, metrics.y_scale));

    metrics.descender = ft_pix_floor(ft_mul_fix(face.descender as FtLong, metrics.y_scale));

    metrics.height = ft_pix_round(ft_mul_fix(face.height as FtLong, metrics.y_scale));

    metrics.max_advance = ft_pix_round(ft_mul_fix(
        face.max_advance_width as FtLong,
        metrics.x_scale,
    ));
}

/// `FT_Select_Metrics`
pub fn ft_select_metrics(face: &mut FtFaceRec, strike_index: FtULong) {
    let bsize = face.available_sizes[strike_index as usize];
    let mut metrics = face.size.metrics;

    metrics.x_ppem = ((bsize.x_ppem + 32) >> 6) as FtUShort;
    metrics.y_ppem = ((bsize.y_ppem + 32) >> 6) as FtUShort;

    if ft_is_scalable(face) {
        metrics.x_scale = ft_div_fix(bsize.x_ppem, face.units_per_EM as FtLong);
        metrics.y_scale = ft_div_fix(bsize.y_ppem, face.units_per_EM as FtLong);

        ft_recompute_scaled_metrics(face, &mut metrics);
    } else {
        metrics.x_scale = 1 << 16;
        metrics.y_scale = 1 << 16;
        metrics.ascender = bsize.y_ppem;
        metrics.descender = 0;
        metrics.height = (bsize.height as FtPos) << 6;
        metrics.max_advance = bsize.x_ppem;
    }
    face.size.metrics = metrics;
}

/// `FT_Request_Metrics`
pub fn ft_request_metrics(face: &mut FtFaceRec, req: &FtSizeRequestRec) -> FtResult<()> {
    let mut metrics = face.size.metrics;

    if ft_is_scalable(face) {
        let mut w: FtLong = 0;
        let mut h: FtLong = 0;
        let mut scaled_w: FtLong;
        let mut scaled_h: FtLong;

        let mut calculate_ppem_only = false;
        match req.type_ {
            FT_SIZE_REQUEST_TYPE_NOMINAL => {
                w = face.units_per_EM as FtLong;
                h = w;
            }

            FT_SIZE_REQUEST_TYPE_REAL_DIM => {
                w = face.ascender as FtLong - face.descender as FtLong;
                h = w;
            }

            FT_SIZE_REQUEST_TYPE_BBOX => {
                w = face.bbox.xMax.wrapping_sub(face.bbox.xMin);
                h = face.bbox.yMax.wrapping_sub(face.bbox.yMin);
            }

            FT_SIZE_REQUEST_TYPE_CELL => {
                w = face.max_advance_width as FtLong;
                h = face.ascender as FtLong - face.descender as FtLong;
            }

            FT_SIZE_REQUEST_TYPE_SCALES => {
                metrics.x_scale = req.width as FtFixed;
                metrics.y_scale = req.height as FtFixed;
                if metrics.x_scale == 0 {
                    metrics.x_scale = metrics.y_scale;
                } else if metrics.y_scale == 0 {
                    metrics.y_scale = metrics.x_scale;
                }
                calculate_ppem_only = true;
            }

            _ => {}
        }

        scaled_w = 0;
        scaled_h = 0;

        if !calculate_ppem_only {
            /* to be on the safe side */
            if w < 0 {
                w = -w;
            }

            if h < 0 {
                h = -h;
            }

            scaled_w = ft_request_width(req);
            scaled_h = ft_request_height(req);

            /* determine scales */
            if req.height != 0 || req.width == 0 {
                if h == 0 {
                    face.size.metrics = metrics;
                    return Err(FT_ERR_DIVIDE_BY_ZERO);
                }

                metrics.y_scale = ft_div_fix(scaled_h, h);
            }

            if req.width != 0 {
                if w == 0 {
                    face.size.metrics = metrics;
                    return Err(FT_ERR_DIVIDE_BY_ZERO);
                }

                metrics.x_scale = ft_div_fix(scaled_w, w);
            } else {
                metrics.x_scale = metrics.y_scale;
                scaled_w = ft_mul_div(scaled_h, w, h);
            }

            if req.height == 0 {
                metrics.y_scale = metrics.x_scale;
                scaled_h = ft_mul_div(scaled_w, h, w);
            }

            if req.type_ == FT_SIZE_REQUEST_TYPE_CELL {
                if metrics.y_scale > metrics.x_scale {
                    metrics.y_scale = metrics.x_scale;
                } else {
                    metrics.x_scale = metrics.y_scale;
                }
            }
        }

        /* Calculate_Ppem: */
        /* calculate the ppems */
        if req.type_ != FT_SIZE_REQUEST_TYPE_NOMINAL {
            scaled_w = ft_mul_fix(face.units_per_EM as FtLong, metrics.x_scale);
            scaled_h = ft_mul_fix(face.units_per_EM as FtLong, metrics.y_scale);
        }

        scaled_w = (scaled_w.wrapping_add(32)) >> 6;
        scaled_h = (scaled_h.wrapping_add(32)) >> 6;
        if scaled_w > 0xFFFF || scaled_h > 0xFFFF {
            face.size.metrics = metrics;
            return Err(FT_ERR_INVALID_PIXEL_SIZE);
        }

        metrics.x_ppem = scaled_w as FtUShort;
        metrics.y_ppem = scaled_h as FtUShort;

        ft_recompute_scaled_metrics(face, &mut metrics);
    } else {
        metrics = FtSizeMetrics::default();
        metrics.x_scale = 1 << 16;
        metrics.y_scale = 1 << 16;
    }

    face.size.metrics = metrics;
    Ok(())
}

/// `FT_Select_Size`
pub fn ft_select_size(face: &mut FtFace, strike_index: FtInt) -> FtResult<()> {
    if !ft_has_fixed_sizes(face) {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    }

    if strike_index < 0 || strike_index >= face.num_fixed_sizes {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let clazz = face.driver_class();

    if let Some(select_size) = clazz.select_size {
        select_size(face, strike_index as FtULong)
    } else {
        ft_select_metrics(face, strike_index as FtULong);
        Ok(())
    }
}

/// `FT_Request_Size`
pub fn ft_request_size(face: &mut FtFace, req: &FtSizeRequestRec) -> FtResult<()> {
    if !face.has_slot_and_size {
        return Err(FT_ERR_INVALID_SIZE_HANDLE);
    }

    if req.width < 0 || req.height < 0 || req.type_ >= FT_SIZE_REQUEST_TYPE_MAX {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* signal the auto-hinter to recompute its size metrics */
    /* (if requested)                                       */
    face.size.internal.autohint_metrics.x_scale = 0;

    let clazz = face.driver_class();

    if let Some(request_size) = clazz.request_size {
        request_size(face, req)
    } else if !ft_is_scalable(face) && ft_has_fixed_sizes(face) {
        /*
         * The reason that a driver doesn't have `request_size' defined is
         * either that the scaling here suffices or that the supported formats
         * are bitmap-only and size matching is not implemented.
         *
         * In the latter case, a simple size matching is done.
         */
        let mut strike_index: FtULong = 0;
        ft_match_size(face, req, false, Some(&mut strike_index))?;

        ft_select_size(face, strike_index as FtInt)
    } else {
        ft_request_metrics(face, req)
    }
}

/// `FT_Set_Char_Size`
pub fn ft_set_char_size(
    face: &mut FtFace,
    mut char_width: FtF26Dot6,
    mut char_height: FtF26Dot6,
    mut horz_resolution: FtUInt,
    mut vert_resolution: FtUInt,
) -> FtResult<()> {
    /* check of `face' delayed to `FT_Request_Size' */

    if char_width == 0 {
        char_width = char_height;
    } else if char_height == 0 {
        char_height = char_width;
    }

    if horz_resolution == 0 {
        horz_resolution = vert_resolution;
    } else if vert_resolution == 0 {
        vert_resolution = horz_resolution;
    }

    if char_width < 64 {
        char_width = 64;
    }
    if char_height < 64 {
        char_height = 64;
    }

    if horz_resolution == 0 {
        horz_resolution = 72;
        vert_resolution = 72;
    }

    let req = FtSizeRequestRec {
        type_: FT_SIZE_REQUEST_TYPE_NOMINAL,
        width: char_width,
        height: char_height,
        horiResolution: horz_resolution,
        vertResolution: vert_resolution,
    };

    ft_request_size(face, &req)
}

/// `FT_Set_Pixel_Sizes`
pub fn ft_set_pixel_sizes(
    face: &mut FtFace,
    mut pixel_width: FtUInt,
    mut pixel_height: FtUInt,
) -> FtResult<()> {
    /* check of `face' delayed to `FT_Request_Size' */

    if pixel_width == 0 {
        pixel_width = pixel_height;
    } else if pixel_height == 0 {
        pixel_height = pixel_width;
    }

    if pixel_width < 1 {
        pixel_width = 1;
    }
    if pixel_height < 1 {
        pixel_height = 1;
    }

    /* use `>=' to avoid potential compiler warning on 16bit platforms */
    if pixel_width >= 0xFFFF {
        pixel_width = 0xFFFF;
    }
    if pixel_height >= 0xFFFF {
        pixel_height = 0xFFFF;
    }

    let req = FtSizeRequestRec {
        type_: FT_SIZE_REQUEST_TYPE_NOMINAL,
        width: (pixel_width << 6) as FtLong,
        height: (pixel_height << 6) as FtLong,
        horiResolution: 0,
        vertResolution: 0,
    };

    ft_request_size(face, &req)
}

/// `FT_Get_Kerning`
pub fn ft_get_kerning(
    face: &mut FtFace,
    left_glyph: FtUInt,
    right_glyph: FtUInt,
    kern_mode: FtUInt,
) -> FtResult<FtVector> {
    let mut akerning = FtVector::default();

    let clazz = face.driver_class();

    if let Some(get_kerning) = clazz.get_kerning {
        akerning = get_kerning(face, left_glyph, right_glyph)?;

        if kern_mode != FT_KERNING_UNSCALED {
            akerning.x = ft_mul_fix(akerning.x, face.size.metrics.x_scale);
            akerning.y = ft_mul_fix(akerning.y, face.size.metrics.y_scale);

            if kern_mode != FT_KERNING_UNFITTED {
                let orig_x = akerning.x;
                let orig_y = akerning.y;

                /* we scale down kerning values for small ppem values */
                /* to avoid that rounding makes them too big.         */
                /* `25' has been determined heuristically.            */
                if face.size.metrics.x_ppem < 25 {
                    akerning.x = ft_mul_div(orig_x, face.size.metrics.x_ppem as FtLong, 25);
                }
                if face.size.metrics.y_ppem < 25 {
                    akerning.y = ft_mul_div(orig_y, face.size.metrics.y_ppem as FtLong, 25);
                }

                akerning.x = ft_pix_round(akerning.x);
                akerning.y = ft_pix_round(akerning.y);
            }
        }
    }

    Ok(akerning)
}

/// `FT_Select_Charmap`
pub fn ft_select_charmap(face: &mut FtFace, encoding: FtEncoding) -> FtResult<()> {
    /* FT_ENCODING_NONE is a valid encoding for BDF, PCF, and Windows FNT */
    if encoding == FT_ENCODING_NONE && face.num_charmaps == 0 {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* FT_ENCODING_UNICODE is special.  We try to find the `best' Unicode */
    /* charmap available, i.e., one with UCS-4 characters, if possible.   */
    /*                                                                    */
    /* This is done by find_unicode_charmap() above, to share code.       */
    if encoding == FT_ENCODING_UNICODE {
        return find_unicode_charmap(face);
    }

    if face.charmaps.is_empty() {
        return Err(FT_ERR_INVALID_CHARMAP_HANDLE);
    }

    for cur in 0..face.num_charmaps.max(0) as usize {
        if face.charmaps[cur].charmap.encoding == encoding {
            face.charmap = Some(cur);
            return Ok(());
        }
    }

    Err(FT_ERR_INVALID_ARGUMENT)
}

/// `FT_Set_Charmap`
pub fn ft_set_charmap(face: &mut FtFace, charmap: usize) -> FtResult<()> {
    if face.charmaps.is_empty() {
        return Err(FT_ERR_INVALID_CHARMAP_HANDLE);
    }

    if charmap < face.num_charmaps.max(0) as usize && ft_get_cmap_format(face, charmap) != 14 {
        face.charmap = Some(charmap);
        return Ok(());
    }

    Err(FT_ERR_INVALID_ARGUMENT)
}

/// `FT_CMap_New`: makes a charmap object and adds it to the face's
/// charmaps (the class's `init` is the caller's construction of `data`)
pub fn ft_cmap_new(
    face: &mut FtFaceRec,
    clazz: &'static FtCMapClassRec,
    data: FtCMapData,
    charmap: FtCharMapRec,
) -> FtResult<usize> {
    let cmap = FtCMapRec {
        charmap,
        clazz,
        data,
    };

    /* add it to our list of charmaps */
    if face.charmaps.try_reserve(1).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    face.charmaps.push(cmap);
    face.num_charmaps += 1;

    Ok(face.charmaps.len() - 1)
}

/// `FT_Get_Char_Index`
pub fn ft_get_char_index(face: &FtFaceRec, charcode: FtULong) -> FtUInt {
    let mut result = 0;

    if let Some(cm) = face.charmap {
        let cmap = &face.charmaps[cm];

        result = (cmap.clazz.char_index)(cmap, charcode as FtUInt32);
        if result as FtLong >= face.num_glyphs {
            result = 0;
        }
    }

    result
}

/// `FT_Get_First_Char`
pub fn ft_get_first_char(face: &mut FtFaceRec, agindex: &mut FtUInt) -> FtULong {
    let mut result = 0;
    let mut gindex = 0;

    /* only do something if we have a charmap, and we have glyphs at all */
    if face.charmap.is_some() && face.num_glyphs != 0 {
        gindex = ft_get_char_index(face, 0);
        if gindex == 0 {
            result = ft_get_next_char(face, 0, &mut gindex);
        }
    }

    *agindex = gindex;

    result
}

/// `FT_Get_Next_Char`
pub fn ft_get_next_char(face: &mut FtFaceRec, charcode: FtULong, agindex: &mut FtUInt) -> FtULong {
    let mut result = 0;
    let mut gindex = 0;

    if let (Some(cm), true) = (face.charmap, face.num_glyphs != 0) {
        let mut code = charcode as FtUInt32;
        let num_glyphs = face.num_glyphs;
        let cmap = &mut face.charmaps[cm];

        loop {
            gindex = (cmap.clazz.char_next)(cmap, &mut code);

            if (gindex as FtLong) < num_glyphs {
                break;
            }
        }

        result = if gindex == 0 { 0 } else { code as FtULong };
    }

    *agindex = gindex;

    result
}

/// `FT_Face_Properties`
pub fn ft_face_properties(face: &mut FtFaceRec, properties: &[FtParameter]) -> FtResult<()> {
    for p in properties {
        if p.tag == FT_PARAM_TAG_STEM_DARKENING {
            match p.data {
                FtParameterData::Bool(b) => {
                    face.internal.no_stem_darkening = if b { 0 } else { 1 };
                }
                _ => {
                    /* use module default */
                    face.internal.no_stem_darkening = -1;
                }
            }
        } else if p.tag == FT_PARAM_TAG_LCD_FILTER_WEIGHTS {
            return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
        } else if p.tag == FT_PARAM_TAG_RANDOM_SEED {
            match p.data {
                FtParameterData::Int32(seed) => {
                    face.internal.random_seed = seed;
                    if face.internal.random_seed < 0 {
                        face.internal.random_seed = 0;
                    }
                }
                _ => {
                    /* use module default */
                    face.internal.random_seed = -1;
                }
            }
        } else {
            return Err(FT_ERR_INVALID_ARGUMENT);
        }
    }

    Ok(())
}

/// `FT_Face_GetCharVariantIndex`
pub fn ft_face_get_char_variant_index(
    face: &FtFace,
    charcode: FtULong,
    variant_selector: FtULong,
) -> FtUInt {
    let mut result = 0;

    if let Some(cm) = face.charmap {
        if face.charmaps[cm].charmap.encoding == FT_ENCODING_UNICODE {
            if let Some(vc) = find_variant_selector_charmap(face) {
                let vcmap = &face.charmaps[vc];
                let ucmap = &face.charmaps[cm];

                if let Some(f) = vcmap.clazz.char_var_index {
                    result = f(
                        vcmap,
                        ucmap,
                        charcode as FtUInt32,
                        variant_selector as FtUInt32,
                    );
                }
            }
        }
    }

    result
}

/// `FT_Face_GetCharVariantIsDefault`
pub fn ft_face_get_char_variant_is_default(
    face: &FtFace,
    charcode: FtULong,
    variant_selector: FtULong,
) -> FtInt {
    let mut result = -1;

    if let Some(vc) = find_variant_selector_charmap(face) {
        let vcmap = &face.charmaps[vc];

        if let Some(f) = vcmap.clazz.char_var_default {
            result = f(vcmap, charcode as FtUInt32, variant_selector as FtUInt32);
        }
    }

    result
}

/// `FT_Face_GetVariantSelectors`
pub fn ft_face_get_variant_selectors(face: &FtFace) -> Option<Vec<FtUInt32>> {
    let vc = find_variant_selector_charmap(face)?;
    let vcmap = &face.charmaps[vc];

    (vcmap.clazz.variant_list?)(vcmap)
}

/// `FT_Face_GetVariantsOfChar`
pub fn ft_face_get_variants_of_char(face: &FtFace, charcode: FtULong) -> Option<Vec<FtUInt32>> {
    let vc = find_variant_selector_charmap(face)?;
    let vcmap = &face.charmaps[vc];

    (vcmap.clazz.charvariant_list?)(vcmap, charcode as FtUInt32)
}

/// `FT_Face_GetCharsOfVariant`
pub fn ft_face_get_chars_of_variant(
    face: &FtFace,
    variant_selector: FtULong,
) -> Option<Vec<FtUInt32>> {
    let vc = find_variant_selector_charmap(face)?;
    let vcmap = &face.charmaps[vc];

    (vcmap.clazz.variantchar_list?)(vcmap, variant_selector as FtUInt32)
}

/// `FT_FACE_FIND_SERVICE`: looks up a service in the face's driver.
pub fn ft_face_find_service(face: &FtFaceRec, service_id: &str) -> Option<FtService> {
    ft_module_get_service(face.library(), face.driver, service_id, false)
}

/// `FT_Get_Name_Index`
pub fn ft_get_name_index(face: &mut FtFace, glyph_name: &[u8]) -> FtUInt {
    let mut result = 0;

    if ft_has_glyph_names(face) {
        if let Some(FtService::GlyphDict(service)) =
            ft_face_find_service(face, FT_SERVICE_ID_GLYPH_DICT)
        {
            result = (service.name_index)(face, glyph_name);
        }
    }

    result
}

/// `FT_Get_Glyph_Name`
pub fn ft_get_glyph_name(
    face: &mut FtFace,
    glyph_index: FtUInt,
    buffer: &mut [u8],
) -> FtResult<()> {
    if buffer.is_empty() {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* clean up buffer */
    buffer[0] = b'\0';

    if glyph_index as FtLong >= face.num_glyphs {
        return Err(FT_ERR_INVALID_GLYPH_INDEX);
    }

    if !ft_has_glyph_names(face) {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    match ft_face_find_service(face, FT_SERVICE_ID_GLYPH_DICT) {
        Some(FtService::GlyphDict(service)) => (service.get_name)(face, glyph_index, buffer),
        _ => Err(FT_ERR_INVALID_ARGUMENT),
    }
}

/// `FT_Get_Postscript_Name`
pub fn ft_get_postscript_name(face: &mut FtFace) -> Option<String> {
    match ft_face_find_service(face, FT_SERVICE_ID_POSTSCRIPT_FONT_NAME) {
        Some(FtService::PsFontName(service)) => (service.get_ps_font_name)(face),
        _ => None,
    }
}

/// `FT_Get_Sfnt_Table` (documentation is in tttables.h)
pub fn ft_get_sfnt_table(face: &FtFace, tag: FtSfntTag) -> Option<FtSfntTable<'_>> {
    if ft_is_sfnt(face) {
        if let Some(FtService::SfntTable(service)) =
            ft_face_find_service(face, FT_SERVICE_ID_SFNT_TABLE)
        {
            return (service.get_table)(face, tag);
        }
    }

    None
}

/// `FT_Load_Sfnt_Table` (documentation is in tttables.h)
pub fn ft_load_sfnt_table(
    face: &mut FtFace,
    tag: FtULong,
    offset: FtLong,
    buffer: Option<&mut [u8]>,
    length: &mut FtULong,
) -> FtResult<()> {
    if !ft_is_sfnt(face) {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    }

    match ft_face_find_service(face, FT_SERVICE_ID_SFNT_TABLE) {
        Some(FtService::SfntTable(service)) => {
            (service.load_table)(face, tag, offset, buffer, length)
        }
        _ => Err(FT_ERR_UNIMPLEMENTED_FEATURE),
    }
}

/// `FT_Sfnt_Table_Info` (documentation is in tttables.h)
pub fn ft_sfnt_table_info(
    face: &FtFace,
    table_index: FtUInt,
    tag: &mut FtULong,
    length: &mut FtULong,
) -> FtResult<()> {
    let mut offset: FtULong = 0;

    /* test for valid `length' delayed to `service->table_info' */

    if !ft_is_sfnt(face) {
        return Err(FT_ERR_INVALID_FACE_HANDLE);
    }

    match ft_face_find_service(face, FT_SERVICE_ID_SFNT_TABLE) {
        Some(FtService::SfntTable(service)) => {
            (service.table_info)(face, table_index, tag, &mut offset, length)
        }
        _ => Err(FT_ERR_UNIMPLEMENTED_FEATURE),
    }
}

/// `FT_Get_CMap_Language_ID` (documentation is in tttables.h)
pub fn ft_get_cmap_language_id(face: &FtFace, charmap: usize) -> FtULong {
    match ft_face_find_service(face, FT_SERVICE_ID_TT_CMAP) {
        Some(FtService::TtCMaps(service)) => match (service.get_cmap_info)(face, charmap) {
            Ok(info) => info.language,
            Err(_) => 0,
        },
        _ => 0,
    }
}

/// `FT_Get_CMap_Format` (documentation is in tttables.h)
pub fn ft_get_cmap_format(face: &FtFace, charmap: usize) -> FtLong {
    match ft_face_find_service(face, FT_SERVICE_ID_TT_CMAP) {
        Some(FtService::TtCMaps(service)) => match (service.get_cmap_info)(face, charmap) {
            Ok(info) => info.format,
            Err(_) => -1,
        },
        _ => -1,
    }
}

/*************************************************************************/
/*                                                                       */
/*                        R E N D E R E R S                              */
/*                                                                       */
/*************************************************************************/

/// `FT_Lookup_Renderer`: lookup a renderer by glyph format in the
/// library's list; `node` is the position in the renderers list
pub fn ft_lookup_renderer(
    library: &FtLibraryRec,
    format: FtGlyphFormat,
    node: &mut Option<usize>,
) -> Option<usize> {
    let mut cur = match *node {
        Some(n) => n + 1,
        None => 0,
    };
    *node = None;

    while cur < library.renderers.len() {
        let renderer = library.renderers[cur];

        if library.modules[renderer].renderer_class().glyph_format == format {
            *node = Some(cur);

            return Some(renderer);
        }

        cur += 1;
    }

    None
}

/// `ft_lookup_glyph_renderer`
fn ft_lookup_glyph_renderer(library: &FtLibraryRec, slot: &FtGlyphSlotRec) -> Option<usize> {
    let mut result = library.cur_renderer;

    if result.is_none()
        || library.modules[result.unwrap()]
            .renderer_class()
            .glyph_format
            != slot.format
    {
        result = ft_lookup_renderer(library, slot.format, &mut None);
    }

    result
}

/// `ft_set_current_renderer`
fn ft_set_current_renderer(library: &mut FtLibraryRec) {
    let renderer = ft_lookup_renderer(library, FT_GLYPH_FORMAT_OUTLINE, &mut None);
    library.cur_renderer = renderer;
}

/// `ft_add_renderer`
fn ft_add_renderer(library: &mut FtLibraryRec, module: usize) -> FtResult<()> {
    /* allocate raster object if needed */
    /* (the rasters are stateless: see `FtRasterFuncs`) */

    /* add to list */
    library.renderers.push(module);
    ft_set_current_renderer(library);

    Ok(())
}

/// `FT_Get_Renderer`
pub fn ft_get_renderer(library: &FtLibraryRec, format: FtGlyphFormat) -> Option<usize> {
    ft_lookup_renderer(library, format, &mut None)
}

/// `FT_Render_Glyph_Internal`, for a slot not in a face (C's slots whose
/// `face` is NULL, as `FT_Glyph_To_Bitmap`'s)
pub fn ft_render_glyph_internal(
    library: &FtLibraryRec,
    slot: &mut FtGlyphSlotRec,
    render_mode: FtRenderMode,
) -> FtResult<()> {
    ft_render_glyph_with_renderers(library, slot, render_mode)
}

/// The renderer loop of `FT_Render_Glyph_Internal`.
fn ft_render_glyph_with_renderers(
    library: &FtLibraryRec,
    slot: &mut FtGlyphSlotRec,
    render_mode: FtRenderMode,
) -> FtResult<()> {
    let mut node: Option<usize> = None;
    let mut renderer;

    /* small shortcut for the very common case */
    if slot.format == FT_GLYPH_FORMAT_OUTLINE {
        renderer = library.cur_renderer;
        node = if library.renderers.is_empty() {
            None
        } else {
            Some(0)
        };
    } else {
        renderer = ft_lookup_renderer(library, slot.format, &mut node);
    }

    let mut error = FT_ERR_CANNOT_RENDER_GLYPH;
    while let Some(r) = renderer {
        let render = library.modules[r].renderer_class().render_glyph;
        error = match render {
            Some(f) => match f(library, r, slot, render_mode, None) {
                Ok(()) => 0,
                Err(e) => e,
            },
            None => FT_ERR_CANNOT_RENDER_GLYPH,
        };
        if error == 0 || ft_err_neq(error, FT_ERR_CANNOT_RENDER_GLYPH) {
            break;
        }

        /* FT_Err_Cannot_Render_Glyph is returned if the render mode   */
        /* is unsupported by the current renderer for this glyph image */
        /* format.                                                     */

        /* now, look for another renderer that supports the same */
        /* format.                                               */
        renderer = ft_lookup_renderer(library, slot.format, &mut node);
    }

    /* it is not an error if we cannot render a bitmap glyph */
    if ft_err_eq(error, FT_ERR_CANNOT_RENDER_GLYPH) && slot.format == FT_GLYPH_FORMAT_BITMAP {
        error = 0;
    }

    if error == 0 {
        Ok(())
    } else {
        Err(error)
    }
}

/// `FT_Render_Glyph` (on `face->glyph`), with `FT_Render_Glyph_Internal`
pub fn ft_render_glyph(face: &mut FtFace, render_mode: FtRenderMode) -> FtResult<()> {
    let library = face.library().clone();

    if face.glyph.internal.load_flags & FT_LOAD_COLOR != 0 {
        let base_glyph = face.glyph.glyph_index;
        let mut iterator = FtLayerIterator::default();
        let mut glyph_index = 0;
        let mut color_index = 0;

        /* check whether we have colored glyph layers */
        let have_layers = ft_get_color_glyph_layer(
            face,
            base_glyph,
            &mut glyph_index,
            &mut color_index,
            &mut iterator,
        );
        if have_layers {
            let error: FtResult<()> = match ft_new_glyph_slot(face) {
                Err(e) => Err(e),
                Ok(new_slot) => {
                    /* the new slot becomes `face->glyph'; `slot' is the old one */
                    let mut slot = std::mem::replace(&mut face.glyph, new_slot);
                    let mut err: FtResult<()>;

                    loop {
                        let mut load_flags = slot.internal.load_flags;

                        /* disable the `FT_LOAD_COLOR' flag to avoid recursion */
                        /* right here in this function                         */
                        load_flags &= !FT_LOAD_COLOR;

                        /* render into the new `face->glyph' glyph slot */
                        load_flags |= FT_LOAD_RENDER;

                        err = ft_load_glyph(face, glyph_index, load_flags);
                        if err.is_err() {
                            break;
                        }

                        /* blend new `face->glyph' into old `slot'; */
                        /* at the first call, `slot' is still empty */
                        err = {
                            let FtFace::Tt(ttface) = face else {
                                return Err(FT_ERR_INVALID_FACE_HANDLE);
                            };
                            let new_glyph = std::mem::take(&mut ttface.root.glyph);
                            let r = super::super::sfnt::ttcolr::tt_face_colr_blend_layer(
                                ttface,
                                color_index,
                                &mut slot,
                                &new_glyph,
                            );
                            ttface.root.glyph = new_glyph;
                            r
                        };
                        if err.is_err() {
                            break;
                        }

                        if !ft_get_color_glyph_layer(
                            face,
                            base_glyph,
                            &mut glyph_index,
                            &mut color_index,
                            &mut iterator,
                        ) {
                            break;
                        }
                    }

                    if err.is_ok() {
                        slot.format = FT_GLYPH_FORMAT_BITMAP;
                    }

                    /* this call also restores `slot' as the glyph slot */
                    let new_slot = std::mem::replace(&mut face.glyph, slot);
                    ft_done_glyph_slot(new_slot);
                    err
                }
            };

            if error.is_ok() {
                return Ok(());
            }

            /* Failed to do the colored layer.  Draw outline instead. */
            face.glyph.format = FT_GLYPH_FORMAT_OUTLINE;
        }
    }

    ft_render_glyph_with_renderers(&library, &mut face.glyph, render_mode)
}

/*************************************************************************/
/*                                                                       */
/*                         M O D U L E S                                 */
/*                                                                       */
/*************************************************************************/

/// `FT_Add_Module`
pub fn ft_add_module(library: &mut FtLibraryRec, clazz: FtModuleClassRef) -> FtResult<()> {
    const FREETYPE_VER_FIXED: FtLong =
        ((FREETYPE_MAJOR as FtLong) << 16) | FREETYPE_MINOR as FtLong;

    let root = clazz.root();

    /* check FreeType version */
    if root.module_requires > FREETYPE_VER_FIXED {
        return Err(FT_ERR_INVALID_VERSION);
    }

    /* look for a module with the same name in the library's table */
    if let Some(nn) = library.get_module(root.module_name) {
        /* this installed module has the same name, compare their versions */
        if root.module_version <= library.modules[nn].clazz.root().module_version {
            return Err(FT_ERR_LOWER_MODULE_VERSION);
        }

        /* (replacing modules isn't supported: there is one version of each) */
        return Err(FT_ERR_LOWER_MODULE_VERSION);
    }

    if library.modules.len() >= FT_MAX_MODULES {
        return Err(FT_ERR_TOO_MANY_DRIVERS);
    }

    /* allocate module object */
    let module = FtModuleRec {
        clazz,
        props: Mutex::new(None),
    };
    let index = library.modules.len();
    library.modules.push(module);

    /* check whether the module is a renderer - this must be performed */
    /* before the normal module initialization                         */
    if library.modules[index].is_renderer() {
        /* add to the renderers list */
        ft_add_renderer(library, index)?;
    }

    /* is the module a auto-hinter? */
    if library.modules[index].is_hinter() {
        library.auto_hinter = Some(index);
    }

    if let Some(module_init) = root.module_init {
        if let Err(e) = module_init(library, index) {
            /* Fail: */
            if library.modules[index].is_renderer() {
                library.renderers.retain(|&r| r != index);
                ft_set_current_renderer(library);
            }
            if library.auto_hinter == Some(index) {
                library.auto_hinter = None;
            }
            library.modules.pop();
            return Err(e);
        }
    }

    Ok(())
}

/// `FT_Get_Module`
pub fn ft_get_module(library: &FtLibraryRec, module_name: &str) -> Option<usize> {
    library.get_module(module_name)
}

/// `ft_module_get_service`
pub fn ft_module_get_service(
    library: &FtLibraryRec,
    module: usize,
    service_id: &str,
    global: bool,
) -> Option<FtService> {
    let m = &library.modules[module];

    /* first, look for the service in the module */
    let mut result = m
        .clazz
        .root()
        .get_interface
        .and_then(|gi| gi(m, service_id));

    if global && result.is_none() {
        /* we didn't find it, look in all other modules then */
        for (i, cur) in library.modules.iter().enumerate() {
            if i != module {
                if let Some(gi) = cur.clazz.root().get_interface {
                    result = gi(cur, service_id);
                    if result.is_some() {
                        break;
                    }
                }
            }
        }
    }

    result
}

/// `ft_property_do`
fn ft_property_do(
    library: &FtLibraryRec,
    module_name: &str,
    property_name: &str,
    value: Option<&FtPropertyValue>,
    value_is_string: bool,
) -> FtResult<Option<FtPropertyValue>> {
    /* search module */
    let cur = match library.get_module(module_name) {
        Some(c) => c,
        None => return Err(FT_ERR_MISSING_MODULE),
    };
    let module = &library.modules[cur];

    /* check whether we have a service interface */
    let get_interface = match module.clazz.root().get_interface {
        Some(gi) => gi,
        None => return Err(FT_ERR_UNIMPLEMENTED_FEATURE),
    };

    /* search property service */
    let service = match get_interface(module, FT_SERVICE_ID_PROPERTIES) {
        Some(FtService::Properties(s)) => s,
        _ => return Err(FT_ERR_UNIMPLEMENTED_FEATURE),
    };

    match value {
        Some(v) => {
            (service.set_property)(module, property_name, v, value_is_string)?;
            Ok(None)
        }
        None => Ok(Some((service.get_property)(module, property_name)?)),
    }
}

/// `FT_Property_Set`
pub fn ft_property_set(
    library: &FtLibraryRec,
    module_name: &str,
    property_name: &str,
    value: &FtPropertyValue,
) -> FtResult<()> {
    ft_property_do(library, module_name, property_name, Some(value), false).map(|_| ())
}

/// `FT_Property_Get`
pub fn ft_property_get(
    library: &FtLibraryRec,
    module_name: &str,
    property_name: &str,
) -> FtResult<FtPropertyValue> {
    ft_property_do(library, module_name, property_name, None, false)
        .map(|v| v.unwrap_or(FtPropertyValue::Int(0)))
}

/// `ft_property_string_set`: this variant is used for handling the
/// FREETYPE_PROPERTIES environment variable
pub fn ft_property_string_set(
    library: &FtLibraryRec,
    module_name: &str,
    property_name: &str,
    value: &str,
) -> FtResult<()> {
    ft_property_do(
        library,
        module_name,
        property_name,
        Some(&FtPropertyValue::Str(value.to_string())),
        true,
    )
    .map(|_| ())
}

/*************************************************************************/
/*                                                                       */
/*                         L I B R A R Y                                 */
/*                                                                       */
/*************************************************************************/

/// `FT_New_Library`
pub fn ft_new_library() -> FtLibraryRec {
    FtLibraryRec {
        version_major: FREETYPE_MAJOR,
        version_minor: FREETYPE_MINOR,
        version_patch: FREETYPE_PATCH,
        modules: Vec::new(),
        renderers: Vec::new(),
        cur_renderer: None,
        auto_hinter: None,
        lcd_geometry: Mutex::new([FtVector::default(); 3]),
    }
}

/// `FT_Library_Version`
pub fn ft_library_version(library: &FtLibraryRec) -> (FtInt, FtInt, FtInt) {
    (
        library.version_major,
        library.version_minor,
        library.version_patch,
    )
}

/// `FT_Get_TrueType_Engine_Type`
pub fn ft_get_truetype_engine_type(library: &FtLibraryRec) -> FtTrueTypeEngineType {
    let mut result = FT_TRUETYPE_ENGINE_TYPE_NONE;

    if let Some(module) = library.get_module("truetype") {
        if let Some(FtService::TrueTypeEngine(t)) =
            ft_module_get_service(library, module, FT_SERVICE_ID_TRUETYPE_ENGINE, false)
        {
            result = t;
        }
    }

    result
}

/// `FT_Get_SubGlyph_Info`
pub fn ft_get_subglyph_info(
    glyph: &FtGlyphSlotRec,
    sub_index: FtUInt,
) -> FtResult<(FtInt, FtUInt, FtInt, FtInt, FtMatrix)> {
    if !glyph.subglyphs.is_empty()
        && glyph.format == FT_GLYPH_FORMAT_COMPOSITE
        && sub_index < glyph.num_subglyphs
    {
        let subg = &glyph.subglyphs[sub_index as usize];

        return Ok((
            subg.index,
            subg.flags as FtUInt,
            subg.arg1,
            subg.arg2,
            subg.transform,
        ));
    }

    Err(FT_ERR_INVALID_ARGUMENT)
}

/// `FT_LayerIterator`
#[derive(Debug, Clone, Copy, Default)]
pub struct FtLayerIterator {
    pub num_layers: FtUInt,
    pub layer: FtUInt,
    /// `p`: an offset into the `COLR` table (`None` is C's NULL)
    pub p: Option<usize>,
}

/// `FT_Get_Color_Glyph_Layer`
pub fn ft_get_color_glyph_layer(
    face: &FtFace,
    base_glyph: FtUInt,
    aglyph_index: &mut FtUInt,
    acolor_index: &mut FtUInt,
    iterator: &mut FtLayerIterator,
) -> bool {
    if base_glyph as FtLong >= face.num_glyphs {
        return false;
    }

    if !ft_is_sfnt(face) {
        return false;
    }

    match face.tt() {
        Some(ttface) => super::super::sfnt::ttcolr::tt_face_get_colr_layer(
            ttface,
            base_glyph,
            aglyph_index,
            acolor_index,
            iterator,
        ),
        None => false,
    }
}
