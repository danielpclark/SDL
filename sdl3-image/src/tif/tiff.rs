// Rust translation of libtiff/tiff.h from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Tag Image File Format (TIFF)
//!
//! Based on Rev 6.0 from:
//!    Developer's Desk
//!    Aldus Corporation
//!    411 First Ave. South
//!    Suite 200
//!    Seattle, WA  98104
//!    206-622-5500
//!
//!    (http://partners.adobe.com/asn/developer/PDFS/TN/TIFF6.pdf)
//!
//! For BigTIFF design notes see the following links
//!    http://www.remotesensing.org/libtiff/bigtiffdesign.html
//!    http://www.awaresystems.be/imaging/tiff/bigtiff.html
//!
//! The header's constants, up to the EXIF and GPS tags (the EXIF and GPS
//! directories are not read).

#![allow(dead_code)] // (the header's whole constant set)

pub(crate) const TIFF_VERSION_CLASSIC: u16 = 42;
pub(crate) const TIFF_VERSION_BIG: u16 = 43;

pub(crate) const TIFF_BIGENDIAN: u16 = 0x4d4d;
pub(crate) const TIFF_LITTLEENDIAN: u16 = 0x4949;
pub(crate) const MDI_LITTLEENDIAN: u16 = 0x5045;
pub(crate) const MDI_BIGENDIAN: u16 = 0x4550;

// TIFF header: `TIFFHeaderClassic` is 8 bytes (magic, version, the 32-bit
// offset of the first directory), `TIFFHeaderBig` 16 (magic, version,
// offset size, unused word, the 64-bit offset).
pub(crate) const SIZEOF_TIFF_HEADER_CLASSIC: usize = 8;
pub(crate) const SIZEOF_TIFF_HEADER_BIG: usize = 16;

// NB: In the comments below,
//  - items marked with a + are obsoleted by revision 5.0,
//  - items marked with a ! are introduced in revision 6.0.
//  - items marked with a % are introduced post revision 6.0.
//  - items marked with a $ are obsoleted by revision 6.0.
//  - items marked with a & are introduced by Adobe DNG specification.

// Tag data type information.
//
// Note: RATIONALs are the ratio of two 32-bit integer values.
pub(crate) type TIFFDataType = u16;
pub(crate) const TIFF_NOTYPE: TIFFDataType = 0; // placeholder
pub(crate) const TIFF_BYTE: TIFFDataType = 1; // 8-bit unsigned integer
pub(crate) const TIFF_ASCII: TIFFDataType = 2; // 8-bit bytes w/ last byte null
pub(crate) const TIFF_SHORT: TIFFDataType = 3; // 16-bit unsigned integer
pub(crate) const TIFF_LONG: TIFFDataType = 4; // 32-bit unsigned integer
pub(crate) const TIFF_RATIONAL: TIFFDataType = 5; // 64-bit unsigned fraction
pub(crate) const TIFF_SBYTE: TIFFDataType = 6; // !8-bit signed integer
pub(crate) const TIFF_UNDEFINED: TIFFDataType = 7; // !8-bit untyped data
pub(crate) const TIFF_SSHORT: TIFFDataType = 8; // !16-bit signed integer
pub(crate) const TIFF_SLONG: TIFFDataType = 9; // !32-bit signed integer
pub(crate) const TIFF_SRATIONAL: TIFFDataType = 10; // !64-bit signed fraction
pub(crate) const TIFF_FLOAT: TIFFDataType = 11; // !32-bit IEEE floating point
pub(crate) const TIFF_DOUBLE: TIFFDataType = 12; // !64-bit IEEE floating point
pub(crate) const TIFF_IFD: TIFFDataType = 13; // %32-bit unsigned integer (offset)
pub(crate) const TIFF_LONG8: TIFFDataType = 16; // BigTIFF 64-bit unsigned integer
pub(crate) const TIFF_SLONG8: TIFFDataType = 17; // BigTIFF 64-bit signed integer
pub(crate) const TIFF_IFD8: TIFFDataType = 18; // BigTIFF 64-bit unsigned integer (offset)
pub(crate) const TIFFTAG_SUBFILETYPE: u32 = 254; // subfile data descriptor
pub(crate) const FILETYPE_REDUCEDIMAGE: u32 = 0x1; // reduced resolution version
pub(crate) const FILETYPE_PAGE: u32 = 0x2; // one page of many
pub(crate) const FILETYPE_MASK: u32 = 0x4; // transparency mask
pub(crate) const TIFFTAG_OSUBFILETYPE: u32 = 255; // +kind of data in subfile
pub(crate) const OFILETYPE_IMAGE: u16 = 1; // full resolution image data
pub(crate) const OFILETYPE_REDUCEDIMAGE: u16 = 2; // reduced size image data
pub(crate) const OFILETYPE_PAGE: u16 = 3; // one page of many
pub(crate) const TIFFTAG_IMAGEWIDTH: u32 = 256; // image width in pixels
pub(crate) const TIFFTAG_IMAGELENGTH: u32 = 257; // image height in pixels
pub(crate) const TIFFTAG_BITSPERSAMPLE: u32 = 258; // bits per channel (sample)
pub(crate) const TIFFTAG_COMPRESSION: u32 = 259; // data compression technique
pub(crate) const COMPRESSION_NONE: u16 = 1; // dump mode
pub(crate) const COMPRESSION_CCITTRLE: u16 = 2; // CCITT modified Huffman RLE
pub(crate) const COMPRESSION_CCITTFAX3: u16 = 3; // CCITT Group 3 fax encoding
pub(crate) const COMPRESSION_CCITT_T4: u16 = 3; // CCITT T.4 (TIFF 6 name)
pub(crate) const COMPRESSION_CCITTFAX4: u16 = 4; // CCITT Group 4 fax encoding
pub(crate) const COMPRESSION_CCITT_T6: u16 = 4; // CCITT T.6 (TIFF 6 name)
pub(crate) const COMPRESSION_LZW: u16 = 5; // Lempel-Ziv  & Welch
pub(crate) const COMPRESSION_OJPEG: u16 = 6; // !6.0 JPEG
pub(crate) const COMPRESSION_JPEG: u16 = 7; // %JPEG DCT compression
pub(crate) const COMPRESSION_T85: u16 = 9; // !TIFF/FX T.85 JBIG compression
pub(crate) const COMPRESSION_T43: u16 = 10; // !TIFF/FX T.43 colour by layered JBIG compression
pub(crate) const COMPRESSION_NEXT: u16 = 32766; // NeXT 2-bit RLE
pub(crate) const COMPRESSION_CCITTRLEW: u16 = 32771; // #1 w/ word alignment
pub(crate) const COMPRESSION_PACKBITS: u16 = 32773; // Macintosh RLE
pub(crate) const COMPRESSION_THUNDERSCAN: u16 = 32809; // ThunderScan RLE
// codes 32895-32898 are reserved for ANSI IT8 TIFF/IT <dkelly@apago.com)
pub(crate) const COMPRESSION_IT8CTPAD: u16 = 32895; // IT8 CT w/padding
pub(crate) const COMPRESSION_IT8LW: u16 = 32896; // IT8 Linework RLE
pub(crate) const COMPRESSION_IT8MP: u16 = 32897; // IT8 Monochrome picture
pub(crate) const COMPRESSION_IT8BL: u16 = 32898; // IT8 Binary line art
// compression codes 32908-32911 are reserved for Pixar
pub(crate) const COMPRESSION_PIXARFILM: u16 = 32908; // Pixar companded 10bit LZW
pub(crate) const COMPRESSION_PIXARLOG: u16 = 32909; // Pixar companded 11bit ZIP
pub(crate) const COMPRESSION_DEFLATE: u16 = 32946; // Deflate compression, legacy tag
pub(crate) const COMPRESSION_ADOBE_DEFLATE: u16 = 8; // Deflate compression, as recognized by Adobe
// compression code 32947 is reserved for Oceana Matrix <dev@oceana.com>
pub(crate) const COMPRESSION_DCS: u16 = 32947; // Kodak DCS encoding
pub(crate) const COMPRESSION_JBIG: u16 = 34661; // ISO JBIG
pub(crate) const COMPRESSION_SGILOG: u16 = 34676; // SGI Log Luminance RLE
pub(crate) const COMPRESSION_SGILOG24: u16 = 34677; // SGI Log 24-bit packed
pub(crate) const COMPRESSION_JP2000: u16 = 34712; // Leadtools JPEG2000
pub(crate) const COMPRESSION_LERC: u16 = 34887; // ESRI Lerc codec: https://github.com/Esri/lerc
// compression codes 34887-34889 are reserved for ESRI
pub(crate) const COMPRESSION_LZMA: u16 = 34925; // LZMA2
pub(crate) const COMPRESSION_ZSTD: u16 = 50000; // ZSTD: WARNING not registered in Adobe-maintained registry
pub(crate) const COMPRESSION_WEBP: u16 = 50001; // WEBP: WARNING not registered in Adobe-maintained registry
pub(crate) const COMPRESSION_JXL: u16 = 50002; // JPEGXL: WARNING not registered in Adobe-maintained registry
pub(crate) const COMPRESSION_JXL_DNG_1_7: u16 = 52546; // JPEGXL from DNG 1.7 specification
pub(crate) const TIFFTAG_PHOTOMETRIC: u32 = 262; // photometric interpretation
pub(crate) const PHOTOMETRIC_MINISWHITE: u16 = 0; // min value is white
pub(crate) const PHOTOMETRIC_MINISBLACK: u16 = 1; // min value is black
pub(crate) const PHOTOMETRIC_RGB: u16 = 2; // RGB color model
pub(crate) const PHOTOMETRIC_PALETTE: u16 = 3; // color map indexed
pub(crate) const PHOTOMETRIC_MASK: u16 = 4; // $holdout mask
pub(crate) const PHOTOMETRIC_SEPARATED: u16 = 5; // !color separations
pub(crate) const PHOTOMETRIC_YCBCR: u16 = 6; // !CCIR 601
pub(crate) const PHOTOMETRIC_CIELAB: u16 = 8; // !1976 CIE L*a*b*
pub(crate) const PHOTOMETRIC_ICCLAB: u16 = 9; // ICC L*a*b* [Adobe TIFF Technote 4]
pub(crate) const PHOTOMETRIC_ITULAB: u16 = 10; // ITU L*a*b*
pub(crate) const PHOTOMETRIC_CFA: u16 = 32803; // color filter array
pub(crate) const PHOTOMETRIC_LOGL: u16 = 32844; // CIE Log2(L)
pub(crate) const PHOTOMETRIC_LOGLUV: u16 = 32845; // CIE Log2(L) (u',v')
pub(crate) const TIFFTAG_THRESHHOLDING: u32 = 263; // +thresholding used on data
pub(crate) const THRESHHOLD_BILEVEL: u16 = 1; // b&w art scan
pub(crate) const THRESHHOLD_HALFTONE: u16 = 2; // or dithered scan
pub(crate) const THRESHHOLD_ERRORDIFFUSE: u16 = 3; // usually floyd-steinberg
pub(crate) const TIFFTAG_CELLWIDTH: u32 = 264; // +dithering matrix width
pub(crate) const TIFFTAG_CELLLENGTH: u32 = 265; // +dithering matrix height
pub(crate) const TIFFTAG_FILLORDER: u32 = 266; // data order within a byte
pub(crate) const FILLORDER_MSB2LSB: u16 = 1; // most significant -> least
pub(crate) const FILLORDER_LSB2MSB: u16 = 2; // least significant -> most
pub(crate) const TIFFTAG_DOCUMENTNAME: u32 = 269; // name of doc. image is from
pub(crate) const TIFFTAG_IMAGEDESCRIPTION: u32 = 270; // info about image
pub(crate) const TIFFTAG_MAKE: u32 = 271; // scanner manufacturer name
pub(crate) const TIFFTAG_MODEL: u32 = 272; // scanner model name/number
pub(crate) const TIFFTAG_STRIPOFFSETS: u32 = 273; // offsets to data strips
pub(crate) const TIFFTAG_ORIENTATION: u32 = 274; // +image orientation
pub(crate) const ORIENTATION_TOPLEFT: u16 = 1; // row 0 top, col 0 lhs
pub(crate) const ORIENTATION_TOPRIGHT: u16 = 2; // row 0 top, col 0 rhs
pub(crate) const ORIENTATION_BOTRIGHT: u16 = 3; // row 0 bottom, col 0 rhs
pub(crate) const ORIENTATION_BOTLEFT: u16 = 4; // row 0 bottom, col 0 lhs
pub(crate) const ORIENTATION_LEFTTOP: u16 = 5; // row 0 lhs, col 0 top
pub(crate) const ORIENTATION_RIGHTTOP: u16 = 6; // row 0 rhs, col 0 top
pub(crate) const ORIENTATION_RIGHTBOT: u16 = 7; // row 0 rhs, col 0 bottom
pub(crate) const ORIENTATION_LEFTBOT: u16 = 8; // row 0 lhs, col 0 bottom
pub(crate) const TIFFTAG_SAMPLESPERPIXEL: u32 = 277; // samples per pixel
pub(crate) const TIFFTAG_ROWSPERSTRIP: u32 = 278; // rows per strip of data
pub(crate) const TIFFTAG_STRIPBYTECOUNTS: u32 = 279; // bytes counts for strips
pub(crate) const TIFFTAG_MINSAMPLEVALUE: u32 = 280; // +minimum sample value
pub(crate) const TIFFTAG_MAXSAMPLEVALUE: u32 = 281; // +maximum sample value
pub(crate) const TIFFTAG_XRESOLUTION: u32 = 282; // pixels/resolution in x
pub(crate) const TIFFTAG_YRESOLUTION: u32 = 283; // pixels/resolution in y
pub(crate) const TIFFTAG_PLANARCONFIG: u32 = 284; // storage organization
pub(crate) const PLANARCONFIG_CONTIG: u16 = 1; // single image plane
pub(crate) const PLANARCONFIG_SEPARATE: u16 = 2; // separate planes of data
pub(crate) const TIFFTAG_PAGENAME: u32 = 285; // page name image is from
pub(crate) const TIFFTAG_XPOSITION: u32 = 286; // x page offset of image lhs
pub(crate) const TIFFTAG_YPOSITION: u32 = 287; // y page offset of image lhs
pub(crate) const TIFFTAG_FREEOFFSETS: u32 = 288; // +byte offset to free block
pub(crate) const TIFFTAG_FREEBYTECOUNTS: u32 = 289; // +sizes of free blocks
pub(crate) const TIFFTAG_GRAYRESPONSEUNIT: u32 = 290; // $gray scale curve accuracy
pub(crate) const GRAYRESPONSEUNIT_10S: u16 = 1; // tenths of a unit
pub(crate) const GRAYRESPONSEUNIT_100S: u16 = 2; // hundredths of a unit
pub(crate) const GRAYRESPONSEUNIT_1000S: u16 = 3; // thousandths of a unit
pub(crate) const GRAYRESPONSEUNIT_10000S: u16 = 4; // ten-thousandths of a unit
pub(crate) const GRAYRESPONSEUNIT_100000S: u16 = 5; // hundred-thousandths
pub(crate) const TIFFTAG_GRAYRESPONSECURVE: u32 = 291; // $gray scale response curve
pub(crate) const TIFFTAG_GROUP3OPTIONS: u32 = 292; // 32 flag bits
pub(crate) const TIFFTAG_T4OPTIONS: u32 = 292; // TIFF 6.0 proper name alias
pub(crate) const GROUP3OPT_2DENCODING: u32 = 0x1; // 2-dimensional coding
pub(crate) const GROUP3OPT_UNCOMPRESSED: u32 = 0x2; // data not compressed
pub(crate) const GROUP3OPT_FILLBITS: u32 = 0x4; // fill to byte boundary
pub(crate) const TIFFTAG_GROUP4OPTIONS: u32 = 293; // 32 flag bits
pub(crate) const TIFFTAG_T6OPTIONS: u32 = 293; // TIFF 6.0 proper name
pub(crate) const GROUP4OPT_UNCOMPRESSED: u32 = 0x2; // data not compressed
pub(crate) const TIFFTAG_RESOLUTIONUNIT: u32 = 296; // units of resolutions
pub(crate) const RESUNIT_NONE: u16 = 1; // no meaningful units
pub(crate) const RESUNIT_INCH: u16 = 2; // english
pub(crate) const RESUNIT_CENTIMETER: u16 = 3; // metric
pub(crate) const TIFFTAG_PAGENUMBER: u32 = 297; // page numbers of multi-page
pub(crate) const TIFFTAG_COLORRESPONSEUNIT: u32 = 300; // $color curve accuracy
pub(crate) const COLORRESPONSEUNIT_10S: u16 = 1; // tenths of a unit
pub(crate) const COLORRESPONSEUNIT_100S: u16 = 2; // hundredths of a unit
pub(crate) const COLORRESPONSEUNIT_1000S: u16 = 3; // thousandths of a unit
pub(crate) const COLORRESPONSEUNIT_10000S: u16 = 4; // ten-thousandths of a unit
pub(crate) const COLORRESPONSEUNIT_100000S: u16 = 5; // hundred-thousandths
pub(crate) const TIFFTAG_TRANSFERFUNCTION: u32 = 301; // !colorimetry info
pub(crate) const TIFFTAG_SOFTWARE: u32 = 305; // name & release
pub(crate) const TIFFTAG_DATETIME: u32 = 306; // creation date and time
pub(crate) const TIFFTAG_ARTIST: u32 = 315; // creator of image
pub(crate) const TIFFTAG_HOSTCOMPUTER: u32 = 316; // machine where created
pub(crate) const TIFFTAG_PREDICTOR: u32 = 317; // prediction scheme w/ LZW
pub(crate) const PREDICTOR_NONE: u16 = 1; // no prediction scheme used
pub(crate) const PREDICTOR_HORIZONTAL: u16 = 2; // horizontal differencing
pub(crate) const PREDICTOR_FLOATINGPOINT: u16 = 3; // floating point predictor
pub(crate) const TIFFTAG_WHITEPOINT: u32 = 318; // image white point
pub(crate) const TIFFTAG_PRIMARYCHROMATICITIES: u32 = 319; // !primary chromaticities
pub(crate) const TIFFTAG_COLORMAP: u32 = 320; // RGB map for palette image
pub(crate) const TIFFTAG_HALFTONEHINTS: u32 = 321; // !highlight+shadow info
pub(crate) const TIFFTAG_TILEWIDTH: u32 = 322; // !tile width in pixels
pub(crate) const TIFFTAG_TILELENGTH: u32 = 323; // !tile height in pixels
pub(crate) const TIFFTAG_TILEOFFSETS: u32 = 324; // !offsets to data tiles
pub(crate) const TIFFTAG_TILEBYTECOUNTS: u32 = 325; // !byte counts for tiles
pub(crate) const TIFFTAG_BADFAXLINES: u32 = 326; // lines w/ wrong pixel count
pub(crate) const TIFFTAG_CLEANFAXDATA: u32 = 327; // regenerated line info
pub(crate) const CLEANFAXDATA_CLEAN: u16 = 0; // no errors detected
pub(crate) const CLEANFAXDATA_REGENERATED: u16 = 1; // receiver regenerated lines
pub(crate) const CLEANFAXDATA_UNCLEAN: u16 = 2; // uncorrected errors exist
pub(crate) const TIFFTAG_CONSECUTIVEBADFAXLINES: u32 = 328; // max consecutive bad lines
pub(crate) const TIFFTAG_SUBIFD: u32 = 330; // subimage descriptors
pub(crate) const TIFFTAG_INKSET: u32 = 332; // !inks in separated image
pub(crate) const INKSET_CMYK: u16 = 1; // !cyan-magenta-yellow-black color
pub(crate) const INKSET_MULTIINK: u16 = 2; // !multi-ink or hi-fi color
pub(crate) const TIFFTAG_INKNAMES: u32 = 333; // !ascii names of inks
pub(crate) const TIFFTAG_NUMBEROFINKS: u32 = 334; // !number of inks
pub(crate) const TIFFTAG_DOTRANGE: u32 = 336; // !0% and 100% dot codes
pub(crate) const TIFFTAG_TARGETPRINTER: u32 = 337; // !separation target
pub(crate) const TIFFTAG_EXTRASAMPLES: u32 = 338; // !info about extra samples
pub(crate) const EXTRASAMPLE_UNSPECIFIED: u16 = 0; // !unspecified data
pub(crate) const EXTRASAMPLE_ASSOCALPHA: u16 = 1; // !associated alpha data
pub(crate) const EXTRASAMPLE_UNASSALPHA: u16 = 2; // !unassociated alpha data
pub(crate) const TIFFTAG_SAMPLEFORMAT: u32 = 339; // !data sample format
pub(crate) const SAMPLEFORMAT_UINT: u16 = 1; // !unsigned integer data
pub(crate) const SAMPLEFORMAT_INT: u16 = 2; // !signed integer data
pub(crate) const SAMPLEFORMAT_IEEEFP: u16 = 3; // !IEEE floating point data
pub(crate) const SAMPLEFORMAT_VOID: u16 = 4; // !untyped data
pub(crate) const SAMPLEFORMAT_COMPLEXINT: u16 = 5; // !complex signed int
pub(crate) const SAMPLEFORMAT_COMPLEXIEEEFP: u16 = 6; // !complex ieee floating
pub(crate) const TIFFTAG_SMINSAMPLEVALUE: u32 = 340; // !variable MinSampleValue
pub(crate) const TIFFTAG_SMAXSAMPLEVALUE: u32 = 341; // !variable MaxSampleValue
pub(crate) const TIFFTAG_CLIPPATH: u32 = 343; // %ClipPath [Adobe TIFF technote 2]
pub(crate) const TIFFTAG_XCLIPPATHUNITS: u32 = 344; // %XClipPathUnits [Adobe TIFF technote 2]
pub(crate) const TIFFTAG_YCLIPPATHUNITS: u32 = 345; // %YClipPathUnits [Adobe TIFF technote 2]
pub(crate) const TIFFTAG_INDEXED: u32 = 346; // %Indexed [Adobe TIFF Technote 3]
pub(crate) const TIFFTAG_JPEGTABLES: u32 = 347; // %JPEG table stream
pub(crate) const TIFFTAG_OPIPROXY: u32 = 351; // %OPI Proxy [Adobe TIFF technote]
// Tags 400-435 are from the TIFF/FX spec
pub(crate) const TIFFTAG_GLOBALPARAMETERSIFD: u32 = 400; // !
pub(crate) const TIFFTAG_PROFILETYPE: u32 = 401; // !
pub(crate) const PROFILETYPE_UNSPECIFIED: u16 = 0; // !
pub(crate) const PROFILETYPE_G3_FAX: u16 = 1; // !
pub(crate) const TIFFTAG_FAXPROFILE: u32 = 402; // !
pub(crate) const FAXPROFILE_S: u16 = 1; // !TIFF/FX FAX profile S
pub(crate) const FAXPROFILE_F: u16 = 2; // !TIFF/FX FAX profile F
pub(crate) const FAXPROFILE_J: u16 = 3; // !TIFF/FX FAX profile J
pub(crate) const FAXPROFILE_C: u16 = 4; // !TIFF/FX FAX profile C
pub(crate) const FAXPROFILE_L: u16 = 5; // !TIFF/FX FAX profile L
pub(crate) const FAXPROFILE_M: u16 = 6; // !TIFF/FX FAX profile LM
pub(crate) const TIFFTAG_CODINGMETHODS: u32 = 403; // !TIFF/FX coding methods
pub(crate) const CODINGMETHODS_T4_1D: u32 = 1 << 1; // !T.4 1D
pub(crate) const CODINGMETHODS_T4_2D: u32 = 1 << 2; // !T.4 2D
pub(crate) const CODINGMETHODS_T6: u32 = 1 << 3; // !T.6
pub(crate) const CODINGMETHODS_T85: u32 = 1 << 4; // !T.85 JBIG
pub(crate) const CODINGMETHODS_T42: u32 = 1 << 5; // !T.42 JPEG
pub(crate) const CODINGMETHODS_T43: u32 = 1 << 6; // !T.43 colour by layered JBIG
pub(crate) const TIFFTAG_VERSIONYEAR: u32 = 404; // !TIFF/FX version year
pub(crate) const TIFFTAG_MODENUMBER: u32 = 405; // !TIFF/FX mode number
pub(crate) const TIFFTAG_DECODE: u32 = 433; // !TIFF/FX decode
pub(crate) const TIFFTAG_IMAGEBASECOLOR: u32 = 434; // !TIFF/FX image base colour
pub(crate) const TIFFTAG_T82OPTIONS: u32 = 435; // !TIFF/FX T.82 options
pub(crate) const TIFFTAG_JPEGPROC: u32 = 512; // !JPEG processing algorithm
pub(crate) const JPEGPROC_BASELINE: u16 = 1; // !baseline sequential
pub(crate) const JPEGPROC_LOSSLESS: u16 = 14; // !Huffman coded lossless
pub(crate) const TIFFTAG_JPEGIFOFFSET: u32 = 513; // !pointer to SOI marker
pub(crate) const TIFFTAG_JPEGIFBYTECOUNT: u32 = 514; // !JFIF stream length
pub(crate) const TIFFTAG_JPEGRESTARTINTERVAL: u32 = 515; // !restart interval length
pub(crate) const TIFFTAG_JPEGLOSSLESSPREDICTORS: u32 = 517; // !lossless proc predictor
pub(crate) const TIFFTAG_JPEGPOINTTRANSFORM: u32 = 518; // !lossless point transform
pub(crate) const TIFFTAG_JPEGQTABLES: u32 = 519; // !Q matrix offsets
pub(crate) const TIFFTAG_JPEGDCTABLES: u32 = 520; // !DCT table offsets
pub(crate) const TIFFTAG_JPEGACTABLES: u32 = 521; // !AC coefficient offsets
pub(crate) const TIFFTAG_YCBCRCOEFFICIENTS: u32 = 529; // !RGB -> YCbCr transform
pub(crate) const TIFFTAG_YCBCRSUBSAMPLING: u32 = 530; // !YCbCr subsampling factors
pub(crate) const TIFFTAG_YCBCRPOSITIONING: u32 = 531; // !subsample positioning
pub(crate) const YCBCRPOSITION_CENTERED: u16 = 1; // !as in PostScript Level 2
pub(crate) const YCBCRPOSITION_COSITED: u16 = 2; // !as in CCIR 601-1
pub(crate) const TIFFTAG_REFERENCEBLACKWHITE: u32 = 532; // !colorimetry info
pub(crate) const TIFFTAG_STRIPROWCOUNTS: u32 = 559; // !TIFF/FX strip row counts
pub(crate) const TIFFTAG_XMLPACKET: u32 = 700; // %XML packet [Adobe XMP Specification, January 2004
pub(crate) const TIFFTAG_OPIIMAGEID: u32 = 32781; // %OPI ImageID [Adobe TIFF technote]
pub(crate) const TIFFTAG_TIFFANNOTATIONDATA: u32 = 32932;
// tags 32952-32956 are private tags registered to Island Graphics
pub(crate) const TIFFTAG_REFPTS: u32 = 32953; // image reference points
pub(crate) const TIFFTAG_REGIONTACKPOINT: u32 = 32954; // region-xform tack point
pub(crate) const TIFFTAG_REGIONWARPCORNERS: u32 = 32955; // warp quadrilateral
pub(crate) const TIFFTAG_REGIONAFFINE: u32 = 32956; // affine transformation mat
// tags 32995-32999 are private tags registered to SGI
pub(crate) const TIFFTAG_MATTEING: u32 = 32995; // $use ExtraSamples
pub(crate) const TIFFTAG_DATATYPE: u32 = 32996; // $use SampleFormat
pub(crate) const TIFFTAG_IMAGEDEPTH: u32 = 32997; // z depth of image
pub(crate) const TIFFTAG_TILEDEPTH: u32 = 32998; // z depth/data tile
// tags 33300-33309 are private tags registered to Pixar
pub(crate) const TIFFTAG_PIXAR_IMAGEFULLWIDTH: u32 = 33300; // full image size in x
pub(crate) const TIFFTAG_PIXAR_IMAGEFULLLENGTH: u32 = 33301; // full image size in y
pub(crate) const TIFFTAG_PIXAR_TEXTUREFORMAT: u32 = 33302; // texture map format
pub(crate) const TIFFTAG_PIXAR_WRAPMODES: u32 = 33303; // s & t wrap modes
pub(crate) const TIFFTAG_PIXAR_FOVCOT: u32 = 33304; // cotan(fov) for env. maps
pub(crate) const TIFFTAG_PIXAR_MATRIX_WORLDTOSCREEN: u32 = 33305;
pub(crate) const TIFFTAG_PIXAR_MATRIX_WORLDTOCAMERA: u32 = 33306;
// tag 33405 is a private tag registered to Eastman Kodak
pub(crate) const TIFFTAG_WRITERSERIALNUMBER: u32 = 33405; // device serial number
pub(crate) const TIFFTAG_CFAREPEATPATTERNDIM: u32 = 33421; // (alias for TIFFTAG_EP_CFAREPEATPATTERNDIM)
pub(crate) const TIFFTAG_CFAPATTERN: u32 = 33422; // (alias for TIFFTAG_EP_CFAPATTERN)
pub(crate) const TIFFTAG_BATTERYLEVEL: u32 = 33423; // (alias for TIFFTAG_EP_BATTERYLEVEL)
// tag 33432 is listed in the 6.0 spec w/ unknown ownership
pub(crate) const TIFFTAG_COPYRIGHT: u32 = 33432; // copyright string
pub(crate) const TIFFTAG_MD_FILETAG: u32 = 33445; // Specifies the pixel data format encoding in the GEL file format.
pub(crate) const TIFFTAG_MD_SCALEPIXEL: u32 = 33446; // scale factor
pub(crate) const TIFFTAG_MD_COLORTABLE: u32 = 33447; // conversion from 16bit to 8bit
pub(crate) const TIFFTAG_MD_LABNAME: u32 = 33448; // name of the lab that scanned this file.
pub(crate) const TIFFTAG_MD_SAMPLEINFO: u32 = 33449; // information about the scanned GEL sample
pub(crate) const TIFFTAG_MD_PREPDATE: u32 = 33450; // information about the date the sample was prepared YY/MM/DD
pub(crate) const TIFFTAG_MD_PREPTIME: u32 = 33451; // information about the time the sample was prepared HH:MM
pub(crate) const TIFFTAG_MD_FILEUNITS: u32 = 33452; // Units for data in this file, as used in the GEL file format.
// IPTC TAG from RichTIFF specifications
pub(crate) const TIFFTAG_RICHTIFFIPTC: u32 = 33723;
pub(crate) const TIFFTAG_INGR_PACKET_DATA_TAG: u32 = 33918; // Intergraph Application specific storage.
pub(crate) const TIFFTAG_INGR_FLAG_REGISTERS: u32 = 33919; // Intergraph Application specific flags.
pub(crate) const TIFFTAG_IRASB_TRANSORMATION_MATRIX: u32 = 33920; // Originally part of Intergraph's GeoTIFF tags, but likely understood by IrasB only.
pub(crate) const TIFFTAG_MODELTIEPOINTTAG: u32 = 33922; // GeoTIFF
// 34016-34029 are reserved for ANSI IT8 TIFF/IT <dkelly@apago.com)
pub(crate) const TIFFTAG_IT8SITE: u32 = 34016; // site name
pub(crate) const TIFFTAG_IT8COLORSEQUENCE: u32 = 34017; // color seq. [RGB,CMYK,etc]
pub(crate) const TIFFTAG_IT8HEADER: u32 = 34018; // DDES Header
pub(crate) const TIFFTAG_IT8RASTERPADDING: u32 = 34019; // raster scanline padding
pub(crate) const TIFFTAG_IT8BITSPERRUNLENGTH: u32 = 34020; // # of bits in short run
pub(crate) const TIFFTAG_IT8BITSPEREXTENDEDRUNLENGTH: u32 = 34021; // # of bits in long run
pub(crate) const TIFFTAG_IT8COLORTABLE: u32 = 34022; // LW colortable
pub(crate) const TIFFTAG_IT8IMAGECOLORINDICATOR: u32 = 34023; // BP/BL image color switch
pub(crate) const TIFFTAG_IT8BKGCOLORINDICATOR: u32 = 34024; // BP/BL bg color switch
pub(crate) const TIFFTAG_IT8IMAGECOLORVALUE: u32 = 34025; // BP/BL image color value
pub(crate) const TIFFTAG_IT8BKGCOLORVALUE: u32 = 34026; // BP/BL bg color value
pub(crate) const TIFFTAG_IT8PIXELINTENSITYRANGE: u32 = 34027; // MP pixel intensity value
pub(crate) const TIFFTAG_IT8TRANSPARENCYINDICATOR: u32 = 34028; // HC transparency switch
pub(crate) const TIFFTAG_IT8COLORCHARACTERIZATION: u32 = 34029; // color character. table
pub(crate) const TIFFTAG_IT8HCUSAGE: u32 = 34030; // HC usage indicator
pub(crate) const TIFFTAG_IT8TRAPINDICATOR: u32 = 34031; // Trapping indicator (untrapped=0, trapped=1)
pub(crate) const TIFFTAG_IT8CMYKEQUIVALENT: u32 = 34032; // CMYK color equivalents
// tags 34232-34236 are private tags registered to Texas Instruments
pub(crate) const TIFFTAG_FRAMECOUNT: u32 = 34232; // Sequence Frame Count
pub(crate) const TIFFTAG_MODELTRANSFORMATIONTAG: u32 = 34264; // Used in interchangeable GeoTIFF files
// tag 34377 is private tag registered to Adobe for PhotoShop
pub(crate) const TIFFTAG_PHOTOSHOP: u32 = 34377;
// tags 34665, 34853 and 40965 are documented in EXIF specification
pub(crate) const TIFFTAG_EXIFIFD: u32 = 34665; // Pointer to EXIF private directory
// tag 34750 is a private tag registered to Adobe?
pub(crate) const TIFFTAG_ICCPROFILE: u32 = 34675; // ICC profile data
pub(crate) const TIFFTAG_IMAGELAYER: u32 = 34732; // !TIFF/FX image layer information
// tag 34750 is a private tag registered to Pixel Magic
pub(crate) const TIFFTAG_JBIGOPTIONS: u32 = 34750; // JBIG options
pub(crate) const TIFFTAG_GPSIFD: u32 = 34853; // Pointer to EXIF GPS private directory
// tags 34908-34914 are private tags registered to SGI
pub(crate) const TIFFTAG_FAXRECVPARAMS: u32 = 34908; // encoded Class 2 ses. params
pub(crate) const TIFFTAG_FAXSUBADDRESS: u32 = 34909; // received SubAddr string
pub(crate) const TIFFTAG_FAXRECVTIME: u32 = 34910; // receive time (secs)
pub(crate) const TIFFTAG_FAXDCS: u32 = 34911; // encoded fax ses. params, Table 2/T.30
// tags 37439-37443 are registered to SGI <gregl@sgi.com>
pub(crate) const TIFFTAG_STONITS: u32 = 37439; // Sample value to Nits
// tag 34929 is a private tag registered to FedEx
pub(crate) const TIFFTAG_FEDEX_EDR: u32 = 34929; // unknown use
pub(crate) const TIFFTAG_IMAGESOURCEDATA: u32 = 37724; // http://justsolve.archiveteam.org/wiki/PSD, http://www.adobe.com/devnet-apps/photoshop/fileformatashtml/
pub(crate) const TIFFTAG_INTEROPERABILITYIFD: u32 = 40965; // Pointer to EXIF Interoperability private directory
pub(crate) const TIFFTAG_GDAL_METADATA: u32 = 42112; // Used by the GDAL library
pub(crate) const TIFFTAG_GDAL_NODATA: u32 = 42113; // Used by the GDAL library
pub(crate) const TIFFTAG_OCE_SCANJOB_DESCRIPTION: u32 = 50215; // Used in the Oce scanning process
pub(crate) const TIFFTAG_OCE_APPLICATION_SELECTOR: u32 = 50216; // Used in the Oce scanning process.
pub(crate) const TIFFTAG_OCE_IDENTIFICATION_NUMBER: u32 = 50217;
pub(crate) const TIFFTAG_OCE_IMAGELOGIC_CHARACTERISTICS: u32 = 50218;
// tags 50674 to 50677 are reserved for ESRI
pub(crate) const TIFFTAG_LERC_PARAMETERS: u32 = 50674; // Stores LERC version and additional compression method
// Adobe Digital Negative (DNG) format tags
pub(crate) const TIFFTAG_DNGVERSION: u32 = 50706; // &DNG version number
pub(crate) const TIFFTAG_DNGBACKWARDVERSION: u32 = 50707; // &DNG compatibility version
pub(crate) const TIFFTAG_UNIQUECAMERAMODEL: u32 = 50708; // &name for the camera model
pub(crate) const TIFFTAG_LOCALIZEDCAMERAMODEL: u32 = 50709; // &localized camera model name (UTF-8)
pub(crate) const TIFFTAG_CFAPLANECOLOR: u32 = 50710; // &CFAPattern->LinearRaw space mapping
pub(crate) const TIFFTAG_CFALAYOUT: u32 = 50711; // &spatial layout of the CFA
pub(crate) const TIFFTAG_LINEARIZATIONTABLE: u32 = 50712; // &lookup table description
pub(crate) const TIFFTAG_BLACKLEVELREPEATDIM: u32 = 50713; // &repeat pattern size for the BlackLevel tag
pub(crate) const TIFFTAG_BLACKLEVEL: u32 = 50714; // &zero light encoding level
pub(crate) const TIFFTAG_BLACKLEVELDELTAH: u32 = 50715; // &zero light encoding level differences (columns)
pub(crate) const TIFFTAG_BLACKLEVELDELTAV: u32 = 50716; // &zero light encoding level differences (rows)
pub(crate) const TIFFTAG_WHITELEVEL: u32 = 50717; // &fully saturated encoding level
pub(crate) const TIFFTAG_DEFAULTSCALE: u32 = 50718; // &default scale factors
pub(crate) const TIFFTAG_DEFAULTCROPORIGIN: u32 = 50719; // &origin of the final image area
pub(crate) const TIFFTAG_DEFAULTCROPSIZE: u32 = 50720; // &size of the final image area
pub(crate) const TIFFTAG_COLORMATRIX1: u32 = 50721; // &XYZ->reference color space transformation matrix 1
pub(crate) const TIFFTAG_COLORMATRIX2: u32 = 50722; // &XYZ->reference color space transformation matrix 2
pub(crate) const TIFFTAG_CAMERACALIBRATION1: u32 = 50723; // &calibration matrix 1
pub(crate) const TIFFTAG_CAMERACALIBRATION2: u32 = 50724; // &calibration matrix 2
pub(crate) const TIFFTAG_REDUCTIONMATRIX1: u32 = 50725; // &dimensionality reduction matrix 1
pub(crate) const TIFFTAG_REDUCTIONMATRIX2: u32 = 50726; // &dimensionality reduction matrix 2
pub(crate) const TIFFTAG_ANALOGBALANCE: u32 = 50727; // &gain applied the stored raw values
pub(crate) const TIFFTAG_ASSHOTNEUTRAL: u32 = 50728; // &selected white balance in linear reference space
pub(crate) const TIFFTAG_ASSHOTWHITEXY: u32 = 50729; // &selected white balance in x-y chromaticity coordinates
pub(crate) const TIFFTAG_BASELINEEXPOSURE: u32 = 50730; // &how much to move the zero point
pub(crate) const TIFFTAG_BASELINENOISE: u32 = 50731; // &relative noise level
pub(crate) const TIFFTAG_BASELINESHARPNESS: u32 = 50732; // &relative amount of sharpening
pub(crate) const TIFFTAG_BAYERGREENSPLIT: u32 = 50733;
pub(crate) const TIFFTAG_LINEARRESPONSELIMIT: u32 = 50734; // &non-linear encoding range
pub(crate) const TIFFTAG_CAMERASERIALNUMBER: u32 = 50735; // &camera's serial number
pub(crate) const TIFFTAG_LENSINFO: u32 = 50736; // info about the lens
pub(crate) const TIFFTAG_CHROMABLURRADIUS: u32 = 50737; // &chroma blur radius
pub(crate) const TIFFTAG_ANTIALIASSTRENGTH: u32 = 50738; // &relative strength of the camera's anti-alias filter
pub(crate) const TIFFTAG_SHADOWSCALE: u32 = 50739; // &used by Adobe Camera Raw
pub(crate) const TIFFTAG_DNGPRIVATEDATA: u32 = 50740; // &manufacturer's private data
pub(crate) const TIFFTAG_MAKERNOTESAFETY: u32 = 50741; // &whether the EXIF MakerNote tag is safe to preserve along with the rest of the EXIF data
pub(crate) const TIFFTAG_CALIBRATIONILLUMINANT1: u32 = 50778; // &illuminant 1
pub(crate) const TIFFTAG_CALIBRATIONILLUMINANT2: u32 = 50779; // &illuminant 2
pub(crate) const TIFFTAG_BESTQUALITYSCALE: u32 = 50780; // &best quality multiplier
pub(crate) const TIFFTAG_RAWDATAUNIQUEID: u32 = 50781; // &unique identifier for the raw image data
pub(crate) const TIFFTAG_ORIGINALRAWFILENAME: u32 = 50827; // &file name of the original raw file (UTF-8)
pub(crate) const TIFFTAG_ORIGINALRAWFILEDATA: u32 = 50828; // &contents of the original raw file
pub(crate) const TIFFTAG_ACTIVEAREA: u32 = 50829; // &active (non-masked) pixels of the sensor
pub(crate) const TIFFTAG_MASKEDAREAS: u32 = 50830; // &list of coordinates of fully masked pixels
pub(crate) const TIFFTAG_ASSHOTICCPROFILE: u32 = 50831; // &these two tags used to
pub(crate) const TIFFTAG_ASSHOTPREPROFILEMATRIX: u32 = 50832; // map cameras's color space  into ICC profile space
pub(crate) const TIFFTAG_CURRENTICCPROFILE: u32 = 50833; // &
pub(crate) const TIFFTAG_CURRENTPREPROFILEMATRIX: u32 = 50834; // &
// DNG 1.2.0.0
pub(crate) const TIFFTAG_COLORIMETRICREFERENCE: u32 = 50879; // &colorimetric reference
pub(crate) const TIFFTAG_CAMERACALIBRATIONSIGNATURE: u32 = 50931; // &camera calibration signature (UTF-8)
pub(crate) const TIFFTAG_PROFILECALIBRATIONSIGNATURE: u32 = 50932; // &profile calibration signature (UTF-8)
// TIFFTAG_EXTRACAMERAPROFILES 50933 &extra camera profiles : is already defined for GeoTIFF DGIWG
pub(crate) const TIFFTAG_ASSHOTPROFILENAME: u32 = 50934; // &as shot profile name (UTF-8)
pub(crate) const TIFFTAG_NOISEREDUCTIONAPPLIED: u32 = 50935; // &amount of applied noise reduction
pub(crate) const TIFFTAG_PROFILENAME: u32 = 50936; // &camera profile name (UTF-8)
pub(crate) const TIFFTAG_PROFILEHUESATMAPDIMS: u32 = 50937; // &dimensions of HSV mapping
pub(crate) const TIFFTAG_PROFILEHUESATMAPDATA1: u32 = 50938; // &first HSV mapping table
pub(crate) const TIFFTAG_PROFILEHUESATMAPDATA2: u32 = 50939; // &second HSV mapping table
pub(crate) const TIFFTAG_PROFILETONECURVE: u32 = 50940; // &default tone curve
pub(crate) const TIFFTAG_PROFILEEMBEDPOLICY: u32 = 50941; // &profile embedding policy
pub(crate) const TIFFTAG_PROFILECOPYRIGHT: u32 = 50942; // &profile copyright information (UTF-8)
pub(crate) const TIFFTAG_FORWARDMATRIX1: u32 = 50964; // &matrix for mapping white balanced camera colors to XYZ D50
pub(crate) const TIFFTAG_FORWARDMATRIX2: u32 = 50965; // &matrix for mapping white balanced camera colors to XYZ D50
pub(crate) const TIFFTAG_PREVIEWAPPLICATIONNAME: u32 = 50966; // &name of application that created preview (UTF-8)
pub(crate) const TIFFTAG_PREVIEWAPPLICATIONVERSION: u32 = 50967; // &version of application that created preview (UTF-8)
pub(crate) const TIFFTAG_PREVIEWSETTINGSNAME: u32 = 50968; // &name of conversion settings (UTF-8)
pub(crate) const TIFFTAG_PREVIEWSETTINGSDIGEST: u32 = 50969; // &unique id of conversion settings
pub(crate) const TIFFTAG_PREVIEWCOLORSPACE: u32 = 50970; // &preview color space
pub(crate) const TIFFTAG_PREVIEWDATETIME: u32 = 50971; // &date/time preview was rendered
pub(crate) const TIFFTAG_RAWIMAGEDIGEST: u32 = 50972; // &md5 of raw image data
pub(crate) const TIFFTAG_ORIGINALRAWFILEDIGEST: u32 = 50973; // &md5 of the data stored in the OriginalRawFileData tag
pub(crate) const TIFFTAG_SUBTILEBLOCKSIZE: u32 = 50974; // &subtile block size
pub(crate) const TIFFTAG_ROWINTERLEAVEFACTOR: u32 = 50975; // &number of interleaved fields
pub(crate) const TIFFTAG_PROFILELOOKTABLEDIMS: u32 = 50981; // &num of input samples in each dim of default "look" table
pub(crate) const TIFFTAG_PROFILELOOKTABLEDATA: u32 = 50982; // &default "look" table for use as starting point
// DNG 1.3.0.0
pub(crate) const TIFFTAG_OPCODELIST1: u32 = 51008; // &opcodes that should be applied to raw image after reading
pub(crate) const TIFFTAG_OPCODELIST2: u32 = 51009; // &opcodes that should be applied after mapping to linear reference
pub(crate) const TIFFTAG_OPCODELIST3: u32 = 51022; // &opcodes that should be applied after demosaicing
pub(crate) const TIFFTAG_NOISEPROFILE: u32 = 51041; // &noise profile
// DNG 1.4.0.0
pub(crate) const TIFFTAG_DEFAULTUSERCROP: u32 = 51125; // &default user crop rectangle in relative coords
pub(crate) const TIFFTAG_DEFAULTBLACKRENDER: u32 = 51110; // &black rendering hint
pub(crate) const TIFFTAG_BASELINEEXPOSUREOFFSET: u32 = 51109; // &baseline exposure offset
pub(crate) const TIFFTAG_PROFILELOOKTABLEENCODING: u32 = 51108; // &3D LookTable indexing conversion
pub(crate) const TIFFTAG_PROFILEHUESATMAPENCODING: u32 = 51107; // &3D HueSatMap indexing conversion
pub(crate) const TIFFTAG_ORIGINALDEFAULTFINALSIZE: u32 = 51089; // &default final size of larger original file for this proxy
pub(crate) const TIFFTAG_ORIGINALBESTQUALITYFINALSIZE: u32 = 51090; // &best quality final size of larger original file for this proxy
pub(crate) const TIFFTAG_ORIGINALDEFAULTCROPSIZE: u32 = 51091; // &the default crop size of larger original file for this proxy
pub(crate) const TIFFTAG_NEWRAWIMAGEDIGEST: u32 = 51111; // &modified MD5 digest of the raw image data
pub(crate) const TIFFTAG_RAWTOPREVIEWGAIN: u32 = 51112; // &The gain between the main raw FD and the preview IFD containing this tag
// DNG 1.5.0.0
pub(crate) const TIFFTAG_DEPTHFORMAT: u32 = 51177; // &encoding of the depth data in the file
pub(crate) const TIFFTAG_DEPTHNEAR: u32 = 51178; // &distance from the camera represented by value 0 in the depth map
pub(crate) const TIFFTAG_DEPTHFAR: u32 = 51179; // &distance from the camera represented by the maximum value in the depth map
pub(crate) const TIFFTAG_DEPTHUNITS: u32 = 51180; // &measurement units for DepthNear and DepthFar
pub(crate) const TIFFTAG_DEPTHMEASURETYPE: u32 = 51181; // &measurement geometry for the depth map
pub(crate) const TIFFTAG_ENHANCEPARAMS: u32 = 51182; // &a string that documents how the enhanced image data was processed.
// DNG 1.6.0.0
pub(crate) const TIFFTAG_PROFILEGAINTABLEMAP: u32 = 52525; // &spatially varying gain tables that can be applied as starting point
pub(crate) const TIFFTAG_SEMANTICNAME: u32 = 52526; // &a string that identifies the semantic mask
pub(crate) const TIFFTAG_SEMANTICINSTANCEID: u32 = 52528; // &a string that identifies a specific instance in a semantic mask
pub(crate) const TIFFTAG_MASKSUBAREA: u32 = 52536; // &the crop rectangle of this IFD's mask, relative to the main image
pub(crate) const TIFFTAG_RGBTABLES: u32 = 52543; // &color transforms to apply to masked image regions
pub(crate) const TIFFTAG_CALIBRATIONILLUMINANT3: u32 = 52529; // &the illuminant used for the third set of color calibration tags
pub(crate) const TIFFTAG_COLORMATRIX3: u32 = 52531; // &matrix to convert XYZ values to reference camera native color space under CalibrationIlluminant3
pub(crate) const TIFFTAG_CAMERACALIBRATION3: u32 = 52530; // &matrix to transform reference camera native space values to individual camera native space values under CalibrationIlluminant3
pub(crate) const TIFFTAG_REDUCTIONMATRIX3: u32 = 52538; // &dimensionality reduction matrix for use in color conversion to XYZ under CalibrationIlluminant3
pub(crate) const TIFFTAG_PROFILEHUESATMAPDATA3: u32 = 52537; // &the data for the third HSV table
pub(crate) const TIFFTAG_FORWARDMATRIX3: u32 = 52532; // &matrix to map white balanced camera colors to XYZ D50
pub(crate) const TIFFTAG_ILLUMINANTDATA1: u32 = 52533; // &data for the first calibration illuminant
pub(crate) const TIFFTAG_ILLUMINANTDATA2: u32 = 52534; // &data for the second calibration illuminant
pub(crate) const TIFFTAG_ILLUMINANTDATA3: u32 = 53535; // &data for the third calibration illuminant
// TIFF/EP
pub(crate) const TIFFTAG_EP_CFAREPEATPATTERNDIM: u32 = 33421; // dimensions of CFA pattern
pub(crate) const TIFFTAG_EP_CFAPATTERN: u32 = 33422; // color filter array pattern
pub(crate) const TIFFTAG_EP_BATTERYLEVEL: u32 = 33423; // battery level (rational or ASCII)
pub(crate) const TIFFTAG_EP_INTERLACE: u32 = 34857; // Number of multi-field images
pub(crate) const TIFFTAG_EP_IPTC_NAA: u32 = 33723; // Alias IPTC/NAA Newspaper Association RichTIFF
pub(crate) const TIFFTAG_EP_TIMEZONEOFFSET: u32 = 34858; // Time zone offset relative to UTC
pub(crate) const TIFFTAG_EP_SELFTIMERMODE: u32 = 34859; // Number of seconds capture was delayed from button press
pub(crate) const TIFFTAG_EP_FLASHENERGY: u32 = 37387; // Flash energy, or range if there is uncertainty
pub(crate) const TIFFTAG_EP_SPATIALFREQUENCYRESPONSE: u32 = 37388; // Spatial frequency response
pub(crate) const TIFFTAG_EP_NOISE: u32 = 37389; // Camera noise measurement values
pub(crate) const TIFFTAG_EP_FOCALPLANEXRESOLUTION: u32 = 37390; // Focal plane X resolution
pub(crate) const TIFFTAG_EP_FOCALPLANEYRESOLUTION: u32 = 37391; // Focal plane Y resolution
pub(crate) const TIFFTAG_EP_FOCALPLANERESOLUTIONUNIT: u32 = 37392; // Focal plane resolution unit
pub(crate) const TIFFTAG_EP_IMAGENUMBER: u32 = 37393; // Number of image when several of burst shot stored in same TIFF/EP
pub(crate) const TIFFTAG_EP_SECURITYCLASSIFICATION: u32 = 37394; // Security classification
pub(crate) const TIFFTAG_EP_IMAGEHISTORY: u32 = 37395; // Record of what has been done to the image
pub(crate) const TIFFTAG_EP_EXPOSUREINDEX: u32 = 37397; // Exposure index
pub(crate) const TIFFTAG_EP_STANDARDID: u32 = 37398; // TIFF/EP standard version, n.n.n.n
pub(crate) const TIFFTAG_EP_SENSINGMETHOD: u32 = 37399; // Type of image sensor
pub(crate) const TIFFTAG_EP_EXPOSURETIME: u32 = 33434; // Exposure time
pub(crate) const TIFFTAG_EP_FNUMBER: u32 = 33437; // F number
pub(crate) const TIFFTAG_EP_EXPOSUREPROGRAM: u32 = 34850; // Exposure program
pub(crate) const TIFFTAG_EP_SPECTRALSENSITIVITY: u32 = 34852; // Spectral sensitivity
pub(crate) const TIFFTAG_EP_ISOSPEEDRATINGS: u32 = 34855; // ISO speed rating
pub(crate) const TIFFTAG_EP_OECF: u32 = 34856; // Optoelectric conversion factor
pub(crate) const TIFFTAG_EP_DATETIMEORIGINAL: u32 = 36867; // Date and time of original data generation
pub(crate) const TIFFTAG_EP_COMPRESSEDBITSPERPIXEL: u32 = 37122; // Image compression mode
pub(crate) const TIFFTAG_EP_SHUTTERSPEEDVALUE: u32 = 37377; // Shutter speed
pub(crate) const TIFFTAG_EP_APERTUREVALUE: u32 = 37378; // Aperture
pub(crate) const TIFFTAG_EP_BRIGHTNESSVALUE: u32 = 37379; // Brightness
pub(crate) const TIFFTAG_EP_EXPOSUREBIASVALUE: u32 = 37380; // Exposure bias
pub(crate) const TIFFTAG_EP_MAXAPERTUREVALUE: u32 = 37381; // Maximum lens aperture
pub(crate) const TIFFTAG_EP_SUBJECTDISTANCE: u32 = 37382; // Subject distance
pub(crate) const TIFFTAG_EP_METERINGMODE: u32 = 37383; // Metering mode
pub(crate) const TIFFTAG_EP_LIGHTSOURCE: u32 = 37384; // Light source
pub(crate) const TIFFTAG_EP_FLASH: u32 = 37385; // Flash
pub(crate) const TIFFTAG_EP_FOCALLENGTH: u32 = 37386; // Lens focal length
pub(crate) const TIFFTAG_EP_SUBJECTLOCATION: u32 = 37396; // Subject location (area)
pub(crate) const TIFFTAG_RPCCOEFFICIENT: u32 = 50844; // Define by GDAL for geospatial georeferencing through RPC: http://geotiff.maptools.org/rpc_prop.html
pub(crate) const TIFFTAG_ALIAS_LAYER_METADATA: u32 = 50784; // Alias Sketchbook Pro layer usage description.
// GeoTIFF DGIWG
pub(crate) const TIFFTAG_TIFF_RSID: u32 = 50908; // https://www.awaresystems.be/imaging/tiff/tifftags/tiff_rsid.html
pub(crate) const TIFFTAG_GEO_METADATA: u32 = 50909; // https://www.awaresystems.be/imaging/tiff/tifftags/geo_metadata.html
pub(crate) const TIFFTAG_EXTRACAMERAPROFILES: u32 = 50933; // http://wwwimages.adobe.com/www.adobe.com/content/dam/Adobe/en/products/photoshop/pdfs/dng_spec_1.4.0.0.pdf
// tag 65535 is an undefined tag used by Eastman Kodak
pub(crate) const TIFFTAG_DCSHUESHIFTVALUES: u32 = 65535; // hue shift correction data
pub(crate) const TIFFTAG_FAXMODE: u32 = 65536; // Group 3/4 format control
pub(crate) const FAXMODE_CLASSIC: i32 = 0x0000; // default, include RTC
pub(crate) const FAXMODE_NORTC: i32 = 0x0001; // no RTC at end of data
pub(crate) const FAXMODE_NOEOL: i32 = 0x0002; // no EOL code at end of row
pub(crate) const FAXMODE_BYTEALIGN: i32 = 0x0004; // byte align row
pub(crate) const FAXMODE_WORDALIGN: i32 = 0x0008; // word align row
pub(crate) const FAXMODE_CLASSF: i32 = FAXMODE_NORTC; // TIFF Class F
pub(crate) const TIFFTAG_JPEGQUALITY: u32 = 65537; // Compression quality level
// Note: quality level is on the IJG 0-100 scale.  Default value is 75
pub(crate) const TIFFTAG_JPEGCOLORMODE: u32 = 65538; // Auto RGB<=>YCbCr convert?
pub(crate) const JPEGCOLORMODE_RAW: u16 = 0x0000; // no conversion (default)
pub(crate) const JPEGCOLORMODE_RGB: u16 = 0x0001; // do auto conversion
pub(crate) const TIFFTAG_JPEGTABLESMODE: u32 = 65539; // What to put in JPEGTables
pub(crate) const JPEGTABLESMODE_QUANT: u32 = 0x0001; // include quantization tbls
pub(crate) const JPEGTABLESMODE_HUFF: u32 = 0x0002; // include Huffman tbls
// Note: default is JPEGTABLESMODE_QUANT | JPEGTABLESMODE_HUFF
pub(crate) const TIFFTAG_FAXFILLFUNC: u32 = 65540; // G3/G4 fill function
pub(crate) const TIFFTAG_PIXARLOGDATAFMT: u32 = 65549; // PixarLogCodec I/O data sz
pub(crate) const PIXARLOGDATAFMT_8BIT: u16 = 0; // regular u_char samples
pub(crate) const PIXARLOGDATAFMT_8BITABGR: u16 = 1; // ABGR-order u_chars
pub(crate) const PIXARLOGDATAFMT_11BITLOG: u16 = 2; // 11-bit log-encoded (raw)
pub(crate) const PIXARLOGDATAFMT_12BITPICIO: u16 = 3; // as per PICIO (1.0==2048)
pub(crate) const PIXARLOGDATAFMT_16BIT: u16 = 4; // signed short samples
pub(crate) const PIXARLOGDATAFMT_FLOAT: u16 = 5; // IEEE float samples
// 65550-65556 are allocated to Oceana Matrix <dev@oceana.com>
pub(crate) const TIFFTAG_DCSIMAGERTYPE: u32 = 65550; // imager model & filter
pub(crate) const DCSIMAGERMODEL_M3: u16 = 0; // M3 chip (1280 x 1024)
pub(crate) const DCSIMAGERMODEL_M5: u16 = 1; // M5 chip (1536 x 1024)
pub(crate) const DCSIMAGERMODEL_M6: u16 = 2; // M6 chip (3072 x 2048)
pub(crate) const DCSIMAGERFILTER_IR: u16 = 0; // infrared filter
pub(crate) const DCSIMAGERFILTER_MONO: u16 = 1; // monochrome filter
pub(crate) const DCSIMAGERFILTER_CFA: u16 = 2; // color filter array
pub(crate) const DCSIMAGERFILTER_OTHER: u16 = 3; // other filter
pub(crate) const TIFFTAG_DCSINTERPMODE: u32 = 65551; // interpolation mode
pub(crate) const DCSINTERPMODE_NORMAL: u16 = 0x0; // whole image, default
pub(crate) const DCSINTERPMODE_PREVIEW: u16 = 0x1; // preview of image (384x256)
pub(crate) const TIFFTAG_DCSBALANCEARRAY: u32 = 65552; // color balance values
pub(crate) const TIFFTAG_DCSCORRECTMATRIX: u32 = 65553; // color correction values
pub(crate) const TIFFTAG_DCSGAMMA: u32 = 65554; // gamma value
pub(crate) const TIFFTAG_DCSTOESHOULDERPTS: u32 = 65555; // toe & shoulder points
pub(crate) const TIFFTAG_DCSCALIBRATIONFD: u32 = 65556; // calibration file desc
// Note: quality level is on the ZLIB 1-9 scale. Default value is -1
pub(crate) const TIFFTAG_ZIPQUALITY: u32 = 65557; // compression quality level
pub(crate) const TIFFTAG_PIXARLOGQUALITY: u32 = 65558; // PixarLog uses same scale
// 65559 is allocated to Oceana Matrix <dev@oceana.com>
pub(crate) const TIFFTAG_DCSCLIPRECTANGLE: u32 = 65559; // area of image to acquire
pub(crate) const TIFFTAG_SGILOGDATAFMT: u32 = 65560; // SGILog user data format
pub(crate) const SGILOGDATAFMT_FLOAT: u16 = 0; // IEEE float samples
pub(crate) const SGILOGDATAFMT_16BIT: u16 = 1; // 16-bit samples
pub(crate) const SGILOGDATAFMT_RAW: u16 = 2; // uninterpreted data
pub(crate) const SGILOGDATAFMT_8BIT: u16 = 3; // 8-bit RGB monitor values
pub(crate) const TIFFTAG_SGILOGENCODE: u32 = 65561; // SGILog data encoding control
pub(crate) const SGILOGENCODE_NODITHER: u16 = 0; // do not dither encoded values
pub(crate) const SGILOGENCODE_RANDITHER: u16 = 1; // randomly dither encd values
pub(crate) const TIFFTAG_LZMAPRESET: u32 = 65562; // LZMA2 preset (compression level)
pub(crate) const TIFFTAG_PERSAMPLE: u32 = 65563; // interface for per sample tags
pub(crate) const PERSAMPLE_MERGED: u16 = 0; // present as a single value
pub(crate) const PERSAMPLE_MULTI: u16 = 1; // present as multiple values
pub(crate) const TIFFTAG_ZSTD_LEVEL: u32 = 65564; // ZSTD compression level
pub(crate) const TIFFTAG_LERC_VERSION: u32 = 65565; // LERC version
pub(crate) const LERC_VERSION_2_4: u16 = 4;
pub(crate) const TIFFTAG_LERC_ADD_COMPRESSION: u32 = 65566; // LERC additional compression
pub(crate) const LERC_ADD_COMPRESSION_NONE: u16 = 0;
pub(crate) const LERC_ADD_COMPRESSION_DEFLATE: u16 = 1;
pub(crate) const LERC_ADD_COMPRESSION_ZSTD: u16 = 2;
pub(crate) const TIFFTAG_LERC_MAXZERROR: u32 = 65567; // LERC maximum error
pub(crate) const TIFFTAG_WEBP_LEVEL: u32 = 65568; // WebP compression level
pub(crate) const TIFFTAG_WEBP_LOSSLESS: u32 = 65569; // WebP lossless/lossy
pub(crate) const TIFFTAG_WEBP_LOSSLESS_EXACT: u32 = 65571; // WebP lossless exact mode. Set-only mode. Default is 1. Can be set to 0 to increase compression rate, but R,G,B in areas where alpha = 0 will not be preserved
pub(crate) const TIFFTAG_DEFLATE_SUBCODEC: u32 = 65570; // ZIP codec: to get/set the sub-codec to use. Will default to libdeflate when available
pub(crate) const DEFLATE_SUBCODEC_ZLIB: u16 = 0;
pub(crate) const DEFLATE_SUBCODEC_LIBDEFLATE: u16 = 1;
