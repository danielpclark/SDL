// Rust translation of src/autofit/afscript.h (with the script classes of
// afglobal.c) from FreeType (2.13.2, as SDL_ttf's external/freetype pins
// it), with `AF_CONFIG_OPTION_CJK` and `AF_CONFIG_OPTION_INDIC` defined.
// Copyright (C) 2013-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter scripts.

use super::afranges::*;
use super::aftypes::{
    AfScript, AfScriptClassRec, AF_HINTING_BOTTOM_TO_TOP, AF_HINTING_TOP_TO_BOTTOM,
};

/// `AF_SCRIPT_ADLM`: Adlam
pub const AF_SCRIPT_ADLM: AfScript = 0;
/// `AF_SCRIPT_ARAB`: Arabic
pub const AF_SCRIPT_ARAB: AfScript = 1;
/// `AF_SCRIPT_ARMN`: Armenian
pub const AF_SCRIPT_ARMN: AfScript = 2;
/// `AF_SCRIPT_AVST`: Avestan
pub const AF_SCRIPT_AVST: AfScript = 3;
/// `AF_SCRIPT_BAMU`: Bamum
pub const AF_SCRIPT_BAMU: AfScript = 4;
/// `AF_SCRIPT_BENG`: Bengali
pub const AF_SCRIPT_BENG: AfScript = 5;
/// `AF_SCRIPT_BUHD`: Buhid
pub const AF_SCRIPT_BUHD: AfScript = 6;
/// `AF_SCRIPT_CAKM`: Chakma
pub const AF_SCRIPT_CAKM: AfScript = 7;
/// `AF_SCRIPT_CANS`: Canadian Syllabics
pub const AF_SCRIPT_CANS: AfScript = 8;
/// `AF_SCRIPT_CARI`: Carian
pub const AF_SCRIPT_CARI: AfScript = 9;
/// `AF_SCRIPT_CHER`: Cherokee
pub const AF_SCRIPT_CHER: AfScript = 10;
/// `AF_SCRIPT_COPT`: Coptic
pub const AF_SCRIPT_COPT: AfScript = 11;
/// `AF_SCRIPT_CPRT`: Cypriot
pub const AF_SCRIPT_CPRT: AfScript = 12;
/// `AF_SCRIPT_CYRL`: Cyrillic
pub const AF_SCRIPT_CYRL: AfScript = 13;
/// `AF_SCRIPT_DEVA`: Devanagari
pub const AF_SCRIPT_DEVA: AfScript = 14;
/// `AF_SCRIPT_DSRT`: Deseret
pub const AF_SCRIPT_DSRT: AfScript = 15;
/// `AF_SCRIPT_ETHI`: Ethiopic
pub const AF_SCRIPT_ETHI: AfScript = 16;
/// `AF_SCRIPT_GEOR`: Georgian (Mkhedruli)
pub const AF_SCRIPT_GEOR: AfScript = 17;
/// `AF_SCRIPT_GEOK`: Georgian (Khutsuri)
pub const AF_SCRIPT_GEOK: AfScript = 18;
/// `AF_SCRIPT_GLAG`: Glagolitic
pub const AF_SCRIPT_GLAG: AfScript = 19;
/// `AF_SCRIPT_GOTH`: Gothic
pub const AF_SCRIPT_GOTH: AfScript = 20;
/// `AF_SCRIPT_GREK`: Greek
pub const AF_SCRIPT_GREK: AfScript = 21;
/// `AF_SCRIPT_GUJR`: Gujarati
pub const AF_SCRIPT_GUJR: AfScript = 22;
/// `AF_SCRIPT_GURU`: Gurmukhi
pub const AF_SCRIPT_GURU: AfScript = 23;
/// `AF_SCRIPT_HEBR`: Hebrew
pub const AF_SCRIPT_HEBR: AfScript = 24;
/// `AF_SCRIPT_KALI`: Kayah Li
pub const AF_SCRIPT_KALI: AfScript = 25;
/// `AF_SCRIPT_KHMR`: Khmer
pub const AF_SCRIPT_KHMR: AfScript = 26;
/// `AF_SCRIPT_KHMS`: Khmer Symbols
pub const AF_SCRIPT_KHMS: AfScript = 27;
/// `AF_SCRIPT_KNDA`: Kannada
pub const AF_SCRIPT_KNDA: AfScript = 28;
/// `AF_SCRIPT_LAO`: Lao
pub const AF_SCRIPT_LAO: AfScript = 29;
/// `AF_SCRIPT_LATN`: Latin
pub const AF_SCRIPT_LATN: AfScript = 30;
/// `AF_SCRIPT_LATB`: Latin Subscript Fallback
pub const AF_SCRIPT_LATB: AfScript = 31;
/// `AF_SCRIPT_LATP`: Latin Superscript Fallback
pub const AF_SCRIPT_LATP: AfScript = 32;
/// `AF_SCRIPT_LISU`: Lisu
pub const AF_SCRIPT_LISU: AfScript = 33;
/// `AF_SCRIPT_MLYM`: Malayalam
pub const AF_SCRIPT_MLYM: AfScript = 34;
/// `AF_SCRIPT_MEDF`: Medefaidrin
pub const AF_SCRIPT_MEDF: AfScript = 35;
/// `AF_SCRIPT_MONG`: Mongolian
pub const AF_SCRIPT_MONG: AfScript = 36;
/// `AF_SCRIPT_MYMR`: Myanmar
pub const AF_SCRIPT_MYMR: AfScript = 37;
/// `AF_SCRIPT_NKOO`: N'Ko
pub const AF_SCRIPT_NKOO: AfScript = 38;
/// `AF_SCRIPT_NONE`: no script
pub const AF_SCRIPT_NONE: AfScript = 39;
/// `AF_SCRIPT_OLCK`: Ol Chiki
pub const AF_SCRIPT_OLCK: AfScript = 40;
/// `AF_SCRIPT_ORKH`: Old Turkic
pub const AF_SCRIPT_ORKH: AfScript = 41;
/// `AF_SCRIPT_OSGE`: Osage
pub const AF_SCRIPT_OSGE: AfScript = 42;
/// `AF_SCRIPT_OSMA`: Osmanya
pub const AF_SCRIPT_OSMA: AfScript = 43;
/// `AF_SCRIPT_ROHG`: Hanifi Rohingya
pub const AF_SCRIPT_ROHG: AfScript = 44;
/// `AF_SCRIPT_SAUR`: Saurashtra
pub const AF_SCRIPT_SAUR: AfScript = 45;
/// `AF_SCRIPT_SHAW`: Shavian
pub const AF_SCRIPT_SHAW: AfScript = 46;
/// `AF_SCRIPT_SINH`: Sinhala
pub const AF_SCRIPT_SINH: AfScript = 47;
/// `AF_SCRIPT_SUND`: Sundanese
pub const AF_SCRIPT_SUND: AfScript = 48;
/// `AF_SCRIPT_TAML`: Tamil
pub const AF_SCRIPT_TAML: AfScript = 49;
/// `AF_SCRIPT_TAVT`: Tai Viet
pub const AF_SCRIPT_TAVT: AfScript = 50;
/// `AF_SCRIPT_TELU`: Telugu
pub const AF_SCRIPT_TELU: AfScript = 51;
/// `AF_SCRIPT_TFNG`: Tifinagh
pub const AF_SCRIPT_TFNG: AfScript = 52;
/// `AF_SCRIPT_THAI`: Thai
pub const AF_SCRIPT_THAI: AfScript = 53;
/// `AF_SCRIPT_VAII`: Vai
pub const AF_SCRIPT_VAII: AfScript = 54;
/// `AF_SCRIPT_LIMB`: Limbu
pub const AF_SCRIPT_LIMB: AfScript = 55;
/// `AF_SCRIPT_ORYA`: Oriya
pub const AF_SCRIPT_ORYA: AfScript = 56;
/// `AF_SCRIPT_SYLO`: Syloti Nagri
pub const AF_SCRIPT_SYLO: AfScript = 57;
/// `AF_SCRIPT_TIBT`: Tibetan
pub const AF_SCRIPT_TIBT: AfScript = 58;
/// `AF_SCRIPT_HANI`: CJKV ideographs
pub const AF_SCRIPT_HANI: AfScript = 59;
/// `AF_SCRIPT_MAX`
pub const AF_SCRIPT_MAX: AfScript = 60;

/// `af_script_classes` (with the `af_*_script_class` records of
/// afglobal.c)
#[rustfmt::skip]
pub static AF_SCRIPT_CLASSES: [AfScriptClassRec; 60] = [
    AfScriptClassRec {
        script: AF_SCRIPT_ADLM,
        script_uni_ranges: AF_ADLM_UNIRANGES,
        script_uni_nonbase_ranges: AF_ADLM_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x9E\xA4\x8C \xF0\x9E\xA4\xAE\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_ARAB,
        script_uni_ranges: AF_ARAB_UNIRANGES,
        script_uni_nonbase_ranges: AF_ARAB_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xD9\x84 \xD8\xAD \xD9\x80\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_ARMN,
        script_uni_ranges: AF_ARMN_UNIRANGES,
        script_uni_nonbase_ranges: AF_ARMN_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xD5\xBD \xD5\x8D\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_AVST,
        script_uni_ranges: AF_AVST_UNIRANGES,
        script_uni_nonbase_ranges: AF_AVST_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x90\xAC\x9A\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_BAMU,
        script_uni_ranges: AF_BAMU_UNIRANGES,
        script_uni_nonbase_ranges: AF_BAMU_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xEA\x9B\x81 \xEA\x9B\xAF\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_BENG,
        script_uni_ranges: AF_BENG_UNIRANGES,
        script_uni_nonbase_ranges: AF_BENG_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_TOP_TO_BOTTOM,
        standard_charstring: b"\xE0\xA7\xA6 \xE0\xA7\xAA\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_BUHD,
        script_uni_ranges: AF_BUHD_UNIRANGES,
        script_uni_nonbase_ranges: AF_BUHD_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\x9D\x8B \xE1\x9D\x8F\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_CAKM,
        script_uni_ranges: AF_CAKM_UNIRANGES,
        script_uni_nonbase_ranges: AF_CAKM_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x91\x84\xA4 \xF0\x91\x84\x89 \xF0\x91\x84\x9B\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_CANS,
        script_uni_ranges: AF_CANS_UNIRANGES,
        script_uni_nonbase_ranges: AF_CANS_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\x91\x8C \xE1\x93\x9A\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_CARI,
        script_uni_ranges: AF_CARI_UNIRANGES,
        script_uni_nonbase_ranges: AF_CARI_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x90\x8A\xAB \xF0\x90\x8B\x89\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_CHER,
        script_uni_ranges: AF_CHER_UNIRANGES,
        script_uni_nonbase_ranges: AF_CHER_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\x8E\xA4 \xE1\x8F\x85 \xEA\xAE\x95\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_COPT,
        script_uni_ranges: AF_COPT_UNIRANGES,
        script_uni_nonbase_ranges: AF_COPT_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE2\xB2\x9E \xE2\xB2\x9F\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_CPRT,
        script_uni_ranges: AF_CPRT_UNIRANGES,
        script_uni_nonbase_ranges: AF_CPRT_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x90\xA0\x85 \xF0\x90\xA0\xA3\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_CYRL,
        script_uni_ranges: AF_CYRL_UNIRANGES,
        script_uni_nonbase_ranges: AF_CYRL_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xD0\xBE \xD0\x9E\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_DEVA,
        script_uni_ranges: AF_DEVA_UNIRANGES,
        script_uni_nonbase_ranges: AF_DEVA_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_TOP_TO_BOTTOM,
        standard_charstring: b"\xE0\xA4\xA0 \xE0\xA4\xB5 \xE0\xA4\x9F\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_DSRT,
        script_uni_ranges: AF_DSRT_UNIRANGES,
        script_uni_nonbase_ranges: AF_DSRT_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x90\x90\x84 \xF0\x90\x90\xAC\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_ETHI,
        script_uni_ranges: AF_ETHI_UNIRANGES,
        script_uni_nonbase_ranges: AF_ETHI_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\x8B\x90\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_GEOR,
        script_uni_ranges: AF_GEOR_UNIRANGES,
        script_uni_nonbase_ranges: AF_GEOR_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\x83\x98 \xE1\x83\x94 \xE1\x83\x90 \xE1\xB2\xBF\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_GEOK,
        script_uni_ranges: AF_GEOK_UNIRANGES,
        script_uni_nonbase_ranges: AF_GEOK_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\x82\xB6 \xE1\x82\xB1 \xE2\xB4\x99\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_GLAG,
        script_uni_ranges: AF_GLAG_UNIRANGES,
        script_uni_nonbase_ranges: AF_GLAG_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE2\xB0\x95 \xE2\xB1\x85\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_GOTH,
        script_uni_ranges: AF_GOTH_UNIRANGES,
        script_uni_nonbase_ranges: AF_GOTH_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_TOP_TO_BOTTOM,
        standard_charstring: b"\xF0\x90\x8C\xB4 \xF0\x90\x8C\xBE \xF0\x90\x8D\x83\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_GREK,
        script_uni_ranges: AF_GREK_UNIRANGES,
        script_uni_nonbase_ranges: AF_GREK_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xCE\xBF \xCE\x9F\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_GUJR,
        script_uni_ranges: AF_GUJR_UNIRANGES,
        script_uni_nonbase_ranges: AF_GUJR_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE0\xAA\x9F \xE0\xAB\xA6\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_GURU,
        script_uni_ranges: AF_GURU_UNIRANGES,
        script_uni_nonbase_ranges: AF_GURU_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_TOP_TO_BOTTOM,
        standard_charstring: b"\xE0\xA8\xA0 \xE0\xA8\xB0 \xE0\xA9\xA6\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_HEBR,
        script_uni_ranges: AF_HEBR_UNIRANGES,
        script_uni_nonbase_ranges: AF_HEBR_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xD7\x9D\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_KALI,
        script_uni_ranges: AF_KALI_UNIRANGES,
        script_uni_nonbase_ranges: AF_KALI_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xEA\xA4\x8D \xEA\xA4\x80\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_KHMR,
        script_uni_ranges: AF_KHMR_UNIRANGES,
        script_uni_nonbase_ranges: AF_KHMR_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\x9F\xA0\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_KHMS,
        script_uni_ranges: AF_KHMS_UNIRANGES,
        script_uni_nonbase_ranges: AF_KHMS_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\xA7\xA1 \xE1\xA7\xAA\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_KNDA,
        script_uni_ranges: AF_KNDA_UNIRANGES,
        script_uni_nonbase_ranges: AF_KNDA_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE0\xB3\xA6 \xE0\xB2\xAC\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_LAO,
        script_uni_ranges: AF_LAO_UNIRANGES,
        script_uni_nonbase_ranges: AF_LAO_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE0\xBB\x90\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_LATN,
        script_uni_ranges: AF_LATN_UNIRANGES,
        script_uni_nonbase_ranges: AF_LATN_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"o O 0\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_LATB,
        script_uni_ranges: AF_LATB_UNIRANGES,
        script_uni_nonbase_ranges: AF_LATB_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE2\x82\x92 \xE2\x82\x80\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_LATP,
        script_uni_ranges: AF_LATP_UNIRANGES,
        script_uni_nonbase_ranges: AF_LATP_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\xB5\x92 \xE1\xB4\xBC \xE2\x81\xB0\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_LISU,
        script_uni_ranges: AF_LISU_UNIRANGES,
        script_uni_nonbase_ranges: AF_LISU_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xEA\x93\xB3\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_MLYM,
        script_uni_ranges: AF_MLYM_UNIRANGES,
        script_uni_nonbase_ranges: AF_MLYM_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE0\xB4\xA0 \xE0\xB4\xB1\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_MEDF,
        script_uni_ranges: AF_MEDF_UNIRANGES,
        script_uni_nonbase_ranges: AF_MEDF_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x96\xB9\xA1 \xF0\x96\xB9\x9B \xF0\x96\xB9\xAF\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_MONG,
        script_uni_ranges: AF_MONG_UNIRANGES,
        script_uni_nonbase_ranges: AF_MONG_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_TOP_TO_BOTTOM,
        standard_charstring: b"\xE1\xA1\x82 \xE1\xA0\xAA\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_MYMR,
        script_uni_ranges: AF_MYMR_UNIRANGES,
        script_uni_nonbase_ranges: AF_MYMR_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\x80\x9D \xE1\x80\x84 \xE1\x80\x82\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_NKOO,
        script_uni_ranges: AF_NKOO_UNIRANGES,
        script_uni_nonbase_ranges: AF_NKOO_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xDF\x8B \xDF\x80\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_NONE,
        script_uni_ranges: AF_NONE_UNIRANGES,
        script_uni_nonbase_ranges: AF_NONE_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_OLCK,
        script_uni_ranges: AF_OLCK_UNIRANGES,
        script_uni_nonbase_ranges: AF_OLCK_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\xB1\x9B\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_ORKH,
        script_uni_ranges: AF_ORKH_UNIRANGES,
        script_uni_nonbase_ranges: AF_ORKH_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x90\xB0\x97\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_OSGE,
        script_uni_ranges: AF_OSGE_UNIRANGES,
        script_uni_nonbase_ranges: AF_OSGE_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x90\x93\x82 \xF0\x90\x93\xAA\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_OSMA,
        script_uni_ranges: AF_OSMA_UNIRANGES,
        script_uni_nonbase_ranges: AF_OSMA_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x90\x92\x86 \xF0\x90\x92\xA0\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_ROHG,
        script_uni_ranges: AF_ROHG_UNIRANGES,
        script_uni_nonbase_ranges: AF_ROHG_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x90\xB4\xB0\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_SAUR,
        script_uni_ranges: AF_SAUR_UNIRANGES,
        script_uni_nonbase_ranges: AF_SAUR_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xEA\xA2\x9D \xEA\xA3\x90\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_SHAW,
        script_uni_ranges: AF_SHAW_UNIRANGES,
        script_uni_nonbase_ranges: AF_SHAW_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xF0\x90\x91\xB4\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_SINH,
        script_uni_ranges: AF_SINH_UNIRANGES,
        script_uni_nonbase_ranges: AF_SINH_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE0\xB6\xA7\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_SUND,
        script_uni_ranges: AF_SUND_UNIRANGES,
        script_uni_nonbase_ranges: AF_SUND_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE1\xAE\xB0\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_TAML,
        script_uni_ranges: AF_TAML_UNIRANGES,
        script_uni_nonbase_ranges: AF_TAML_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE0\xAF\xA6\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_TAVT,
        script_uni_ranges: AF_TAVT_UNIRANGES,
        script_uni_nonbase_ranges: AF_TAVT_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xEA\xAA\x92 \xEA\xAA\xAB\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_TELU,
        script_uni_ranges: AF_TELU_UNIRANGES,
        script_uni_nonbase_ranges: AF_TELU_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE0\xB1\xA6 \xE0\xB1\xA7\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_TFNG,
        script_uni_ranges: AF_TFNG_UNIRANGES,
        script_uni_nonbase_ranges: AF_TFNG_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE2\xB5\x94\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_THAI,
        script_uni_ranges: AF_THAI_UNIRANGES,
        script_uni_nonbase_ranges: AF_THAI_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE0\xB8\xB2 \xE0\xB9\x85 \xE0\xB9\x90\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_VAII,
        script_uni_ranges: AF_VAII_UNIRANGES,
        script_uni_nonbase_ranges: AF_VAII_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xEA\x98\x93 \xEA\x96\x9C \xEA\x96\xB4\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_LIMB,
        script_uni_ranges: AF_LIMB_UNIRANGES,
        script_uni_nonbase_ranges: AF_LIMB_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"o\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_ORYA,
        script_uni_ranges: AF_ORYA_UNIRANGES,
        script_uni_nonbase_ranges: AF_ORYA_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"o\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_SYLO,
        script_uni_ranges: AF_SYLO_UNIRANGES,
        script_uni_nonbase_ranges: AF_SYLO_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"o\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_TIBT,
        script_uni_ranges: AF_TIBT_UNIRANGES,
        script_uni_nonbase_ranges: AF_TIBT_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"o\0",
    },
    AfScriptClassRec {
        script: AF_SCRIPT_HANI,
        script_uni_ranges: AF_HANI_UNIRANGES,
        script_uni_nonbase_ranges: AF_HANI_NONBASE_UNIRANGES,
        top_to_bottom_hinting: AF_HINTING_BOTTOM_TO_TOP,
        standard_charstring: b"\xE7\x94\xB0 \xE5\x9B\x97\0",
    },
];
