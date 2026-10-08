// Rust translation of src/autofit/afstyles.h (with the style classes of
// afglobal.c) from FreeType (2.13.2, as SDL_ttf's external/freetype pins
// it), with `AF_CONFIG_OPTION_CJK` and `AF_CONFIG_OPTION_INDIC` defined.
// Copyright (C) 2013-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Auto-fitter styles.

use super::afblue::*;
use super::afscript::*;
use super::aftypes::*;

/// `AF_STYLE_ADLM_DFLT`: Adlam default style
pub const AF_STYLE_ADLM_DFLT: AfStyle = 0;
/// `AF_STYLE_ARAB_DFLT`: Arabic default style
pub const AF_STYLE_ARAB_DFLT: AfStyle = 1;
/// `AF_STYLE_ARMN_DFLT`: Armenian default style
pub const AF_STYLE_ARMN_DFLT: AfStyle = 2;
/// `AF_STYLE_AVST_DFLT`: Avestan default style
pub const AF_STYLE_AVST_DFLT: AfStyle = 3;
/// `AF_STYLE_BAMU_DFLT`: Bamum default style
pub const AF_STYLE_BAMU_DFLT: AfStyle = 4;
/// `AF_STYLE_BENG_DFLT`: Bengali default style
pub const AF_STYLE_BENG_DFLT: AfStyle = 5;
/// `AF_STYLE_BUHD_DFLT`: Buhid default style
pub const AF_STYLE_BUHD_DFLT: AfStyle = 6;
/// `AF_STYLE_CAKM_DFLT`: Chakma default style
pub const AF_STYLE_CAKM_DFLT: AfStyle = 7;
/// `AF_STYLE_CANS_DFLT`: Canadian Syllabics default style
pub const AF_STYLE_CANS_DFLT: AfStyle = 8;
/// `AF_STYLE_CARI_DFLT`: Carian default style
pub const AF_STYLE_CARI_DFLT: AfStyle = 9;
/// `AF_STYLE_CHER_DFLT`: Cherokee default style
pub const AF_STYLE_CHER_DFLT: AfStyle = 10;
/// `AF_STYLE_COPT_DFLT`: Coptic default style
pub const AF_STYLE_COPT_DFLT: AfStyle = 11;
/// `AF_STYLE_CPRT_DFLT`: Cypriot default style
pub const AF_STYLE_CPRT_DFLT: AfStyle = 12;
/// `AF_STYLE_CYRL_C2CP`: Cyrillic petite capitals from capitals style
pub const AF_STYLE_CYRL_C2CP: AfStyle = 13;
/// `AF_STYLE_CYRL_C2SC`: Cyrillic small capitals from capitals style
pub const AF_STYLE_CYRL_C2SC: AfStyle = 14;
/// `AF_STYLE_CYRL_ORDN`: Cyrillic ordinals style
pub const AF_STYLE_CYRL_ORDN: AfStyle = 15;
/// `AF_STYLE_CYRL_PCAP`: Cyrillic petite capitals style
pub const AF_STYLE_CYRL_PCAP: AfStyle = 16;
/// `AF_STYLE_CYRL_SINF`: Cyrillic scientific inferiors style
pub const AF_STYLE_CYRL_SINF: AfStyle = 17;
/// `AF_STYLE_CYRL_SMCP`: Cyrillic small capitals style
pub const AF_STYLE_CYRL_SMCP: AfStyle = 18;
/// `AF_STYLE_CYRL_SUBS`: Cyrillic subscript style
pub const AF_STYLE_CYRL_SUBS: AfStyle = 19;
/// `AF_STYLE_CYRL_SUPS`: Cyrillic superscript style
pub const AF_STYLE_CYRL_SUPS: AfStyle = 20;
/// `AF_STYLE_CYRL_TITL`: Cyrillic titling style
pub const AF_STYLE_CYRL_TITL: AfStyle = 21;
/// `AF_STYLE_CYRL_DFLT`: Cyrillic default style
pub const AF_STYLE_CYRL_DFLT: AfStyle = 22;
/// `AF_STYLE_DEVA_DFLT`: Devanagari default style
pub const AF_STYLE_DEVA_DFLT: AfStyle = 23;
/// `AF_STYLE_DSRT_DFLT`: Deseret default style
pub const AF_STYLE_DSRT_DFLT: AfStyle = 24;
/// `AF_STYLE_ETHI_DFLT`: Ethiopic default style
pub const AF_STYLE_ETHI_DFLT: AfStyle = 25;
/// `AF_STYLE_GEOR_DFLT`: Georgian (Mkhedruli) default style
pub const AF_STYLE_GEOR_DFLT: AfStyle = 26;
/// `AF_STYLE_GEOK_DFLT`: Georgian (Khutsuri) default style
pub const AF_STYLE_GEOK_DFLT: AfStyle = 27;
/// `AF_STYLE_GLAG_DFLT`: Glagolitic default style
pub const AF_STYLE_GLAG_DFLT: AfStyle = 28;
/// `AF_STYLE_GOTH_DFLT`: Gothic default style
pub const AF_STYLE_GOTH_DFLT: AfStyle = 29;
/// `AF_STYLE_GREK_C2CP`: Greek petite capitals from capitals style
pub const AF_STYLE_GREK_C2CP: AfStyle = 30;
/// `AF_STYLE_GREK_C2SC`: Greek small capitals from capitals style
pub const AF_STYLE_GREK_C2SC: AfStyle = 31;
/// `AF_STYLE_GREK_ORDN`: Greek ordinals style
pub const AF_STYLE_GREK_ORDN: AfStyle = 32;
/// `AF_STYLE_GREK_PCAP`: Greek petite capitals style
pub const AF_STYLE_GREK_PCAP: AfStyle = 33;
/// `AF_STYLE_GREK_SINF`: Greek scientific inferiors style
pub const AF_STYLE_GREK_SINF: AfStyle = 34;
/// `AF_STYLE_GREK_SMCP`: Greek small capitals style
pub const AF_STYLE_GREK_SMCP: AfStyle = 35;
/// `AF_STYLE_GREK_SUBS`: Greek subscript style
pub const AF_STYLE_GREK_SUBS: AfStyle = 36;
/// `AF_STYLE_GREK_SUPS`: Greek superscript style
pub const AF_STYLE_GREK_SUPS: AfStyle = 37;
/// `AF_STYLE_GREK_TITL`: Greek titling style
pub const AF_STYLE_GREK_TITL: AfStyle = 38;
/// `AF_STYLE_GREK_DFLT`: Greek default style
pub const AF_STYLE_GREK_DFLT: AfStyle = 39;
/// `AF_STYLE_GUJR_DFLT`: Gujarati default style
pub const AF_STYLE_GUJR_DFLT: AfStyle = 40;
/// `AF_STYLE_GURU_DFLT`: Gurmukhi default style
pub const AF_STYLE_GURU_DFLT: AfStyle = 41;
/// `AF_STYLE_HEBR_DFLT`: Hebrew default style
pub const AF_STYLE_HEBR_DFLT: AfStyle = 42;
/// `AF_STYLE_KALI_DFLT`: Kayah Li default style
pub const AF_STYLE_KALI_DFLT: AfStyle = 43;
/// `AF_STYLE_KHMR_DFLT`: Khmer default style
pub const AF_STYLE_KHMR_DFLT: AfStyle = 44;
/// `AF_STYLE_KHMS_DFLT`: Khmer Symbols default style
pub const AF_STYLE_KHMS_DFLT: AfStyle = 45;
/// `AF_STYLE_KNDA_DFLT`: Kannada default style
pub const AF_STYLE_KNDA_DFLT: AfStyle = 46;
/// `AF_STYLE_LAO_DFLT`: Lao default style
pub const AF_STYLE_LAO_DFLT: AfStyle = 47;
/// `AF_STYLE_LATN_C2CP`: Latin petite capitals from capitals style
pub const AF_STYLE_LATN_C2CP: AfStyle = 48;
/// `AF_STYLE_LATN_C2SC`: Latin small capitals from capitals style
pub const AF_STYLE_LATN_C2SC: AfStyle = 49;
/// `AF_STYLE_LATN_ORDN`: Latin ordinals style
pub const AF_STYLE_LATN_ORDN: AfStyle = 50;
/// `AF_STYLE_LATN_PCAP`: Latin petite capitals style
pub const AF_STYLE_LATN_PCAP: AfStyle = 51;
/// `AF_STYLE_LATN_SINF`: Latin scientific inferiors style
pub const AF_STYLE_LATN_SINF: AfStyle = 52;
/// `AF_STYLE_LATN_SMCP`: Latin small capitals style
pub const AF_STYLE_LATN_SMCP: AfStyle = 53;
/// `AF_STYLE_LATN_SUBS`: Latin subscript style
pub const AF_STYLE_LATN_SUBS: AfStyle = 54;
/// `AF_STYLE_LATN_SUPS`: Latin superscript style
pub const AF_STYLE_LATN_SUPS: AfStyle = 55;
/// `AF_STYLE_LATN_TITL`: Latin titling style
pub const AF_STYLE_LATN_TITL: AfStyle = 56;
/// `AF_STYLE_LATN_DFLT`: Latin default style
pub const AF_STYLE_LATN_DFLT: AfStyle = 57;
/// `AF_STYLE_LATB_DFLT`: Latin subscript fallback default style
pub const AF_STYLE_LATB_DFLT: AfStyle = 58;
/// `AF_STYLE_LATP_DFLT`: Latin superscript fallback default style
pub const AF_STYLE_LATP_DFLT: AfStyle = 59;
/// `AF_STYLE_LISU_DFLT`: Lisu default style
pub const AF_STYLE_LISU_DFLT: AfStyle = 60;
/// `AF_STYLE_MLYM_DFLT`: Malayalam default style
pub const AF_STYLE_MLYM_DFLT: AfStyle = 61;
/// `AF_STYLE_MEDF_DFLT`: Medefaidrin default style
pub const AF_STYLE_MEDF_DFLT: AfStyle = 62;
/// `AF_STYLE_MONG_DFLT`: Mongolian default style
pub const AF_STYLE_MONG_DFLT: AfStyle = 63;
/// `AF_STYLE_MYMR_DFLT`: Myanmar default style
pub const AF_STYLE_MYMR_DFLT: AfStyle = 64;
/// `AF_STYLE_NKOO_DFLT`: N'Ko default style
pub const AF_STYLE_NKOO_DFLT: AfStyle = 65;
/// `AF_STYLE_NONE_DFLT`: no style
pub const AF_STYLE_NONE_DFLT: AfStyle = 66;
/// `AF_STYLE_OLCK_DFLT`: Ol Chiki default style
pub const AF_STYLE_OLCK_DFLT: AfStyle = 67;
/// `AF_STYLE_ORKH_DFLT`: Old Turkic default style
pub const AF_STYLE_ORKH_DFLT: AfStyle = 68;
/// `AF_STYLE_OSGE_DFLT`: Osage default style
pub const AF_STYLE_OSGE_DFLT: AfStyle = 69;
/// `AF_STYLE_OSMA_DFLT`: Osmanya default style
pub const AF_STYLE_OSMA_DFLT: AfStyle = 70;
/// `AF_STYLE_ROHG_DFLT`: Hanifi Rohingya default style
pub const AF_STYLE_ROHG_DFLT: AfStyle = 71;
/// `AF_STYLE_SAUR_DFLT`: Saurashtra default style
pub const AF_STYLE_SAUR_DFLT: AfStyle = 72;
/// `AF_STYLE_SHAW_DFLT`: Shavian default style
pub const AF_STYLE_SHAW_DFLT: AfStyle = 73;
/// `AF_STYLE_SINH_DFLT`: Sinhala default style
pub const AF_STYLE_SINH_DFLT: AfStyle = 74;
/// `AF_STYLE_SUND_DFLT`: Sundanese default style
pub const AF_STYLE_SUND_DFLT: AfStyle = 75;
/// `AF_STYLE_TAML_DFLT`: Tamil default style
pub const AF_STYLE_TAML_DFLT: AfStyle = 76;
/// `AF_STYLE_TAVT_DFLT`: Tai Viet default style
pub const AF_STYLE_TAVT_DFLT: AfStyle = 77;
/// `AF_STYLE_TELU_DFLT`: Telugu default style
pub const AF_STYLE_TELU_DFLT: AfStyle = 78;
/// `AF_STYLE_TFNG_DFLT`: Tifinagh default style
pub const AF_STYLE_TFNG_DFLT: AfStyle = 79;
/// `AF_STYLE_THAI_DFLT`: Thai default style
pub const AF_STYLE_THAI_DFLT: AfStyle = 80;
/// `AF_STYLE_VAII_DFLT`: Vai default style
pub const AF_STYLE_VAII_DFLT: AfStyle = 81;
/// `AF_STYLE_LIMB_DFLT`: Limbu default style
pub const AF_STYLE_LIMB_DFLT: AfStyle = 82;
/// `AF_STYLE_ORYA_DFLT`: Oriya default style
pub const AF_STYLE_ORYA_DFLT: AfStyle = 83;
/// `AF_STYLE_SYLO_DFLT`: Syloti Nagri default style
pub const AF_STYLE_SYLO_DFLT: AfStyle = 84;
/// `AF_STYLE_TIBT_DFLT`: Tibetan default style
pub const AF_STYLE_TIBT_DFLT: AfStyle = 85;
/// `AF_STYLE_HANI_DFLT`: CJKV ideographs default style
pub const AF_STYLE_HANI_DFLT: AfStyle = 86;
/// `AF_STYLE_MAX`
pub const AF_STYLE_MAX: AfStyle = 87;

/// `af_style_classes` (with the `af_*_style_class` records of afglobal.c)
#[rustfmt::skip]
pub static AF_STYLE_CLASSES: [AfStyleClassRec; 87] = [
    AfStyleClassRec {
        style: AF_STYLE_ADLM_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_ADLM,
        blue_stringset: AF_BLUE_STRINGSET_ADLM,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_ARAB_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_ARAB,
        blue_stringset: AF_BLUE_STRINGSET_ARAB,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_ARMN_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_ARMN,
        blue_stringset: AF_BLUE_STRINGSET_ARMN,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_AVST_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_AVST,
        blue_stringset: AF_BLUE_STRINGSET_AVST,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_BAMU_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_BAMU,
        blue_stringset: AF_BLUE_STRINGSET_BAMU,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_BENG_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_BENG,
        blue_stringset: AF_BLUE_STRINGSET_BENG,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_BUHD_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_BUHD,
        blue_stringset: AF_BLUE_STRINGSET_BUHD,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_CAKM_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CAKM,
        blue_stringset: AF_BLUE_STRINGSET_CAKM,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_CANS_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CANS,
        blue_stringset: AF_BLUE_STRINGSET_CANS,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_CARI_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CARI,
        blue_stringset: AF_BLUE_STRINGSET_CARI,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_CHER_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CHER,
        blue_stringset: AF_BLUE_STRINGSET_CHER,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_COPT_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_COPT,
        blue_stringset: AF_BLUE_STRINGSET_COPT,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_CPRT_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CPRT,
        blue_stringset: AF_BLUE_STRINGSET_CPRT,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_C2CP,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_PETITE_CAPITALS_FROM_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_C2SC,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_SMALL_CAPITALS_FROM_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_ORDN,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_ORDINALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_PCAP,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_PETITE_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_SINF,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_SCIENTIFIC_INFERIORS,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_SMCP,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_SMALL_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_SUBS,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_SUBSCRIPT,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_SUPS,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_SUPERSCRIPT,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_TITL,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_TITLING,
    },
    AfStyleClassRec {
        style: AF_STYLE_CYRL_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_CYRL,
        blue_stringset: AF_BLUE_STRINGSET_CYRL,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_DEVA_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_DEVA,
        blue_stringset: AF_BLUE_STRINGSET_DEVA,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_DSRT_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_DSRT,
        blue_stringset: AF_BLUE_STRINGSET_DSRT,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_ETHI_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_ETHI,
        blue_stringset: AF_BLUE_STRINGSET_ETHI,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_GEOR_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GEOR,
        blue_stringset: AF_BLUE_STRINGSET_GEOR,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_GEOK_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GEOK,
        blue_stringset: AF_BLUE_STRINGSET_GEOK,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_GLAG_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GLAG,
        blue_stringset: AF_BLUE_STRINGSET_GLAG,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_GOTH_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GOTH,
        blue_stringset: AF_BLUE_STRINGSET_GOTH,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_C2CP,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_PETITE_CAPITALS_FROM_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_C2SC,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_SMALL_CAPITALS_FROM_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_ORDN,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_ORDINALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_PCAP,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_PETITE_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_SINF,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_SCIENTIFIC_INFERIORS,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_SMCP,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_SMALL_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_SUBS,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_SUBSCRIPT,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_SUPS,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_SUPERSCRIPT,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_TITL,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_TITLING,
    },
    AfStyleClassRec {
        style: AF_STYLE_GREK_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GREK,
        blue_stringset: AF_BLUE_STRINGSET_GREK,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_GUJR_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GUJR,
        blue_stringset: AF_BLUE_STRINGSET_GUJR,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_GURU_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_GURU,
        blue_stringset: AF_BLUE_STRINGSET_GURU,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_HEBR_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_HEBR,
        blue_stringset: AF_BLUE_STRINGSET_HEBR,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_KALI_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_KALI,
        blue_stringset: AF_BLUE_STRINGSET_KALI,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_KHMR_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_KHMR,
        blue_stringset: AF_BLUE_STRINGSET_KHMR,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_KHMS_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_KHMS,
        blue_stringset: AF_BLUE_STRINGSET_KHMS,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_KNDA_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_KNDA,
        blue_stringset: AF_BLUE_STRINGSET_KNDA,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_LAO_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LAO,
        blue_stringset: AF_BLUE_STRINGSET_LAO,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_C2CP,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_PETITE_CAPITALS_FROM_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_C2SC,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_SMALL_CAPITALS_FROM_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_ORDN,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_ORDINALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_PCAP,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_PETITE_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_SINF,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_SCIENTIFIC_INFERIORS,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_SMCP,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_SMALL_CAPITALS,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_SUBS,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_SUBSCRIPT,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_SUPS,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_SUPERSCRIPT,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_TITL,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_TITLING,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATN_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATN,
        blue_stringset: AF_BLUE_STRINGSET_LATN,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATB_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATB,
        blue_stringset: AF_BLUE_STRINGSET_LATB,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_LATP_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LATP,
        blue_stringset: AF_BLUE_STRINGSET_LATP,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_LISU_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_LISU,
        blue_stringset: AF_BLUE_STRINGSET_LISU,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_MLYM_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_MLYM,
        blue_stringset: AF_BLUE_STRINGSET_MLYM,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_MEDF_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_MEDF,
        blue_stringset: AF_BLUE_STRINGSET_MEDF,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_MONG_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_MONG,
        blue_stringset: AF_BLUE_STRINGSET_MONG,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_MYMR_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_MYMR,
        blue_stringset: AF_BLUE_STRINGSET_MYMR,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_NKOO_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_NKOO,
        blue_stringset: AF_BLUE_STRINGSET_NKOO,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_NONE_DFLT,
        writing_system: AF_WRITING_SYSTEM_DUMMY,
        script: AF_SCRIPT_NONE,
        blue_stringset: AF_BLUE_STRINGSET_NONE,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_OLCK_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_OLCK,
        blue_stringset: AF_BLUE_STRINGSET_OLCK,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_ORKH_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_ORKH,
        blue_stringset: AF_BLUE_STRINGSET_ORKH,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_OSGE_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_OSGE,
        blue_stringset: AF_BLUE_STRINGSET_OSGE,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_OSMA_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_OSMA,
        blue_stringset: AF_BLUE_STRINGSET_OSMA,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_ROHG_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_ROHG,
        blue_stringset: AF_BLUE_STRINGSET_ROHG,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_SAUR_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_SAUR,
        blue_stringset: AF_BLUE_STRINGSET_SAUR,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_SHAW_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_SHAW,
        blue_stringset: AF_BLUE_STRINGSET_SHAW,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_SINH_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_SINH,
        blue_stringset: AF_BLUE_STRINGSET_SINH,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_SUND_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_SUND,
        blue_stringset: AF_BLUE_STRINGSET_SUND,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_TAML_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_TAML,
        blue_stringset: AF_BLUE_STRINGSET_TAML,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_TAVT_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_TAVT,
        blue_stringset: AF_BLUE_STRINGSET_TAVT,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_TELU_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_TELU,
        blue_stringset: AF_BLUE_STRINGSET_TELU,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_TFNG_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_TFNG,
        blue_stringset: AF_BLUE_STRINGSET_TFNG,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_THAI_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_THAI,
        blue_stringset: AF_BLUE_STRINGSET_THAI,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_VAII_DFLT,
        writing_system: AF_WRITING_SYSTEM_LATIN,
        script: AF_SCRIPT_VAII,
        blue_stringset: AF_BLUE_STRINGSET_VAII,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_LIMB_DFLT,
        writing_system: AF_WRITING_SYSTEM_INDIC,
        script: AF_SCRIPT_LIMB,
        blue_stringset: 0,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_ORYA_DFLT,
        writing_system: AF_WRITING_SYSTEM_INDIC,
        script: AF_SCRIPT_ORYA,
        blue_stringset: 0,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_SYLO_DFLT,
        writing_system: AF_WRITING_SYSTEM_INDIC,
        script: AF_SCRIPT_SYLO,
        blue_stringset: 0,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_TIBT_DFLT,
        writing_system: AF_WRITING_SYSTEM_INDIC,
        script: AF_SCRIPT_TIBT,
        blue_stringset: 0,
        coverage: AF_COVERAGE_DEFAULT,
    },
    AfStyleClassRec {
        style: AF_STYLE_HANI_DFLT,
        writing_system: AF_WRITING_SYSTEM_CJK,
        script: AF_SCRIPT_HANI,
        blue_stringset: AF_BLUE_STRINGSET_HANI,
        coverage: AF_COVERAGE_DEFAULT,
    },
];
