// Rust translation of libtiff/tif_predict.c and libtiff/tif_predict.h from
// libtiff (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's
// external/libtiff pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Predictor Tag Support (used by multiple codecs).
//!
//! ``Library-private'' Support for the Predictor Tag: the decoding side
//! (horizontal accumulation and the floating point predictor; the
//! differencing of the encoder is left out). The accumulators are named
//! by a [`DecodePFunc`], as the C compares them. The SSE2 path of
//! `fpAcc()` (an optimization giving the same bytes) is not translated.

use super::tif_dir::{tiff_set_field_bit, Gv, TIFFVGetMethod, TIFFVSetMethod, VaList, FIELD_CODEC};
use super::tif_dirinfo::_tiff_merge_fields;
use super::tif_error::tiff_error_ext_r;
use super::tif_strip::tiff_scanline_size;
use super::tif_swab::{
    tiff_swab_array_of_long, tiff_swab_array_of_long8, tiff_swab_array_of_short,
};
use super::tif_tile::tiff_tile_row_size;
use super::tiff::*;
use super::tiffio::{field, TIFFField};
use super::tiffiop::{
    TIFFBoolMethod, TIFFCodeMethod, TIFFPostMethod, TifData, Tiff, TmSize, TIFF_DIRTYDIRECT,
    TIFF_SWAB,
};

/// `TIFFEncodeDecodeMethod`: the horizontal accumulators.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DecodePFunc {
    HorAcc8,
    HorAcc16,
    HorAcc32,
    HorAcc64,
    SwabHorAcc16,
    SwabHorAcc32,
    SwabHorAcc64,
    FpAcc,
}

/// Translation of `TIFFPredictorState`: Codecs that want to support the
/// Predictor tag must place this structure first in their private state
/// block so that the predictor code can cast tif_data to find its state.
/// (The encoding methods are left out.)
#[derive(Clone, Copy)]
pub(crate) struct TIFFPredictorState {
    pub(crate) predictor: i32,                   /* predictor tag value */
    pub(crate) stride: TmSize,                   /* sample stride over data */
    pub(crate) rowsize: TmSize,                  /* tile/strip row size */
    pub(crate) decoderow: TIFFCodeMethod,        /* parent codec encode/decode row */
    pub(crate) decodestrip: TIFFCodeMethod,      /* parent codec encode/decode strip */
    pub(crate) decodetile: TIFFCodeMethod,       /* parent codec encode/decode tile */
    pub(crate) decodepfunc: Option<DecodePFunc>, /* horizontal accumulator */
    pub(crate) vgetparent: TIFFVGetMethod,       /* super-class method */
    pub(crate) vsetparent: TIFFVSetMethod,       /* super-class method */
    pub(crate) setupdecode: TIFFBoolMethod,      /* super-class method */
}

impl TIFFPredictorState {
    /// A state to be initialized by `TIFFPredictorInit()`.
    pub(crate) fn new(tif: &Tiff<'_>) -> Self {
        TIFFPredictorState {
            predictor: 0,
            stride: 0,
            rowsize: 0,
            decoderow: tif.tif_decoderow,
            decodestrip: tif.tif_decodestrip,
            decodetile: tif.tif_decodetile,
            decodepfunc: None,
            vgetparent: tif.tif_tagmethods.vgetfield,
            vsetparent: tif.tif_tagmethods.vsetfield,
            setupdecode: tif.tif_setupdecode,
        }
    }
}

/// Translation of `PredictorState()`: the predictor state at the start of
/// the codec's private data.
fn predictor_state<'s>(tif: &'s mut Tiff<'_>) -> Option<&'s mut TIFFPredictorState> {
    match &mut tif.tif_data {
        TifData::Lzw(sp) => Some(&mut sp.predict),
        _ => None,
    }
}

/// A copy of the predictor state (the C's `sp` while it calls back into
/// the handle).
fn predictor_state_copy(tif: &mut Tiff<'_>) -> Option<TIFFPredictorState> {
    predictor_state(tif).copied()
}

/// Translation of `PredictorSetup()`.
fn predictor_setup(tif: &mut Tiff<'_>) -> i32 {
    const MODULE: &str = "PredictorSetup";

    let Some(sp) = predictor_state_copy(tif) else {
        return 0;
    };
    let td = &tif.tif_dir;

    match sp.predictor {
        /* no differencing */
        p if p == PREDICTOR_NONE as i32 => return 1,
        p if p == PREDICTOR_HORIZONTAL as i32 => {
            if td.td_bitspersample != 8
                && td.td_bitspersample != 16
                && td.td_bitspersample != 32
                && td.td_bitspersample != 64
            {
                tiff_error_ext_r!(
                    MODULE,
                    "Horizontal differencing \"Predictor\" not supported with {}-bit samples",
                    td.td_bitspersample
                );
                return 0;
            }
        }
        p if p == PREDICTOR_FLOATINGPOINT as i32 => {
            if td.td_sampleformat != SAMPLEFORMAT_IEEEFP {
                tiff_error_ext_r!(
                    MODULE,
                    "Floating point \"Predictor\" not supported with {} data format",
                    td.td_sampleformat
                );
                return 0;
            }
            if td.td_bitspersample != 16
                && td.td_bitspersample != 24
                && td.td_bitspersample != 32
                && td.td_bitspersample != 64
            {
                /* Should 64 be allowed? */
                tiff_error_ext_r!(
                    MODULE,
                    "Floating point \"Predictor\" not supported with {}-bit samples",
                    td.td_bitspersample
                );
                return 0;
            }
        }
        _ => {
            tiff_error_ext_r!(MODULE, "\"Predictor\" value {} not supported", sp.predictor);
            return 0;
        }
    }
    let stride = if td.td_planarconfig == PLANARCONFIG_CONTIG {
        td.td_samplesperpixel as TmSize
    } else {
        1
    };
    /*
     * Calculate the scanline/tile-width size in bytes.
     */
    let rowsize = if tif.is_tiled() {
        tiff_tile_row_size(tif)
    } else {
        tiff_scanline_size(tif)
    };
    if let Some(sp) = predictor_state(tif) {
        sp.stride = stride;
        sp.rowsize = rowsize;
    }
    if rowsize == 0 {
        return 0;
    }

    1
}

/// Translation of `PredictorSetupDecode()`.
fn predictor_setup_decode(tif: &mut Tiff<'_>) -> i32 {
    let Some(sp) = predictor_state_copy(tif) else {
        return 0;
    };

    /* Note: when PredictorSetup() fails, the effets of setupdecode() */
    /* will not be "canceled" so setupdecode() might be robust to */
    /* be called several times. */
    if (sp.setupdecode)(tif) == 0 || predictor_setup(tif) == 0 {
        return 0;
    }

    let Some(mut sp) = predictor_state_copy(tif) else {
        return 0;
    };
    let bitspersample = tif.tif_dir.td_bitspersample;
    if sp.predictor == 2 {
        match bitspersample {
            8 => sp.decodepfunc = Some(DecodePFunc::HorAcc8),
            16 => sp.decodepfunc = Some(DecodePFunc::HorAcc16),
            32 => sp.decodepfunc = Some(DecodePFunc::HorAcc32),
            64 => sp.decodepfunc = Some(DecodePFunc::HorAcc64),
            _ => {}
        }
        /*
         * Override default decoding method with one that does the
         * predictor stuff.
         */
        if tif.tif_decoderow as usize != predictor_decode_row as TIFFCodeMethod as usize {
            sp.decoderow = tif.tif_decoderow;
            tif.tif_decoderow = predictor_decode_row;
            sp.decodestrip = tif.tif_decodestrip;
            tif.tif_decodestrip = predictor_decode_tile;
            sp.decodetile = tif.tif_decodetile;
            tif.tif_decodetile = predictor_decode_tile;
        }

        /*
         * If the data is horizontally differenced 16-bit data that
         * requires byte-swapping, then it must be byte swapped before
         * the accumulation step.  We do this with a special-purpose
         * routine and override the normal post decoding logic that
         * the library setup when the directory was read.
         */
        if (tif.tif_flags & TIFF_SWAB) != 0 {
            if sp.decodepfunc == Some(DecodePFunc::HorAcc16) {
                sp.decodepfunc = Some(DecodePFunc::SwabHorAcc16);
                tif.tif_postdecode = TIFFPostMethod::NoPostDecode;
            } else if sp.decodepfunc == Some(DecodePFunc::HorAcc32) {
                sp.decodepfunc = Some(DecodePFunc::SwabHorAcc32);
                tif.tif_postdecode = TIFFPostMethod::NoPostDecode;
            } else if sp.decodepfunc == Some(DecodePFunc::HorAcc64) {
                sp.decodepfunc = Some(DecodePFunc::SwabHorAcc64);
                tif.tif_postdecode = TIFFPostMethod::NoPostDecode;
            }
        }
    } else if sp.predictor == 3 {
        sp.decodepfunc = Some(DecodePFunc::FpAcc);
        /*
         * Override default decoding method with one that does the
         * predictor stuff.
         */
        if tif.tif_decoderow as usize != predictor_decode_row as TIFFCodeMethod as usize {
            sp.decoderow = tif.tif_decoderow;
            tif.tif_decoderow = predictor_decode_row;
            sp.decodestrip = tif.tif_decodestrip;
            tif.tif_decodestrip = predictor_decode_tile;
            sp.decodetile = tif.tif_decodetile;
            tif.tif_decodetile = predictor_decode_tile;
        }
        /*
         * The data should not be swapped outside of the floating
         * point predictor, the accumulation routine should return
         * bytes in the native order.
         */
        if (tif.tif_flags & TIFF_SWAB) != 0 {
            tif.tif_postdecode = TIFFPostMethod::NoPostDecode;
        }
    }
    if let Some(s) = predictor_state(tif) {
        *s = sp;
    }

    1
}

/* Remarks related to C standard compliance in all below functions : */
/* - to avoid any undefined behavior, we only operate on unsigned types */
/*   since the behavior of "overflows" is defined (wrap over) */
/* - when storing into the byte stream, we explicitly mask with 0xff so */
/*   as to make icc -check=conversions happy (not necessary by the standard) */

/// The accumulator named by `f`.
fn decode_pfunc(tif: &mut Tiff<'_>, f: DecodePFunc, cp0: &mut [u8], cc: TmSize) -> i32 {
    match f {
        DecodePFunc::HorAcc8 => hor_acc8(tif, cp0, cc),
        DecodePFunc::HorAcc16 => hor_acc16(tif, cp0, cc),
        DecodePFunc::HorAcc32 => hor_acc32(tif, cp0, cc),
        DecodePFunc::HorAcc64 => hor_acc64(tif, cp0, cc),
        DecodePFunc::SwabHorAcc16 => swab_hor_acc16(tif, cp0, cc),
        DecodePFunc::SwabHorAcc32 => swab_hor_acc32(tif, cp0, cc),
        DecodePFunc::SwabHorAcc64 => swab_hor_acc64(tif, cp0, cc),
        DecodePFunc::FpAcc => fp_acc(tif, cp0, cc),
    }
}

/// The predictor's sample stride.
fn stride_of(tif: &mut Tiff<'_>) -> TmSize {
    predictor_state(tif).map_or(1, |sp| sp.stride).max(1)
}

/// Translation of `horAcc8()`.
fn hor_acc8(tif: &mut Tiff<'_>, cp0: &mut [u8], cc: TmSize) -> i32 {
    let stride = stride_of(tif);

    let cp = cp0;
    if (cc % stride) != 0 {
        tiff_error_ext_r!("horAcc8", "{}", "(cc%stride)!=0");
        return 0;
    }

    if cc > stride {
        let cc = (cc as usize).min(cp.len());
        let stride = stride as usize;
        // (the C pipelines strides 1, 3 and 4: the same sums)
        for i in stride..cc {
            cp[i] = cp[i].wrapping_add(cp[i - stride]);
        }
    }
    1
}

/// Translation of `swabHorAcc16()`.
fn swab_hor_acc16(tif: &mut Tiff<'_>, cp0: &mut [u8], cc: TmSize) -> i32 {
    let wc = cc / 2;

    tiff_swab_array_of_short(cp0, wc);
    hor_acc16(tif, cp0, cc)
}

/// `horAcc16()`, `horAcc32()` and `horAcc64()`: accumulate the `N`-byte
/// host order words of the buffer.
fn hor_acc_n<const N: usize>(
    tif: &mut Tiff<'_>,
    cp0: &mut [u8],
    cc: TmSize,
    name: &str,
    what: &str,
) -> i32 {
    let stride = stride_of(tif);
    let wc = cc / N as TmSize;

    if (cc % (N as TmSize * stride)) != 0 {
        tiff_error_ext_r!(name, "{}", what);
        return 0;
    }

    if wc > stride {
        let stride = stride as usize;
        let wc = (wc as usize).min(cp0.len() / N);
        let get = |b: &[u8], i: usize| -> u64 {
            let mut a = [0u8; 8];
            a[..N].copy_from_slice(&b[i * N..i * N + N]);
            let v = u64::from_ne_bytes(a);
            if cfg!(target_endian = "big") {
                v >> (64 - 8 * N)
            } else {
                v
            }
        };
        for i in stride..wc {
            let v = get(cp0, i).wrapping_add(get(cp0, i - stride));
            let bytes = if cfg!(target_endian = "big") {
                (v << (64 - 8 * N)).to_ne_bytes()
            } else {
                v.to_ne_bytes()
            };
            cp0[i * N..i * N + N].copy_from_slice(&bytes[..N]);
        }
    }
    1
}

/// Translation of `horAcc16()`.
fn hor_acc16(tif: &mut Tiff<'_>, cp0: &mut [u8], cc: TmSize) -> i32 {
    hor_acc_n::<2>(tif, cp0, cc, "horAcc16", "cc%(2*stride))!=0")
}

/// Translation of `swabHorAcc32()`.
fn swab_hor_acc32(tif: &mut Tiff<'_>, cp0: &mut [u8], cc: TmSize) -> i32 {
    let wc = cc / 4;

    tiff_swab_array_of_long(cp0, wc);
    hor_acc32(tif, cp0, cc)
}

/// Translation of `horAcc32()`.
fn hor_acc32(tif: &mut Tiff<'_>, cp0: &mut [u8], cc: TmSize) -> i32 {
    hor_acc_n::<4>(tif, cp0, cc, "horAcc32", "cc%(4*stride))!=0")
}

/// Translation of `swabHorAcc64()`.
fn swab_hor_acc64(tif: &mut Tiff<'_>, cp0: &mut [u8], cc: TmSize) -> i32 {
    let wc = cc / 8;

    tiff_swab_array_of_long8(cp0, wc);
    hor_acc64(tif, cp0, cc)
}

/// Translation of `horAcc64()`.
fn hor_acc64(tif: &mut Tiff<'_>, cp0: &mut [u8], cc: TmSize) -> i32 {
    hor_acc_n::<8>(tif, cp0, cc, "horAcc64", "cc%(8*stride))!=0")
}

/// Translation of `fpAcc()`: Floating point predictor accumulation
/// routine.
fn fp_acc(tif: &mut Tiff<'_>, cp0: &mut [u8], cc: TmSize) -> i32 {
    let stride = stride_of(tif);
    let bps = (tif.tif_dir.td_bitspersample / 8) as TmSize;
    if bps == 0 {
        // (the C divides by zero; the setup only allows 16 to 64 bits)
        return 0;
    }
    let wc = cc / bps;

    if cc % (bps * stride) != 0 {
        tiff_error_ext_r!("fpAcc", "{}", "cc%(bps*stride))!=0");
        return 0;
    }

    let n = (cc.max(0) as usize).min(cp0.len());
    let cp = &mut cp0[..n];
    let stride = stride as usize;
    // (the C's stride == 1 case is an optimization of the same sums)
    for i in stride..n {
        cp[i] = cp[i].wrapping_add(cp[i - stride]);
    }

    let Some(tmp) = super::tiffiop::try_copy(cp) else {
        return 0;
    };

    let wc = wc as usize;
    let bps = bps as usize;
    for count in 0..wc {
        for byte in 0..bps {
            let src = if cfg!(target_endian = "big") {
                byte * wc + count
            } else {
                (bps - byte - 1) * wc + count
            };
            if let (Some(d), Some(&s)) = (cp.get_mut(bps * count + byte), tmp.get(src)) {
                *d = s;
            }
        }
    }
    1
}

/// Translation of `PredictorDecodeRow()`: Decode a scanline and apply the
/// predictor routine.
fn predictor_decode_row(tif: &mut Tiff<'_>, op0: &mut [u8], occ0: TmSize, s: u16) -> i32 {
    let Some(sp) = predictor_state_copy(tif) else {
        return 0;
    };

    if (sp.decoderow)(tif, op0, occ0, s) != 0 {
        match sp.decodepfunc {
            Some(f) => decode_pfunc(tif, f, op0, occ0),
            None => 0,
        }
    } else {
        0
    }
}

/// Translation of `PredictorDecodeTile()`: Decode a tile/strip and apply
/// the predictor routine. Note that horizontal differencing must be done
/// on a row-by-row basis.  The width of a "row" has already been
/// calculated at pre-decode time according to the strip/tile dimensions.
fn predictor_decode_tile(tif: &mut Tiff<'_>, op0: &mut [u8], mut occ0: TmSize, s: u16) -> i32 {
    let Some(sp) = predictor_state_copy(tif) else {
        return 0;
    };

    if (sp.decodetile)(tif, op0, occ0, s) != 0 {
        let rowsize = sp.rowsize;
        if rowsize <= 0 || (occ0 % rowsize) != 0 {
            tiff_error_ext_r!("PredictorDecodeTile", "{}", "occ0%rowsize != 0");
            return 0;
        }
        let Some(f) = sp.decodepfunc else {
            return 0;
        };
        let mut off = 0usize;
        while occ0 > 0 {
            let row = op0.get_mut(off..).unwrap_or(&mut []);
            if decode_pfunc(tif, f, row, rowsize) == 0 {
                return 0;
            }
            occ0 -= rowsize;
            off += rowsize as usize;
        }
        1
    } else {
        0
    }
}

const FIELD_PREDICTOR: u16 = FIELD_CODEC; /* XXX */

static PREDICT_FIELDS: [TIFFField; 1] = [field(
    TIFFTAG_PREDICTOR,
    1,
    1,
    TIFF_SHORT,
    0,
    super::tif_dir::TIFF_SETGET_UINT16,
    FIELD_PREDICTOR,
    0,
    0,
    "Predictor",
)];

/// Translation of `PredictorVSetField()`.
fn predictor_vset_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut VaList<'_>) -> i32 {
    let Some(sp) = predictor_state_copy(tif) else {
        return 0;
    };

    match tag {
        TIFFTAG_PREDICTOR => {
            let v = ap.int() as u16;
            if let Some(sp) = predictor_state(tif) {
                sp.predictor = v as i32;
            }
            tiff_set_field_bit(tif, FIELD_PREDICTOR);
        }
        _ => return (sp.vsetparent)(tif, tag, ap),
    }
    tif.tif_flags |= TIFF_DIRTYDIRECT;
    1
}

/// Translation of `PredictorVGetField()`.
fn predictor_vget_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut Vec<Gv>) -> i32 {
    let Some(sp) = predictor_state_copy(tif) else {
        return 0;
    };

    match tag {
        TIFFTAG_PREDICTOR => ap.push(Gv::U16(sp.predictor as u16)),
        _ => return (sp.vgetparent)(tif, tag, ap),
    }
    1
}

/// Translation of `TIFFPredictorInit()` (the directory printer and the
/// encoding setup are left out).
pub(crate) fn tiff_predictor_init(tif: &mut Tiff<'_>) -> i32 {
    /*
     * Merge codec-specific tag information.
     */
    if _tiff_merge_fields(tif, &PREDICT_FIELDS) == 0 {
        tiff_error_ext_r!(
            "TIFFPredictorInit",
            "Merging Predictor codec-specific tags failed"
        );
        return 0;
    }

    let vgetfield = tif.tif_tagmethods.vgetfield;
    let vsetfield = tif.tif_tagmethods.vsetfield;
    let setupdecode = tif.tif_setupdecode;
    let Some(sp) = predictor_state(tif) else {
        return 0;
    };
    /*
     * Override parent get/set field methods.
     */
    sp.vgetparent = vgetfield;
    sp.vsetparent = vsetfield;
    sp.setupdecode = setupdecode;

    sp.predictor = 1; /* default value */
    sp.decodepfunc = None; /* no predictor routine */

    tif.tif_tagmethods.vgetfield = predictor_vget_field; /* hook for predictor tag */
    tif.tif_tagmethods.vsetfield = predictor_vset_field; /* hook for predictor tag */
    tif.tif_setupdecode = predictor_setup_decode;
    1
}

/// Translation of `TIFFPredictorCleanup()`.
pub(crate) fn tiff_predictor_cleanup(tif: &mut Tiff<'_>) -> i32 {
    let Some(sp) = predictor_state_copy(tif) else {
        return 0;
    };

    tif.tif_tagmethods.vgetfield = sp.vgetparent;
    tif.tif_tagmethods.vsetfield = sp.vsetparent;
    tif.tif_setupdecode = sp.setupdecode;

    1
}
