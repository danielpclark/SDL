// Rust translation of include/freetype/tttables.h, tttags.h and ttnameid.h
// from FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Basic SFNT/TrueType tables definitions and interface, the table tags
//! and the name table identifiers.

#![allow(non_snake_case, non_upper_case_globals)]

use super::fttypes::*;

/// `TT_Header`: the `head` table.
#[derive(Debug, Clone, Copy, Default)]
pub struct TtHeader {
    pub Table_Version: FtFixed,
    pub Font_Revision: FtFixed,
    pub CheckSum_Adjust: FtLong,
    pub Magic_Number: FtLong,
    pub Flags: FtUShort,
    pub Units_Per_EM: FtUShort,
    pub Created: [FtULong; 2],
    pub Modified: [FtULong; 2],
    pub xMin: FtShort,
    pub yMin: FtShort,
    pub xMax: FtShort,
    pub yMax: FtShort,
    pub Mac_Style: FtUShort,
    pub Lowest_Rec_PPEM: FtUShort,
    pub Font_Direction: FtShort,
    pub Index_To_Loc_Format: FtShort,
    pub Glyph_Data_Format: FtShort,
}

/// `TT_HoriHeader`: the `hhea` table (its `long_metrics` and
/// `short_metrics` pointers are unused upstream and left out).
#[derive(Debug, Clone, Copy, Default)]
pub struct TtHoriHeader {
    pub Version: FtFixed,
    pub Ascender: FtShort,
    pub Descender: FtShort,
    pub Line_Gap: FtShort,
    pub advance_Width_Max: FtUShort,
    pub min_Left_Side_Bearing: FtShort,
    pub min_Right_Side_Bearing: FtShort,
    pub xMax_Extent: FtShort,
    pub caret_Slope_Rise: FtShort,
    pub caret_Slope_Run: FtShort,
    pub caret_Offset: FtShort,
    pub Reserved: [FtShort; 4],
    pub metric_Data_Format: FtShort,
    pub number_Of_HMetrics: FtUShort,
}

/// `TT_VertHeader`: the `vhea` table.
#[derive(Debug, Clone, Copy, Default)]
pub struct TtVertHeader {
    pub Version: FtFixed,
    pub Ascender: FtShort,
    pub Descender: FtShort,
    pub Line_Gap: FtShort,
    pub advance_Height_Max: FtUShort,
    pub min_Top_Side_Bearing: FtShort,
    pub min_Bottom_Side_Bearing: FtShort,
    pub yMax_Extent: FtShort,
    pub caret_Slope_Rise: FtShort,
    pub caret_Slope_Run: FtShort,
    pub caret_Offset: FtShort,
    pub Reserved: [FtShort; 4],
    pub metric_Data_Format: FtShort,
    pub number_Of_VMetrics: FtUShort,
}

/// `TT_OS2`: the `OS/2` table.
#[derive(Debug, Clone, Copy, Default)]
pub struct TtOs2 {
    pub version: FtUShort, /* 0x0001 - more or 0xFFFF */
    pub xAvgCharWidth: FtShort,
    pub usWeightClass: FtUShort,
    pub usWidthClass: FtUShort,
    pub fsType: FtUShort,
    pub ySubscriptXSize: FtShort,
    pub ySubscriptYSize: FtShort,
    pub ySubscriptXOffset: FtShort,
    pub ySubscriptYOffset: FtShort,
    pub ySuperscriptXSize: FtShort,
    pub ySuperscriptYSize: FtShort,
    pub ySuperscriptXOffset: FtShort,
    pub ySuperscriptYOffset: FtShort,
    pub yStrikeoutSize: FtShort,
    pub yStrikeoutPosition: FtShort,
    pub sFamilyClass: FtShort,

    pub panose: [FtByte; 10],

    pub ulUnicodeRange1: FtULong, /* Bits 0-31   */
    pub ulUnicodeRange2: FtULong, /* Bits 32-63  */
    pub ulUnicodeRange3: FtULong, /* Bits 64-95  */
    pub ulUnicodeRange4: FtULong, /* Bits 96-127 */

    pub achVendID: [FtChar; 4],

    pub fsSelection: FtUShort,
    pub usFirstCharIndex: FtUShort,
    pub usLastCharIndex: FtUShort,
    pub sTypoAscender: FtShort,
    pub sTypoDescender: FtShort,
    pub sTypoLineGap: FtShort,
    pub usWinAscent: FtUShort,
    pub usWinDescent: FtUShort,

    /* only version 1 and higher: */
    pub ulCodePageRange1: FtULong, /* Bits 0-31   */
    pub ulCodePageRange2: FtULong, /* Bits 32-63  */

    /* only version 2 and higher: */
    pub sxHeight: FtShort,
    pub sCapHeight: FtShort,
    pub usDefaultChar: FtUShort,
    pub usBreakChar: FtUShort,
    pub usMaxContext: FtUShort,

    /* only version 5 and higher: */
    pub usLowerOpticalPointSize: FtUShort, /* in twips (1/20th points) */
    pub usUpperOpticalPointSize: FtUShort, /* in twips (1/20th points) */
}

/// `TT_Postscript`: the `post` table header.
#[derive(Debug, Clone, Copy, Default)]
pub struct TtPostscript {
    pub FormatType: FtFixed,
    pub italicAngle: FtFixed,
    pub underlinePosition: FtShort,
    pub underlineThickness: FtShort,
    pub isFixedPitch: FtULong,
    pub minMemType42: FtULong,
    pub maxMemType42: FtULong,
    pub minMemType1: FtULong,
    pub maxMemType1: FtULong,
    /* Glyph names follow in the `post' table, but we don't */
    /* load them by default.                                */
}

/// `TT_PCLT`: the `PCLT` table.
#[derive(Debug, Clone, Copy, Default)]
pub struct TtPclt {
    pub Version: FtFixed,
    pub FontNumber: FtULong,
    pub Pitch: FtUShort,
    pub xHeight: FtUShort,
    pub Style: FtUShort,
    pub TypeFamily: FtUShort,
    pub CapHeight: FtUShort,
    pub SymbolSet: FtUShort,
    pub TypeFace: [FtChar; 16],
    pub CharacterComplement: [FtChar; 8],
    pub FileName: [FtChar; 6],
    pub StrokeWeight: FtChar,
    pub WidthType: FtChar,
    pub SerifStyle: FtByte,
    pub Reserved: FtByte,
}

/// `TT_MaxProfile`: the `maxp` table.
#[derive(Debug, Clone, Copy, Default)]
pub struct TtMaxProfile {
    pub version: FtFixed,
    pub numGlyphs: FtUShort,
    pub maxPoints: FtUShort,
    pub maxContours: FtUShort,
    pub maxCompositePoints: FtUShort,
    pub maxCompositeContours: FtUShort,
    pub maxZones: FtUShort,
    pub maxTwilightPoints: FtUShort,
    pub maxStorage: FtUShort,
    pub maxFunctionDefs: FtUShort,
    pub maxInstructionDefs: FtUShort,
    pub maxStackElements: FtUShort,
    pub maxSizeOfInstructions: FtUShort,
    pub maxComponentElements: FtUShort,
    pub maxComponentDepth: FtUShort,
}

/// `FT_Sfnt_Tag`
pub type FtSfntTag = u32;
pub const FT_SFNT_HEAD: FtSfntTag = 0;
pub const FT_SFNT_MAXP: FtSfntTag = 1;
pub const FT_SFNT_OS2: FtSfntTag = 2;
pub const FT_SFNT_HHEA: FtSfntTag = 3;
pub const FT_SFNT_VHEA: FtSfntTag = 4;
pub const FT_SFNT_POST: FtSfntTag = 5;
pub const FT_SFNT_PCLT: FtSfntTag = 6;
pub const FT_SFNT_MAX: FtSfntTag = 7;

/// A table returned by `FT_Get_Sfnt_Table` (C's `void*`, typed).
#[derive(Debug, Clone, Copy)]
pub enum FtSfntTable<'a> {
    Head(&'a TtHeader),
    Maxp(&'a TtMaxProfile),
    Os2(&'a TtOs2),
    Hhea(&'a TtHoriHeader),
    Vhea(&'a TtVertHeader),
    Post(&'a TtPostscript),
    Pclt(&'a TtPclt),
}

/* tttags.h */

pub const TTAG_avar: u32 = ft_make_tag(b'a', b'v', b'a', b'r');
pub const TTAG_BASE: u32 = ft_make_tag(b'B', b'A', b'S', b'E');
pub const TTAG_bdat: u32 = ft_make_tag(b'b', b'd', b'a', b't');
pub const TTAG_BDF: u32 = ft_make_tag(b'B', b'D', b'F', b' ');
pub const TTAG_bhed: u32 = ft_make_tag(b'b', b'h', b'e', b'd');
pub const TTAG_bloc: u32 = ft_make_tag(b'b', b'l', b'o', b'c');
pub const TTAG_bsln: u32 = ft_make_tag(b'b', b's', b'l', b'n');
pub const TTAG_CBDT: u32 = ft_make_tag(b'C', b'B', b'D', b'T');
pub const TTAG_CBLC: u32 = ft_make_tag(b'C', b'B', b'L', b'C');
pub const TTAG_CFF: u32 = ft_make_tag(b'C', b'F', b'F', b' ');
pub const TTAG_CFF2: u32 = ft_make_tag(b'C', b'F', b'F', b'2');
pub const TTAG_CID: u32 = ft_make_tag(b'C', b'I', b'D', b' ');
pub const TTAG_cmap: u32 = ft_make_tag(b'c', b'm', b'a', b'p');
pub const TTAG_COLR: u32 = ft_make_tag(b'C', b'O', b'L', b'R');
pub const TTAG_CPAL: u32 = ft_make_tag(b'C', b'P', b'A', b'L');
pub const TTAG_cvar: u32 = ft_make_tag(b'c', b'v', b'a', b'r');
pub const TTAG_cvt: u32 = ft_make_tag(b'c', b'v', b't', b' ');
pub const TTAG_DSIG: u32 = ft_make_tag(b'D', b'S', b'I', b'G');
pub const TTAG_EBDT: u32 = ft_make_tag(b'E', b'B', b'D', b'T');
pub const TTAG_EBLC: u32 = ft_make_tag(b'E', b'B', b'L', b'C');
pub const TTAG_EBSC: u32 = ft_make_tag(b'E', b'B', b'S', b'C');
pub const TTAG_feat: u32 = ft_make_tag(b'f', b'e', b'a', b't');
pub const TTAG_FOND: u32 = ft_make_tag(b'F', b'O', b'N', b'D');
pub const TTAG_fpgm: u32 = ft_make_tag(b'f', b'p', b'g', b'm');
pub const TTAG_fvar: u32 = ft_make_tag(b'f', b'v', b'a', b'r');
pub const TTAG_gasp: u32 = ft_make_tag(b'g', b'a', b's', b'p');
pub const TTAG_GDEF: u32 = ft_make_tag(b'G', b'D', b'E', b'F');
pub const TTAG_glyf: u32 = ft_make_tag(b'g', b'l', b'y', b'f');
pub const TTAG_GPOS: u32 = ft_make_tag(b'G', b'P', b'O', b'S');
pub const TTAG_GSUB: u32 = ft_make_tag(b'G', b'S', b'U', b'B');
pub const TTAG_gvar: u32 = ft_make_tag(b'g', b'v', b'a', b'r');
pub const TTAG_HVAR: u32 = ft_make_tag(b'H', b'V', b'A', b'R');
pub const TTAG_hdmx: u32 = ft_make_tag(b'h', b'd', b'm', b'x');
pub const TTAG_head: u32 = ft_make_tag(b'h', b'e', b'a', b'd');
pub const TTAG_hhea: u32 = ft_make_tag(b'h', b'h', b'e', b'a');
pub const TTAG_hmtx: u32 = ft_make_tag(b'h', b'm', b't', b'x');
pub const TTAG_JSTF: u32 = ft_make_tag(b'J', b'S', b'T', b'F');
pub const TTAG_just: u32 = ft_make_tag(b'j', b'u', b's', b't');
pub const TTAG_kern: u32 = ft_make_tag(b'k', b'e', b'r', b'n');
pub const TTAG_lcar: u32 = ft_make_tag(b'l', b'c', b'a', b'r');
pub const TTAG_loca: u32 = ft_make_tag(b'l', b'o', b'c', b'a');
pub const TTAG_LTSH: u32 = ft_make_tag(b'L', b'T', b'S', b'H');
pub const TTAG_LWFN: u32 = ft_make_tag(b'L', b'W', b'F', b'N');
pub const TTAG_MATH: u32 = ft_make_tag(b'M', b'A', b'T', b'H');
pub const TTAG_maxp: u32 = ft_make_tag(b'm', b'a', b'x', b'p');
pub const TTAG_META: u32 = ft_make_tag(b'M', b'E', b'T', b'A');
pub const TTAG_MMFX: u32 = ft_make_tag(b'M', b'M', b'F', b'X');
pub const TTAG_MMSD: u32 = ft_make_tag(b'M', b'M', b'S', b'D');
pub const TTAG_mort: u32 = ft_make_tag(b'm', b'o', b'r', b't');
pub const TTAG_morx: u32 = ft_make_tag(b'm', b'o', b'r', b'x');
pub const TTAG_MVAR: u32 = ft_make_tag(b'M', b'V', b'A', b'R');
pub const TTAG_name: u32 = ft_make_tag(b'n', b'a', b'm', b'e');
pub const TTAG_opbd: u32 = ft_make_tag(b'o', b'p', b'b', b'd');
pub const TTAG_OS2: u32 = ft_make_tag(b'O', b'S', b'/', b'2');
pub const TTAG_OTTO: u32 = ft_make_tag(b'O', b'T', b'T', b'O');
pub const TTAG_PCLT: u32 = ft_make_tag(b'P', b'C', b'L', b'T');
pub const TTAG_POST: u32 = ft_make_tag(b'P', b'O', b'S', b'T');
pub const TTAG_post: u32 = ft_make_tag(b'p', b'o', b's', b't');
pub const TTAG_prep: u32 = ft_make_tag(b'p', b'r', b'e', b'p');
pub const TTAG_prop: u32 = ft_make_tag(b'p', b'r', b'o', b'p');
pub const TTAG_sbix: u32 = ft_make_tag(b's', b'b', b'i', b'x');
pub const TTAG_sfnt: u32 = ft_make_tag(b's', b'f', b'n', b't');
pub const TTAG_SING: u32 = ft_make_tag(b'S', b'I', b'N', b'G');
pub const TTAG_SVG: u32 = ft_make_tag(b'S', b'V', b'G', b' ');
pub const TTAG_trak: u32 = ft_make_tag(b't', b'r', b'a', b'k');
pub const TTAG_true: u32 = ft_make_tag(b't', b'r', b'u', b'e');
pub const TTAG_ttc: u32 = ft_make_tag(b't', b't', b'c', b' ');
pub const TTAG_ttcf: u32 = ft_make_tag(b't', b't', b'c', b'f');
pub const TTAG_TYP1: u32 = ft_make_tag(b'T', b'Y', b'P', b'1');
pub const TTAG_typ1: u32 = ft_make_tag(b't', b'y', b'p', b'1');
pub const TTAG_VDMX: u32 = ft_make_tag(b'V', b'D', b'M', b'X');
pub const TTAG_vhea: u32 = ft_make_tag(b'v', b'h', b'e', b'a');
pub const TTAG_vmtx: u32 = ft_make_tag(b'v', b'm', b't', b'x');
pub const TTAG_VVAR: u32 = ft_make_tag(b'V', b'V', b'A', b'R');
pub const TTAG_wOFF: u32 = ft_make_tag(b'w', b'O', b'F', b'F');
pub const TTAG_wOF2: u32 = ft_make_tag(b'w', b'O', b'F', b'2');
pub const TTAG_0xA5kbd: u32 = ft_make_tag(0xA5u8, b'k', b'b', b'd');
pub const TTAG_0xA5lst: u32 = ft_make_tag(0xA5u8, b'l', b's', b't');
pub const TT_PLATFORM_APPLE_UNICODE: u16 = 0;
pub const TT_PLATFORM_MACINTOSH: u16 = 1;
pub const TT_PLATFORM_ISO: u16 = 2;
pub const TT_PLATFORM_MICROSOFT: u16 = 3;
pub const TT_PLATFORM_CUSTOM: u16 = 4;
pub const TT_PLATFORM_ADOBE: u16 = 7;
pub const TT_APPLE_ID_DEFAULT: u16 = 0;
pub const TT_APPLE_ID_UNICODE_1_1: u16 = 1;
pub const TT_APPLE_ID_ISO_10646: u16 = 2;
pub const TT_APPLE_ID_UNICODE_2_0: u16 = 3;
pub const TT_APPLE_ID_UNICODE_32: u16 = 4;
pub const TT_APPLE_ID_VARIANT_SELECTOR: u16 = 5;
pub const TT_APPLE_ID_FULL_UNICODE: u16 = 6;
pub const TT_MAC_ID_ROMAN: u16 = 0;
pub const TT_MAC_ID_JAPANESE: u16 = 1;
pub const TT_MAC_ID_TRADITIONAL_CHINESE: u16 = 2;
pub const TT_MAC_ID_KOREAN: u16 = 3;
pub const TT_MAC_ID_ARABIC: u16 = 4;
pub const TT_MAC_ID_HEBREW: u16 = 5;
pub const TT_MAC_ID_GREEK: u16 = 6;
pub const TT_MAC_ID_RUSSIAN: u16 = 7;
pub const TT_MAC_ID_RSYMBOL: u16 = 8;
pub const TT_MAC_ID_DEVANAGARI: u16 = 9;
pub const TT_MAC_ID_GURMUKHI: u16 = 10;
pub const TT_MAC_ID_GUJARATI: u16 = 11;
pub const TT_MAC_ID_ORIYA: u16 = 12;
pub const TT_MAC_ID_BENGALI: u16 = 13;
pub const TT_MAC_ID_TAMIL: u16 = 14;
pub const TT_MAC_ID_TELUGU: u16 = 15;
pub const TT_MAC_ID_KANNADA: u16 = 16;
pub const TT_MAC_ID_MALAYALAM: u16 = 17;
pub const TT_MAC_ID_SINHALESE: u16 = 18;
pub const TT_MAC_ID_BURMESE: u16 = 19;
pub const TT_MAC_ID_KHMER: u16 = 20;
pub const TT_MAC_ID_THAI: u16 = 21;
pub const TT_MAC_ID_LAOTIAN: u16 = 22;
pub const TT_MAC_ID_GEORGIAN: u16 = 23;
pub const TT_MAC_ID_ARMENIAN: u16 = 24;
pub const TT_MAC_ID_MALDIVIAN: u16 = 25;
pub const TT_MAC_ID_SIMPLIFIED_CHINESE: u16 = 25;
pub const TT_MAC_ID_TIBETAN: u16 = 26;
pub const TT_MAC_ID_MONGOLIAN: u16 = 27;
pub const TT_MAC_ID_GEEZ: u16 = 28;
pub const TT_MAC_ID_SLAVIC: u16 = 29;
pub const TT_MAC_ID_VIETNAMESE: u16 = 30;
pub const TT_MAC_ID_SINDHI: u16 = 31;
pub const TT_MAC_ID_UNINTERP: u16 = 32;
pub const TT_ISO_ID_7BIT_ASCII: u16 = 0;
pub const TT_ISO_ID_10646: u16 = 1;
pub const TT_ISO_ID_8859_1: u16 = 2;
pub const TT_MS_ID_SYMBOL_CS: u16 = 0;
pub const TT_MS_ID_UNICODE_CS: u16 = 1;
pub const TT_MS_ID_SJIS: u16 = 2;
pub const TT_MS_ID_PRC: u16 = 3;
pub const TT_MS_ID_BIG_5: u16 = 4;
pub const TT_MS_ID_WANSUNG: u16 = 5;
pub const TT_MS_ID_JOHAB: u16 = 6;
pub const TT_MS_ID_UCS_4: u16 = 10;
pub const TT_MS_ID_GB2312: u16 = TT_MS_ID_PRC;
pub const TT_ADOBE_ID_STANDARD: u16 = 0;
pub const TT_ADOBE_ID_EXPERT: u16 = 1;
pub const TT_ADOBE_ID_CUSTOM: u16 = 2;
pub const TT_ADOBE_ID_LATIN_1: u16 = 3;
pub const TT_MAC_LANGID_ENGLISH: u16 = 0;
pub const TT_MAC_LANGID_FRENCH: u16 = 1;
pub const TT_MAC_LANGID_GERMAN: u16 = 2;
pub const TT_MAC_LANGID_ITALIAN: u16 = 3;
pub const TT_MAC_LANGID_DUTCH: u16 = 4;
pub const TT_MAC_LANGID_SWEDISH: u16 = 5;
pub const TT_MAC_LANGID_SPANISH: u16 = 6;
pub const TT_MAC_LANGID_DANISH: u16 = 7;
pub const TT_MAC_LANGID_PORTUGUESE: u16 = 8;
pub const TT_MAC_LANGID_NORWEGIAN: u16 = 9;
pub const TT_MAC_LANGID_HEBREW: u16 = 10;
pub const TT_MAC_LANGID_JAPANESE: u16 = 11;
pub const TT_MAC_LANGID_ARABIC: u16 = 12;
pub const TT_MAC_LANGID_FINNISH: u16 = 13;
pub const TT_MAC_LANGID_GREEK: u16 = 14;
pub const TT_MAC_LANGID_ICELANDIC: u16 = 15;
pub const TT_MAC_LANGID_MALTESE: u16 = 16;
pub const TT_MAC_LANGID_TURKISH: u16 = 17;
pub const TT_MAC_LANGID_CROATIAN: u16 = 18;
pub const TT_MAC_LANGID_CHINESE_TRADITIONAL: u16 = 19;
pub const TT_MAC_LANGID_URDU: u16 = 20;
pub const TT_MAC_LANGID_HINDI: u16 = 21;
pub const TT_MAC_LANGID_THAI: u16 = 22;
pub const TT_MAC_LANGID_KOREAN: u16 = 23;
pub const TT_MAC_LANGID_LITHUANIAN: u16 = 24;
pub const TT_MAC_LANGID_POLISH: u16 = 25;
pub const TT_MAC_LANGID_HUNGARIAN: u16 = 26;
pub const TT_MAC_LANGID_ESTONIAN: u16 = 27;
pub const TT_MAC_LANGID_LETTISH: u16 = 28;
pub const TT_MAC_LANGID_SAAMISK: u16 = 29;
pub const TT_MAC_LANGID_FAEROESE: u16 = 30;
pub const TT_MAC_LANGID_FARSI: u16 = 31;
pub const TT_MAC_LANGID_RUSSIAN: u16 = 32;
pub const TT_MAC_LANGID_CHINESE_SIMPLIFIED: u16 = 33;
pub const TT_MAC_LANGID_FLEMISH: u16 = 34;
pub const TT_MAC_LANGID_IRISH: u16 = 35;
pub const TT_MAC_LANGID_ALBANIAN: u16 = 36;
pub const TT_MAC_LANGID_ROMANIAN: u16 = 37;
pub const TT_MAC_LANGID_CZECH: u16 = 38;
pub const TT_MAC_LANGID_SLOVAK: u16 = 39;
pub const TT_MAC_LANGID_SLOVENIAN: u16 = 40;
pub const TT_MAC_LANGID_YIDDISH: u16 = 41;
pub const TT_MAC_LANGID_SERBIAN: u16 = 42;
pub const TT_MAC_LANGID_MACEDONIAN: u16 = 43;
pub const TT_MAC_LANGID_BULGARIAN: u16 = 44;
pub const TT_MAC_LANGID_UKRAINIAN: u16 = 45;
pub const TT_MAC_LANGID_BYELORUSSIAN: u16 = 46;
pub const TT_MAC_LANGID_UZBEK: u16 = 47;
pub const TT_MAC_LANGID_KAZAKH: u16 = 48;
pub const TT_MAC_LANGID_AZERBAIJANI: u16 = 49;
pub const TT_MAC_LANGID_AZERBAIJANI_CYRILLIC_SCRIPT: u16 = 49;
pub const TT_MAC_LANGID_AZERBAIJANI_ARABIC_SCRIPT: u16 = 50;
pub const TT_MAC_LANGID_ARMENIAN: u16 = 51;
pub const TT_MAC_LANGID_GEORGIAN: u16 = 52;
pub const TT_MAC_LANGID_MOLDAVIAN: u16 = 53;
pub const TT_MAC_LANGID_KIRGHIZ: u16 = 54;
pub const TT_MAC_LANGID_TAJIKI: u16 = 55;
pub const TT_MAC_LANGID_TURKMEN: u16 = 56;
pub const TT_MAC_LANGID_MONGOLIAN: u16 = 57;
pub const TT_MAC_LANGID_MONGOLIAN_MONGOLIAN_SCRIPT: u16 = 57;
pub const TT_MAC_LANGID_MONGOLIAN_CYRILLIC_SCRIPT: u16 = 58;
pub const TT_MAC_LANGID_PASHTO: u16 = 59;
pub const TT_MAC_LANGID_KURDISH: u16 = 60;
pub const TT_MAC_LANGID_KASHMIRI: u16 = 61;
pub const TT_MAC_LANGID_SINDHI: u16 = 62;
pub const TT_MAC_LANGID_TIBETAN: u16 = 63;
pub const TT_MAC_LANGID_NEPALI: u16 = 64;
pub const TT_MAC_LANGID_SANSKRIT: u16 = 65;
pub const TT_MAC_LANGID_MARATHI: u16 = 66;
pub const TT_MAC_LANGID_BENGALI: u16 = 67;
pub const TT_MAC_LANGID_ASSAMESE: u16 = 68;
pub const TT_MAC_LANGID_GUJARATI: u16 = 69;
pub const TT_MAC_LANGID_PUNJABI: u16 = 70;
pub const TT_MAC_LANGID_ORIYA: u16 = 71;
pub const TT_MAC_LANGID_MALAYALAM: u16 = 72;
pub const TT_MAC_LANGID_KANNADA: u16 = 73;
pub const TT_MAC_LANGID_TAMIL: u16 = 74;
pub const TT_MAC_LANGID_TELUGU: u16 = 75;
pub const TT_MAC_LANGID_SINHALESE: u16 = 76;
pub const TT_MAC_LANGID_BURMESE: u16 = 77;
pub const TT_MAC_LANGID_KHMER: u16 = 78;
pub const TT_MAC_LANGID_LAO: u16 = 79;
pub const TT_MAC_LANGID_VIETNAMESE: u16 = 80;
pub const TT_MAC_LANGID_INDONESIAN: u16 = 81;
pub const TT_MAC_LANGID_TAGALOG: u16 = 82;
pub const TT_MAC_LANGID_MALAY_ROMAN_SCRIPT: u16 = 83;
pub const TT_MAC_LANGID_MALAY_ARABIC_SCRIPT: u16 = 84;
pub const TT_MAC_LANGID_AMHARIC: u16 = 85;
pub const TT_MAC_LANGID_TIGRINYA: u16 = 86;
pub const TT_MAC_LANGID_GALLA: u16 = 87;
pub const TT_MAC_LANGID_SOMALI: u16 = 88;
pub const TT_MAC_LANGID_SWAHILI: u16 = 89;
pub const TT_MAC_LANGID_RUANDA: u16 = 90;
pub const TT_MAC_LANGID_RUNDI: u16 = 91;
pub const TT_MAC_LANGID_CHEWA: u16 = 92;
pub const TT_MAC_LANGID_MALAGASY: u16 = 93;
pub const TT_MAC_LANGID_ESPERANTO: u16 = 94;
pub const TT_MAC_LANGID_WELSH: u16 = 128;
pub const TT_MAC_LANGID_BASQUE: u16 = 129;
pub const TT_MAC_LANGID_CATALAN: u16 = 130;
pub const TT_MAC_LANGID_LATIN: u16 = 131;
pub const TT_MAC_LANGID_QUECHUA: u16 = 132;
pub const TT_MAC_LANGID_GUARANI: u16 = 133;
pub const TT_MAC_LANGID_AYMARA: u16 = 134;
pub const TT_MAC_LANGID_TATAR: u16 = 135;
pub const TT_MAC_LANGID_UIGHUR: u16 = 136;
pub const TT_MAC_LANGID_DZONGKHA: u16 = 137;
pub const TT_MAC_LANGID_JAVANESE: u16 = 138;
pub const TT_MAC_LANGID_SUNDANESE: u16 = 139;
pub const TT_MAC_LANGID_GALICIAN: u16 = 140;
pub const TT_MAC_LANGID_AFRIKAANS: u16 = 141;
pub const TT_MAC_LANGID_BRETON: u16 = 142;
pub const TT_MAC_LANGID_INUKTITUT: u16 = 143;
pub const TT_MAC_LANGID_SCOTTISH_GAELIC: u16 = 144;
pub const TT_MAC_LANGID_MANX_GAELIC: u16 = 145;
pub const TT_MAC_LANGID_IRISH_GAELIC: u16 = 146;
pub const TT_MAC_LANGID_TONGAN: u16 = 147;
pub const TT_MAC_LANGID_GREEK_POLYTONIC: u16 = 148;
pub const TT_MAC_LANGID_GREELANDIC: u16 = 149;
pub const TT_MAC_LANGID_AZERBAIJANI_ROMAN_SCRIPT: u16 = 150;
pub const TT_MS_LANGID_ARABIC_SAUDI_ARABIA: u16 = 0x0401;
pub const TT_MS_LANGID_ARABIC_IRAQ: u16 = 0x0801;
pub const TT_MS_LANGID_ARABIC_EGYPT: u16 = 0x0C01;
pub const TT_MS_LANGID_ARABIC_LIBYA: u16 = 0x1001;
pub const TT_MS_LANGID_ARABIC_ALGERIA: u16 = 0x1401;
pub const TT_MS_LANGID_ARABIC_MOROCCO: u16 = 0x1801;
pub const TT_MS_LANGID_ARABIC_TUNISIA: u16 = 0x1C01;
pub const TT_MS_LANGID_ARABIC_OMAN: u16 = 0x2001;
pub const TT_MS_LANGID_ARABIC_YEMEN: u16 = 0x2401;
pub const TT_MS_LANGID_ARABIC_SYRIA: u16 = 0x2801;
pub const TT_MS_LANGID_ARABIC_JORDAN: u16 = 0x2C01;
pub const TT_MS_LANGID_ARABIC_LEBANON: u16 = 0x3001;
pub const TT_MS_LANGID_ARABIC_KUWAIT: u16 = 0x3401;
pub const TT_MS_LANGID_ARABIC_UAE: u16 = 0x3801;
pub const TT_MS_LANGID_ARABIC_BAHRAIN: u16 = 0x3C01;
pub const TT_MS_LANGID_ARABIC_QATAR: u16 = 0x4001;
pub const TT_MS_LANGID_BULGARIAN_BULGARIA: u16 = 0x0402;
pub const TT_MS_LANGID_CATALAN_CATALAN: u16 = 0x0403;
pub const TT_MS_LANGID_CHINESE_TAIWAN: u16 = 0x0404;
pub const TT_MS_LANGID_CHINESE_PRC: u16 = 0x0804;
pub const TT_MS_LANGID_CHINESE_HONG_KONG: u16 = 0x0C04;
pub const TT_MS_LANGID_CHINESE_SINGAPORE: u16 = 0x1004;
pub const TT_MS_LANGID_CHINESE_MACAO: u16 = 0x1404;
pub const TT_MS_LANGID_CZECH_CZECH_REPUBLIC: u16 = 0x0405;
pub const TT_MS_LANGID_DANISH_DENMARK: u16 = 0x0406;
pub const TT_MS_LANGID_GERMAN_GERMANY: u16 = 0x0407;
pub const TT_MS_LANGID_GERMAN_SWITZERLAND: u16 = 0x0807;
pub const TT_MS_LANGID_GERMAN_AUSTRIA: u16 = 0x0C07;
pub const TT_MS_LANGID_GERMAN_LUXEMBOURG: u16 = 0x1007;
pub const TT_MS_LANGID_GERMAN_LIECHTENSTEIN: u16 = 0x1407;
pub const TT_MS_LANGID_GREEK_GREECE: u16 = 0x0408;
pub const TT_MS_LANGID_ENGLISH_UNITED_STATES: u16 = 0x0409;
pub const TT_MS_LANGID_ENGLISH_UNITED_KINGDOM: u16 = 0x0809;
pub const TT_MS_LANGID_ENGLISH_AUSTRALIA: u16 = 0x0C09;
pub const TT_MS_LANGID_ENGLISH_CANADA: u16 = 0x1009;
pub const TT_MS_LANGID_ENGLISH_NEW_ZEALAND: u16 = 0x1409;
pub const TT_MS_LANGID_ENGLISH_IRELAND: u16 = 0x1809;
pub const TT_MS_LANGID_ENGLISH_SOUTH_AFRICA: u16 = 0x1C09;
pub const TT_MS_LANGID_ENGLISH_JAMAICA: u16 = 0x2009;
pub const TT_MS_LANGID_ENGLISH_CARIBBEAN: u16 = 0x2409;
pub const TT_MS_LANGID_ENGLISH_BELIZE: u16 = 0x2809;
pub const TT_MS_LANGID_ENGLISH_TRINIDAD: u16 = 0x2C09;
pub const TT_MS_LANGID_ENGLISH_ZIMBABWE: u16 = 0x3009;
pub const TT_MS_LANGID_ENGLISH_PHILIPPINES: u16 = 0x3409;
pub const TT_MS_LANGID_ENGLISH_INDIA: u16 = 0x4009;
pub const TT_MS_LANGID_ENGLISH_MALAYSIA: u16 = 0x4409;
pub const TT_MS_LANGID_ENGLISH_SINGAPORE: u16 = 0x4809;
pub const TT_MS_LANGID_SPANISH_SPAIN_TRADITIONAL_SORT: u16 = 0x040A;
pub const TT_MS_LANGID_SPANISH_MEXICO: u16 = 0x080A;
pub const TT_MS_LANGID_SPANISH_SPAIN_MODERN_SORT: u16 = 0x0C0A;
pub const TT_MS_LANGID_SPANISH_GUATEMALA: u16 = 0x100A;
pub const TT_MS_LANGID_SPANISH_COSTA_RICA: u16 = 0x140A;
pub const TT_MS_LANGID_SPANISH_PANAMA: u16 = 0x180A;
pub const TT_MS_LANGID_SPANISH_DOMINICAN_REPUBLIC: u16 = 0x1C0A;
pub const TT_MS_LANGID_SPANISH_VENEZUELA: u16 = 0x200A;
pub const TT_MS_LANGID_SPANISH_COLOMBIA: u16 = 0x240A;
pub const TT_MS_LANGID_SPANISH_PERU: u16 = 0x280A;
pub const TT_MS_LANGID_SPANISH_ARGENTINA: u16 = 0x2C0A;
pub const TT_MS_LANGID_SPANISH_ECUADOR: u16 = 0x300A;
pub const TT_MS_LANGID_SPANISH_CHILE: u16 = 0x340A;
pub const TT_MS_LANGID_SPANISH_URUGUAY: u16 = 0x380A;
pub const TT_MS_LANGID_SPANISH_PARAGUAY: u16 = 0x3C0A;
pub const TT_MS_LANGID_SPANISH_BOLIVIA: u16 = 0x400A;
pub const TT_MS_LANGID_SPANISH_EL_SALVADOR: u16 = 0x440A;
pub const TT_MS_LANGID_SPANISH_HONDURAS: u16 = 0x480A;
pub const TT_MS_LANGID_SPANISH_NICARAGUA: u16 = 0x4C0A;
pub const TT_MS_LANGID_SPANISH_PUERTO_RICO: u16 = 0x500A;
pub const TT_MS_LANGID_SPANISH_UNITED_STATES: u16 = 0x540A;
pub const TT_MS_LANGID_FINNISH_FINLAND: u16 = 0x040B;
pub const TT_MS_LANGID_FRENCH_FRANCE: u16 = 0x040C;
pub const TT_MS_LANGID_FRENCH_BELGIUM: u16 = 0x080C;
pub const TT_MS_LANGID_FRENCH_CANADA: u16 = 0x0C0C;
pub const TT_MS_LANGID_FRENCH_SWITZERLAND: u16 = 0x100C;
pub const TT_MS_LANGID_FRENCH_LUXEMBOURG: u16 = 0x140C;
pub const TT_MS_LANGID_FRENCH_MONACO: u16 = 0x180C;
pub const TT_MS_LANGID_HEBREW_ISRAEL: u16 = 0x040D;
pub const TT_MS_LANGID_HUNGARIAN_HUNGARY: u16 = 0x040E;
pub const TT_MS_LANGID_ICELANDIC_ICELAND: u16 = 0x040F;
pub const TT_MS_LANGID_ITALIAN_ITALY: u16 = 0x0410;
pub const TT_MS_LANGID_ITALIAN_SWITZERLAND: u16 = 0x0810;
pub const TT_MS_LANGID_JAPANESE_JAPAN: u16 = 0x0411;
pub const TT_MS_LANGID_KOREAN_KOREA: u16 = 0x0412;
pub const TT_MS_LANGID_DUTCH_NETHERLANDS: u16 = 0x0413;
pub const TT_MS_LANGID_DUTCH_BELGIUM: u16 = 0x0813;
pub const TT_MS_LANGID_NORWEGIAN_NORWAY_BOKMAL: u16 = 0x0414;
pub const TT_MS_LANGID_NORWEGIAN_NORWAY_NYNORSK: u16 = 0x0814;
pub const TT_MS_LANGID_POLISH_POLAND: u16 = 0x0415;
pub const TT_MS_LANGID_PORTUGUESE_BRAZIL: u16 = 0x0416;
pub const TT_MS_LANGID_PORTUGUESE_PORTUGAL: u16 = 0x0816;
pub const TT_MS_LANGID_ROMANSH_SWITZERLAND: u16 = 0x0417;
pub const TT_MS_LANGID_ROMANIAN_ROMANIA: u16 = 0x0418;
pub const TT_MS_LANGID_RUSSIAN_RUSSIA: u16 = 0x0419;
pub const TT_MS_LANGID_CROATIAN_CROATIA: u16 = 0x041A;
pub const TT_MS_LANGID_SERBIAN_SERBIA_LATIN: u16 = 0x081A;
pub const TT_MS_LANGID_SERBIAN_SERBIA_CYRILLIC: u16 = 0x0C1A;
pub const TT_MS_LANGID_CROATIAN_BOSNIA_HERZEGOVINA: u16 = 0x101A;
pub const TT_MS_LANGID_BOSNIAN_BOSNIA_HERZEGOVINA: u16 = 0x141A;
pub const TT_MS_LANGID_SERBIAN_BOSNIA_HERZ_LATIN: u16 = 0x181A;
pub const TT_MS_LANGID_SERBIAN_BOSNIA_HERZ_CYRILLIC: u16 = 0x1C1A;
pub const TT_MS_LANGID_BOSNIAN_BOSNIA_HERZ_CYRILLIC: u16 = 0x201A;
pub const TT_MS_LANGID_SLOVAK_SLOVAKIA: u16 = 0x041B;
pub const TT_MS_LANGID_ALBANIAN_ALBANIA: u16 = 0x041C;
pub const TT_MS_LANGID_SWEDISH_SWEDEN: u16 = 0x041D;
pub const TT_MS_LANGID_SWEDISH_FINLAND: u16 = 0x081D;
pub const TT_MS_LANGID_THAI_THAILAND: u16 = 0x041E;
pub const TT_MS_LANGID_TURKISH_TURKEY: u16 = 0x041F;
pub const TT_MS_LANGID_URDU_PAKISTAN: u16 = 0x0420;
pub const TT_MS_LANGID_INDONESIAN_INDONESIA: u16 = 0x0421;
pub const TT_MS_LANGID_UKRAINIAN_UKRAINE: u16 = 0x0422;
pub const TT_MS_LANGID_BELARUSIAN_BELARUS: u16 = 0x0423;
pub const TT_MS_LANGID_SLOVENIAN_SLOVENIA: u16 = 0x0424;
pub const TT_MS_LANGID_ESTONIAN_ESTONIA: u16 = 0x0425;
pub const TT_MS_LANGID_LATVIAN_LATVIA: u16 = 0x0426;
pub const TT_MS_LANGID_LITHUANIAN_LITHUANIA: u16 = 0x0427;
pub const TT_MS_LANGID_TAJIK_TAJIKISTAN: u16 = 0x0428;
pub const TT_MS_LANGID_VIETNAMESE_VIET_NAM: u16 = 0x042A;
pub const TT_MS_LANGID_ARMENIAN_ARMENIA: u16 = 0x042B;
pub const TT_MS_LANGID_AZERI_AZERBAIJAN_LATIN: u16 = 0x042C;
pub const TT_MS_LANGID_AZERI_AZERBAIJAN_CYRILLIC: u16 = 0x082C;
pub const TT_MS_LANGID_BASQUE_BASQUE: u16 = 0x042D;
pub const TT_MS_LANGID_UPPER_SORBIAN_GERMANY: u16 = 0x042E;
pub const TT_MS_LANGID_LOWER_SORBIAN_GERMANY: u16 = 0x082E;
pub const TT_MS_LANGID_MACEDONIAN_MACEDONIA: u16 = 0x042F;
pub const TT_MS_LANGID_SETSWANA_SOUTH_AFRICA: u16 = 0x0432;
pub const TT_MS_LANGID_ISIXHOSA_SOUTH_AFRICA: u16 = 0x0434;
pub const TT_MS_LANGID_ISIZULU_SOUTH_AFRICA: u16 = 0x0435;
pub const TT_MS_LANGID_AFRIKAANS_SOUTH_AFRICA: u16 = 0x0436;
pub const TT_MS_LANGID_GEORGIAN_GEORGIA: u16 = 0x0437;
pub const TT_MS_LANGID_FAEROESE_FAEROE_ISLANDS: u16 = 0x0438;
pub const TT_MS_LANGID_HINDI_INDIA: u16 = 0x0439;
pub const TT_MS_LANGID_MALTESE_MALTA: u16 = 0x043A;
pub const TT_MS_LANGID_SAMI_NORTHERN_NORWAY: u16 = 0x043B;
pub const TT_MS_LANGID_SAMI_NORTHERN_SWEDEN: u16 = 0x083B;
pub const TT_MS_LANGID_SAMI_NORTHERN_FINLAND: u16 = 0x0C3B;
pub const TT_MS_LANGID_SAMI_LULE_NORWAY: u16 = 0x103B;
pub const TT_MS_LANGID_SAMI_LULE_SWEDEN: u16 = 0x143B;
pub const TT_MS_LANGID_SAMI_SOUTHERN_NORWAY: u16 = 0x183B;
pub const TT_MS_LANGID_SAMI_SOUTHERN_SWEDEN: u16 = 0x1C3B;
pub const TT_MS_LANGID_SAMI_SKOLT_FINLAND: u16 = 0x203B;
pub const TT_MS_LANGID_SAMI_INARI_FINLAND: u16 = 0x243B;
pub const TT_MS_LANGID_IRISH_IRELAND: u16 = 0x083C;
pub const TT_MS_LANGID_MALAY_MALAYSIA: u16 = 0x043E;
pub const TT_MS_LANGID_MALAY_BRUNEI_DARUSSALAM: u16 = 0x083E;
pub const TT_MS_LANGID_KAZAKH_KAZAKHSTAN: u16 = 0x043F;
pub const TT_MS_LANGID_KISWAHILI_KENYA: u16 = 0x0441;
pub const TT_MS_LANGID_TURKMEN_TURKMENISTAN: u16 = 0x0442;
pub const TT_MS_LANGID_UZBEK_UZBEKISTAN_LATIN: u16 = 0x0443;
pub const TT_MS_LANGID_UZBEK_UZBEKISTAN_CYRILLIC: u16 = 0x0843;
pub const TT_MS_LANGID_TATAR_RUSSIA: u16 = 0x0444;
pub const TT_MS_LANGID_BENGALI_INDIA: u16 = 0x0445;
pub const TT_MS_LANGID_BENGALI_BANGLADESH: u16 = 0x0845;
pub const TT_MS_LANGID_PUNJABI_INDIA: u16 = 0x0446;
pub const TT_MS_LANGID_GUJARATI_INDIA: u16 = 0x0447;
pub const TT_MS_LANGID_ODIA_INDIA: u16 = 0x0448;
pub const TT_MS_LANGID_TAMIL_INDIA: u16 = 0x0449;
pub const TT_MS_LANGID_TELUGU_INDIA: u16 = 0x044A;
pub const TT_MS_LANGID_KANNADA_INDIA: u16 = 0x044B;
pub const TT_MS_LANGID_MALAYALAM_INDIA: u16 = 0x044C;
pub const TT_MS_LANGID_ASSAMESE_INDIA: u16 = 0x044D;
pub const TT_MS_LANGID_MARATHI_INDIA: u16 = 0x044E;
pub const TT_MS_LANGID_SANSKRIT_INDIA: u16 = 0x044F;
pub const TT_MS_LANGID_MONGOLIAN_PRC: u16 = 0x0850;
pub const TT_MS_LANGID_TIBETAN_PRC: u16 = 0x0451;
pub const TT_MS_LANGID_WELSH_UNITED_KINGDOM: u16 = 0x0452;
pub const TT_MS_LANGID_KHMER_CAMBODIA: u16 = 0x0453;
pub const TT_MS_LANGID_LAO_LAOS: u16 = 0x0454;
pub const TT_MS_LANGID_GALICIAN_GALICIAN: u16 = 0x0456;
pub const TT_MS_LANGID_KONKANI_INDIA: u16 = 0x0457;
pub const TT_MS_LANGID_SYRIAC_SYRIA: u16 = 0x045A;
pub const TT_MS_LANGID_SINHALA_SRI_LANKA: u16 = 0x045B;
pub const TT_MS_LANGID_INUKTITUT_CANADA: u16 = 0x045D;
pub const TT_MS_LANGID_INUKTITUT_CANADA_LATIN: u16 = 0x085D;
pub const TT_MS_LANGID_AMHARIC_ETHIOPIA: u16 = 0x045E;
pub const TT_MS_LANGID_TAMAZIGHT_ALGERIA: u16 = 0x085F;
pub const TT_MS_LANGID_NEPALI_NEPAL: u16 = 0x0461;
pub const TT_MS_LANGID_FRISIAN_NETHERLANDS: u16 = 0x0462;
pub const TT_MS_LANGID_PASHTO_AFGHANISTAN: u16 = 0x0463;
pub const TT_MS_LANGID_FILIPINO_PHILIPPINES: u16 = 0x0464;
pub const TT_MS_LANGID_DHIVEHI_MALDIVES: u16 = 0x0465;
pub const TT_MS_LANGID_HAUSA_NIGERIA: u16 = 0x0468;
pub const TT_MS_LANGID_YORUBA_NIGERIA: u16 = 0x046A;
pub const TT_MS_LANGID_QUECHUA_BOLIVIA: u16 = 0x046B;
pub const TT_MS_LANGID_QUECHUA_ECUADOR: u16 = 0x086B;
pub const TT_MS_LANGID_QUECHUA_PERU: u16 = 0x0C6B;
pub const TT_MS_LANGID_SESOTHO_SA_LEBOA_SOUTH_AFRICA: u16 = 0x046C;
pub const TT_MS_LANGID_BASHKIR_RUSSIA: u16 = 0x046D;
pub const TT_MS_LANGID_LUXEMBOURGISH_LUXEMBOURG: u16 = 0x046E;
pub const TT_MS_LANGID_GREENLANDIC_GREENLAND: u16 = 0x046F;
pub const TT_MS_LANGID_IGBO_NIGERIA: u16 = 0x0470;
pub const TT_MS_LANGID_YI_PRC: u16 = 0x0478;
pub const TT_MS_LANGID_MAPUDUNGUN_CHILE: u16 = 0x047A;
pub const TT_MS_LANGID_MOHAWK_MOHAWK: u16 = 0x047C;
pub const TT_MS_LANGID_BRETON_FRANCE: u16 = 0x047E;
pub const TT_MS_LANGID_UIGHUR_PRC: u16 = 0x0480;
pub const TT_MS_LANGID_MAORI_NEW_ZEALAND: u16 = 0x0481;
pub const TT_MS_LANGID_OCCITAN_FRANCE: u16 = 0x0482;
pub const TT_MS_LANGID_CORSICAN_FRANCE: u16 = 0x0483;
pub const TT_MS_LANGID_ALSATIAN_FRANCE: u16 = 0x0484;
pub const TT_MS_LANGID_YAKUT_RUSSIA: u16 = 0x0485;
pub const TT_MS_LANGID_KICHE_GUATEMALA: u16 = 0x0486;
pub const TT_MS_LANGID_KINYARWANDA_RWANDA: u16 = 0x0487;
pub const TT_MS_LANGID_WOLOF_SENEGAL: u16 = 0x0488;
pub const TT_MS_LANGID_DARI_AFGHANISTAN: u16 = 0x048C;
pub const TT_MS_LANGID_ARABIC_GENERAL: u16 = 0x0001;
pub const TT_MS_LANGID_CHINESE_GENERAL: u16 = 0x0004;
pub const TT_MS_LANGID_ENGLISH_GENERAL: u16 = 0x0009;
pub const TT_MS_LANGID_ENGLISH_INDONESIA: u16 = 0x3809;
pub const TT_MS_LANGID_ENGLISH_HONG_KONG: u16 = 0x3C09;
pub const TT_MS_LANGID_FRENCH_WEST_INDIES: u16 = 0x1C0C;
pub const TT_MS_LANGID_FRENCH_REUNION: u16 = 0x200C;
pub const TT_MS_LANGID_FRENCH_CONGO: u16 = 0x240C;
pub const TT_MS_LANGID_FRENCH_SENEGAL: u16 = 0x280C;
pub const TT_MS_LANGID_FRENCH_CAMEROON: u16 = 0x2C0C;
pub const TT_MS_LANGID_FRENCH_COTE_D_IVOIRE: u16 = 0x300C;
pub const TT_MS_LANGID_FRENCH_MALI: u16 = 0x340C;
pub const TT_MS_LANGID_FRENCH_MOROCCO: u16 = 0x380C;
pub const TT_MS_LANGID_FRENCH_HAITI: u16 = 0x3C0C;
pub const TT_MS_LANGID_KOREAN_JOHAB_KOREA: u16 = 0x0812;
pub const TT_MS_LANGID_MOLDAVIAN_MOLDAVIA: u16 = 0x0818;
pub const TT_MS_LANGID_RUSSIAN_MOLDAVIA: u16 = 0x0819;
pub const TT_MS_LANGID_URDU_INDIA: u16 = 0x0820;
pub const TT_MS_LANGID_CLASSIC_LITHUANIAN_LITHUANIA: u16 = 0x0827;
pub const TT_MS_LANGID_FARSI_IRAN: u16 = 0x0429;
pub const TT_MS_LANGID_SUTU_SOUTH_AFRICA: u16 = 0x0430;
pub const TT_MS_LANGID_TSONGA_SOUTH_AFRICA: u16 = 0x0431;
pub const TT_MS_LANGID_VENDA_SOUTH_AFRICA: u16 = 0x0433;
pub const TT_MS_LANGID_SAAMI_LAPONIA: u16 = 0x043B;
pub const TT_MS_LANGID_IRISH_GAELIC_IRELAND: u16 = 0x043C;
pub const TT_MS_LANGID_SCOTTISH_GAELIC_UNITED_KINGDOM: u16 = 0x083C;
pub const TT_MS_LANGID_YIDDISH_GERMANY: u16 = 0x043D;
pub const TT_MS_LANGID_PUNJABI_ARABIC_PAKISTAN: u16 = 0x0846;
pub const TT_MS_LANGID_DZONGHKA_BHUTAN: u16 = 0x0851;
pub const TT_MS_LANGID_BURMESE_MYANMAR: u16 = 0x0455;
pub const TT_MS_LANGID_SINDHI_PAKISTAN: u16 = 0x0859;
pub const TT_MS_LANGID_CHEROKEE_UNITED_STATES: u16 = 0x045C;
pub const TT_MS_LANGID_KASHMIRI_SASIA: u16 = 0x0860;
pub const TT_MS_LANGID_NEPALI_INDIA: u16 = 0x0861;
pub const TT_MS_LANGID_EDO_NIGERIA: u16 = 0x0466;
pub const TT_MS_LANGID_FULFULDE_NIGERIA: u16 = 0x0467;
pub const TT_MS_LANGID_IBIBIO_NIGERIA: u16 = 0x0469;
pub const TT_MS_LANGID_KANURI_NIGERIA: u16 = 0x0471;
pub const TT_MS_LANGID_OROMO_ETHIOPIA: u16 = 0x0472;
pub const TT_MS_LANGID_TIGRIGNA_ETHIOPIA: u16 = 0x0473;
pub const TT_MS_LANGID_TIGRIGNA_ERYTHREA: u16 = 0x0873;
pub const TT_MS_LANGID_GUARANI_PARAGUAY: u16 = 0x0474;
pub const TT_MS_LANGID_HAWAIIAN_UNITED_STATES: u16 = 0x0475;
pub const TT_MS_LANGID_LATIN: u16 = 0x0476;
pub const TT_MS_LANGID_SOMALI_SOMALIA: u16 = 0x0477;
pub const TT_MS_LANGID_PAPIAMENTU_NETHERLANDS_ANTILLES: u16 = 0x0479;
pub const TT_NAME_ID_COPYRIGHT: u16 = 0;
pub const TT_NAME_ID_FONT_FAMILY: u16 = 1;
pub const TT_NAME_ID_FONT_SUBFAMILY: u16 = 2;
pub const TT_NAME_ID_UNIQUE_ID: u16 = 3;
pub const TT_NAME_ID_FULL_NAME: u16 = 4;
pub const TT_NAME_ID_VERSION_STRING: u16 = 5;
pub const TT_NAME_ID_PS_NAME: u16 = 6;
pub const TT_NAME_ID_TRADEMARK: u16 = 7;
pub const TT_NAME_ID_MANUFACTURER: u16 = 8;
pub const TT_NAME_ID_DESIGNER: u16 = 9;
pub const TT_NAME_ID_DESCRIPTION: u16 = 10;
pub const TT_NAME_ID_VENDOR_URL: u16 = 11;
pub const TT_NAME_ID_DESIGNER_URL: u16 = 12;
pub const TT_NAME_ID_LICENSE: u16 = 13;
pub const TT_NAME_ID_LICENSE_URL: u16 = 14;
pub const TT_NAME_ID_TYPOGRAPHIC_FAMILY: u16 = 16;
pub const TT_NAME_ID_TYPOGRAPHIC_SUBFAMILY: u16 = 17;
pub const TT_NAME_ID_MAC_FULL_NAME: u16 = 18;
pub const TT_NAME_ID_SAMPLE_TEXT: u16 = 19;
pub const TT_NAME_ID_CID_FINDFONT_NAME: u16 = 20;
pub const TT_NAME_ID_WWS_FAMILY: u16 = 21;
pub const TT_NAME_ID_WWS_SUBFAMILY: u16 = 22;
pub const TT_NAME_ID_LIGHT_BACKGROUND: u16 = 23;
pub const TT_NAME_ID_DARK_BACKGROUND: u16 = 24;
pub const TT_NAME_ID_VARIATIONS_PREFIX: u16 = 25;
pub const TT_NAME_ID_PREFERRED_FAMILY: u16 = TT_NAME_ID_TYPOGRAPHIC_FAMILY;
pub const TT_NAME_ID_PREFERRED_SUBFAMILY: u16 = TT_NAME_ID_TYPOGRAPHIC_SUBFAMILY;
