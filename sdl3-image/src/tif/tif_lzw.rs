// Rust translation of libtiff/tif_lzw.c from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1988-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// Copyright (c) 1985, 1986 The Regents of the University of California.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Rev 5.0 Lempel-Ziv & Welch Compression Support
//!
//! This code is derived from the compress program whose code is
//! derived from software contributed to Berkeley by James A. Woods,
//! derived from original work by Spencer Thomas and Joseph Orost.
//!
//! The original Berkeley copyright notice appears in LICENSE.txt.
//!
//! The decoder (and the backwards compatible decoder of old,
//! bit-reversed files); the encoder is left out. The code table's
//! pointers are indices here (-1 for the C's `dec_codetab - 1` and
//! `NULL`), and the 64-bit `WordType` of 64-bit hosts is used everywhere.

use super::tif_compress::_tiff_set_default_compression_state;
use super::tif_error::{tiff_error_ext_r, tiff_warning_ext_r};
use super::tif_predict::{tiff_predictor_cleanup, tiff_predictor_init, TIFFPredictorState};
use super::tiffiop::{TifData, Tiff, TmSize};

/*
 * NB: The 5.0 spec describes a different algorithm than Aldus
 *     implements.  Specifically, Aldus does code length transitions
 *     one code earlier than should be done (for real LZW).
 *     Earlier versions of this library implemented the correct
 *     LZW algorithm, but emitted codes in a bit order opposite
 *     to the TIFF spec.  Thus, to maintain compatibility w/ Aldus
 *     we interpret MSB-LSB ordered codes to be images written w/
 *     old versions of this library, but otherwise adhere to the
 *     Aldus "off by one" algorithm.
 *
 * Future revisions to the TIFF spec are expected to "clarify this issue".
 */
// (LZW_COMPAT: include backwards compatibility code)

/* Select the plausible largest natural integer type for the architecture */
const SIZEOF_WORDTYPE: i64 = 8;
type WordType = u64;

/// Translation of `MAXCODE()`.
const fn maxcode(n: i64) -> i64 {
    (1 << n) - 1
}
/*
 * The TIFF spec specifies that encoded bit
 * strings range from 9 to 12 bits.
 */
const BITS_MIN: i64 = 9; /* start with 9 bits */
const BITS_MAX: i64 = 12; /* max of 12 bit strings */
/* predefined codes */
const CODE_CLEAR: i64 = 256; /* code to clear string table */
const CODE_EOI: i64 = 257; /* end-of-information code */
const CODE_FIRST: i64 = 258; /* first free code entry */
/* NB: +1024 is for compatibility with old files */
const CSIZE: i64 = maxcode(BITS_MAX) + 1024;

/// Translation of `code_t` (`struct code_ent`): Decoding-specific state.
#[derive(Clone, Copy, Debug, Default)]
struct CodeEnt {
    next: i32,     /* (an index, -1 for NULL) */
    length: u16,   /* string len, including this token */
    firstchar: u8, /* first token of string */
    value: u8,     /* data value */
    repeated: bool,
}

/// `decodeFunc`: regular or backwards compatible
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DecodeFunc {
    LzwDecode,
    LzwDecodeCompat,
}

/// Translation of `LZWCodecState` (with its `LZWBaseState`; the encoding
/// specific data left out). Note that the predictor state block must be
/// first in this data structure.
pub(crate) struct LZWCodecState {
    pub(crate) predict: TIFFPredictorState, /* predictor super class */

    lzw_nbits: u16,         /* # of bits/code */
    lzw_maxcode: u16,       /* maximum code for lzw_nbits */
    lzw_nextdata: WordType, /* next bits of i/o */
    lzw_nextbits: i64,      /* # of valid bits in lzw_nextdata */

    rw_mode: i32, /* preserve rw_mode from init */

    /* Decoding specific data */
    dec_nbitsmask: i64,  /* lzw_nbits 1 bits, right adjusted */
    dec_restart: TmSize, /* restart count */
    dec_bitsleft: u64,   /* available bits in raw data */
    old_tif_rawcc: TmSize, /* value of tif_rawcc at the end of the previous
                         TIFLZWDecode() call */
    dec_decode: Option<DecodeFunc>, /* regular or backwards compatible */
    dec_codep: i64,                 /* current recognized code */
    dec_oldcodep: i64,              /* previously recognized code */
    dec_free_entp: i64,             /* next free entry */
    dec_maxcodep: i64,              /* max available entry */
    dec_codetab: Option<Vec<CodeEnt>>, /* kept separate for small machines */
    read_error: i32,                /* whether a read error has occurred, and which should cause
                                    further reads in the same strip/tile to be aborted */
}

/// Translation of `LZWDecoderState()`.
fn lzw_decoder_state<'s>(tif: &'s mut Tiff<'_>) -> Option<&'s mut LZWCodecState> {
    match &mut tif.tif_data {
        TifData::Lzw(sp) => Some(sp),
        _ => None,
    }
}

/*
 * LZW Decoder.
 */

/// Translation of `LZWFixupTags()`.
fn lzw_fixup_tags(_tif: &mut Tiff<'_>) -> i32 {
    1
}

/// Translation of `LZWSetupDecode()`.
fn lzw_setup_decode(tif: &mut Tiff<'_>) -> i32 {
    const MODULE: &str = "LZWSetupDecode";

    // (the state block is always allocated by TIFFInitLZW())
    let Some(sp) = lzw_decoder_state(tif) else {
        tiff_error_ext_r!(MODULE, "No space for LZW state block");
        return 0;
    };

    if sp.dec_codetab.is_none() {
        let mut tab = Vec::new();
        if tab.try_reserve_exact(CSIZE as usize).is_err() {
            tiff_error_ext_r!(MODULE, "No space for LZW code table");
            return 0;
        }
        tab.resize(CSIZE as usize, CodeEnt::default());
        /*
         * Pre-load the table.
         */
        for code in (0..=255usize).rev() {
            tab[code].firstchar = code as u8;
            tab[code].value = code as u8;
            tab[code].repeated = true;
            tab[code].length = 1;
            tab[code].next = -1;
        }
        /*
         * Zero-out the unused entries  */
        /* Silence false positive */
        /* coverity[overrun-buffer-arg] */
        for e in &mut tab[CODE_CLEAR as usize..CODE_FIRST as usize] {
            *e = CodeEnt::default();
        }
        sp.dec_codetab = Some(tab);
    }
    1
}

/// Translation of `LZWPreDecode()`: Setup state for decoding a strip.
fn lzw_pre_decode(tif: &mut Tiff<'_>, _s: u16) -> i32 {
    const MODULE: &str = "LZWPreDecode";

    let has_codetab = lzw_decoder_state(tif).is_some_and(|sp| sp.dec_codetab.is_some());
    if !has_codetab {
        (tif.tif_setupdecode)(tif);
        if !lzw_decoder_state(tif).is_some_and(|sp| sp.dec_codetab.is_some()) {
            return 0;
        }
    }

    /*
     * Check for old bit-reversed codes.
     */
    let old_style = tif.tif_rawcc >= 2
        && tif.tif_rawdata.first() == Some(&0)
        && (tif.tif_rawdata.get(1).copied().unwrap_or(0) & 0x1) != 0;
    if old_style {
        let dec_decode = lzw_decoder_state(tif).and_then(|sp| sp.dec_decode);
        if dec_decode.is_none() {
            tiff_warning_ext_r!(MODULE, "Old-style LZW codes, convert file");
            /*
             * Override default decoding methods with
             * ones that deal with the old coding.
             * Otherwise the predictor versions set
             * above will call the compatibility routines
             * through the dec_decode method.
             */
            tif.tif_decoderow = lzw_decode_compat;
            tif.tif_decodestrip = lzw_decode_compat;
            tif.tif_decodetile = lzw_decode_compat;
            /*
             * If doing horizontal differencing, must
             * re-setup the predictor logic since we
             * switched the basic decoder methods...
             */
            (tif.tif_setupdecode)(tif);
            if let Some(sp) = lzw_decoder_state(tif) {
                sp.dec_decode = Some(DecodeFunc::LzwDecodeCompat);
            }
        }
        if let Some(sp) = lzw_decoder_state(tif) {
            sp.lzw_maxcode = maxcode(BITS_MIN) as u16;
        }
    } else if let Some(sp) = lzw_decoder_state(tif) {
        sp.lzw_maxcode = (maxcode(BITS_MIN) - 1) as u16;
        sp.dec_decode = Some(DecodeFunc::LzwDecode);
    }
    let Some(sp) = lzw_decoder_state(tif) else {
        return 0;
    };
    sp.lzw_nbits = BITS_MIN as u16;
    sp.lzw_nextbits = 0;
    sp.lzw_nextdata = 0;

    sp.dec_restart = 0;
    sp.dec_nbitsmask = maxcode(BITS_MIN);
    sp.dec_bitsleft = 0;
    sp.old_tif_rawcc = 0;
    sp.dec_free_entp = -1; // + CODE_FIRST;
                           /*
                            * Zero entries that are not yet filled in.  We do
                            * this to guard against bogus input data that causes
                            * us to index into undefined entries.  If you can
                            * come up with a way to safely bounds-check input codes
                            * while decoding then you can remove this operation.
                            */
    sp.dec_oldcodep = 0;
    sp.dec_maxcodep = sp.dec_nbitsmask - 1;
    sp.read_error = 0;
    1
}

/// Translation of `LZWDecode()`: Decode a "hunk of data".
fn lzw_decode(tif: &mut Tiff<'_>, op0: &mut [u8], occ0: TmSize, _s: u16) -> i32 {
    let mut data = std::mem::take(&mut tif.tif_data);
    let r = match &mut data {
        TifData::Lzw(sp) => match sp.dec_codetab.take() {
            Some(mut tab) => {
                let r = lzw_decode_inner(tif, sp, &mut tab, op0, occ0);
                sp.dec_codetab = Some(tab);
                r
            }
            None => 0,
        },
        _ => 0,
    };
    tif.tif_data = data;
    r
}

/// How the C's `LZWDecode()` leaves its code loop (its labels).
enum Exit {
    AfterLoop,
    TooShortBuffer,
    NoEoi,
    ErrorCode,
}

/// The body of `LZWDecode()`, over the state and code table taken out of
/// the handle.
fn lzw_decode_inner(
    tif: &mut Tiff<'_>,
    sp: &mut LZWCodecState,
    tab: &mut [CodeEnt],
    op0: &mut [u8],
    occ0: TmSize,
) -> i32 {
    const MODULE: &str = "LZWDecode";
    let olen = op0.len();
    let mut op: usize = 0;
    let mut occ: TmSize = occ0.min(olen as TmSize);

    let ent = |tab: &[CodeEnt], i: i64| -> CodeEnt {
        if i >= 0 {
            tab.get(i as usize).copied().unwrap_or_default()
        } else {
            CodeEnt::default()
        }
    };
    let next = |tab: &[CodeEnt], i: i64| -> i64 { ent(tab, i).next as i64 };

    if sp.read_error != 0 {
        op0[..occ as usize].fill(0);
        tiff_error_ext_r!(
            MODULE,
            "LZWDecode: Scanline {} cannot be read due to previous error",
            tif.tif_dir.td_row
        );
        return 0;
    }

    /*
     * Restart interrupted output operation.
     */
    if sp.dec_restart != 0 {
        let mut codep = sp.dec_codep;
        let mut residue = ent(tab, codep).length as TmSize - sp.dec_restart;
        if residue > occ {
            /*
             * Residue from previous decode is sufficient
             * to satisfy decode request.  Skip to the
             * start of the decoded string, place decoded
             * values in the output buffer, and return.
             */
            sp.dec_restart += occ;
            loop {
                codep = next(tab, codep);
                residue -= 1;
                if !(residue > occ && codep >= 0) {
                    break;
                }
            }
            if codep >= 0 {
                let mut tp = op + occ as usize;
                loop {
                    tp -= 1;
                    op0[tp] = ent(tab, codep).value;
                    codep = next(tab, codep);
                    occ -= 1;
                    if !(occ != 0 && codep >= 0) || tp == 0 {
                        break;
                    }
                }
            }
            return 1;
        }
        /*
         * Residue satisfies only part of the decode request.
         */
        if residue < 0 {
            residue = 0;
        }
        op += residue as usize;
        occ -= residue;
        let mut tp = op;
        while residue > 0 {
            tp -= 1;
            op0[tp] = ent(tab, codep).value;
            codep = next(tab, codep);
            residue -= 1;
            if codep < 0 || tp == 0 {
                break;
            }
        }
        sp.dec_restart = 0;
    }

    let mut bp = tif.tif_rawcp;
    sp.dec_bitsleft = sp
        .dec_bitsleft
        .wrapping_add(((tif.tif_rawcc as u64).wrapping_sub(sp.old_tif_rawcc as u64)) << 3);
    let mut dec_bitsleft = sp.dec_bitsleft;
    let mut nbits = sp.lzw_nbits as i64;
    let mut nextdata = sp.lzw_nextdata;
    let mut nextbits = sp.lzw_nextbits;
    let mut nbitsmask = sp.dec_nbitsmask;
    let mut oldcodep = sp.dec_oldcodep;
    let mut free_entp = sp.dec_free_entp;
    let mut maxcodep = sp.dec_maxcodep;
    let mut codep: i64 = 0;

    let raw = &tif.tif_rawdata;
    let byte = |i: usize| raw.get(i).copied().unwrap_or(0);

    /// Translation of `GetNextCodeLZW()`: `Err` is `goto no_eoi`.
    macro_rules! get_next_code_lzw {
        () => {{
            let code: WordType;
            nextbits -= nbits;
            if nextbits < 0 {
                if dec_bitsleft >= 8 * SIZEOF_WORDTYPE as u64 {
                    let codetmp = (nextdata << (-nextbits)) as u32;
                    let mut b = [0u8; 8];
                    for (k, x) in b.iter_mut().enumerate() {
                        *x = byte(bp + k);
                    }
                    nextdata = u64::from_be_bytes(b);
                    bp += SIZEOF_WORDTYPE as usize;
                    nextbits += 8 * SIZEOF_WORDTYPE;
                    dec_bitsleft -= 8 * SIZEOF_WORDTYPE as u64;
                    code =
                        ((codetmp as WordType) | (nextdata >> nextbits)) & (nbitsmask as WordType);
                    Ok(code)
                } else {
                    if dec_bitsleft < 8 {
                        Err(())
                    } else {
                        nextdata = (nextdata << 8) | byte(bp) as WordType;
                        bp += 1;
                        nextbits += 8;
                        dec_bitsleft -= 8;
                        if nextbits < 0 {
                            if dec_bitsleft < 8 {
                                Err(())
                            } else {
                                nextdata = (nextdata << 8) | byte(bp) as WordType;
                                bp += 1;
                                nextbits += 8;
                                dec_bitsleft -= 8;
                                Ok((nextdata >> nextbits) & (nbitsmask as WordType))
                            }
                        } else {
                            Ok((nextdata >> nextbits) & (nbitsmask as WordType))
                        }
                    }
                }
            } else {
                Ok((nextdata >> nextbits) & (nbitsmask as WordType))
            }
        }};
    }

    let exit = 'top: {
        if occ == 0 {
            break 'top Exit::AfterLoop;
        }
        'begin: loop {
            let Ok(code) = get_next_code_lzw!() else {
                break 'top Exit::NoEoi;
            };
            let code = code as i64;
            codep = code;
            if code >= CODE_FIRST {
                /* code_above_or_equal_to_258: */
                /*
                 * Add the new entry to the code table.
                 */
                let old = ent(tab, oldcodep);
                let fvalue = if codep >= free_entp {
                    if codep != free_entp {
                        break 'top Exit::ErrorCode;
                    }
                    old.firstchar
                } else {
                    ent(tab, codep).firstchar
                };
                let Some(fe) = usize::try_from(free_entp).ok().and_then(|i| tab.get_mut(i)) else {
                    break 'top Exit::ErrorCode;
                };
                fe.value = fvalue;
                fe.repeated = old.repeated & (old.value == fvalue);
                fe.next = oldcodep as i32;

                fe.firstchar = old.firstchar;
                fe.length = old.length.wrapping_add(1);
                free_entp += 1;
                if free_entp > maxcodep {
                    nbits += 1;
                    if nbits > BITS_MAX {
                        /* should not happen for a conformant encoder */
                        nbits = BITS_MAX;
                    }
                    nbitsmask = maxcode(nbits);
                    maxcodep = nbitsmask - 1;
                    if free_entp >= CSIZE {
                        /* At that point, the next valid states are either EOI or a */
                        /* CODE_CLEAR. If a regular code is read, at the next */
                        /* attempt at registering a new entry, we will error out */
                        /* due to setting free_entp before any valid code */
                        free_entp = -1;
                    }
                }
                oldcodep = codep;

                /*
                 * Code maps to a string, copy string
                 * value to output (written in reverse).
                 */
                /* tiny bit faster on x86_64 to store in unsigned short than int */
                let c = ent(tab, codep);
                let len = c.length;

                if len < 3 {
                    /* equivalent to len == 2 given all other conditions */
                    if occ <= 2 {
                        if occ == 2 {
                            op0[op] = c.firstchar;
                            op0[op + 1] = c.value;
                            op += 2;
                            occ -= 2;
                            break 'top Exit::AfterLoop;
                        }
                        break 'top Exit::TooShortBuffer;
                    }

                    op0[op] = c.firstchar;
                    op0[op + 1] = c.value;
                    op += 2;
                    occ -= 2;
                    continue 'begin; /* we can save the comparison occ > 0 */
                }

                if len == 3 {
                    if occ <= 3 {
                        if occ == 3 {
                            op0[op] = c.firstchar;
                            op0[op + 1] = ent(tab, c.next as i64).value;
                            op0[op + 2] = c.value;
                            op += 3;
                            occ -= 3;
                            break 'top Exit::AfterLoop;
                        }
                        break 'top Exit::TooShortBuffer;
                    }

                    op0[op] = c.firstchar;
                    op0[op + 1] = ent(tab, c.next as i64).value;
                    op0[op + 2] = c.value;
                    op += 3;
                    occ -= 3;
                    continue 'begin; /* we can save the comparison occ > 0 */
                }

                if len as TmSize > occ {
                    break 'top Exit::TooShortBuffer;
                }

                if c.repeated {
                    op0[op..op + len as usize].fill(c.value);
                    op += len as usize;
                    occ -= len as TmSize;
                    if occ == 0 {
                        break 'top Exit::AfterLoop;
                    }
                    continue 'begin;
                }

                let mut tp = op + len as usize;

                tp -= 1;
                op0[tp] = ent(tab, codep).value;
                codep = next(tab, codep);
                tp -= 1;
                op0[tp] = ent(tab, codep).value;
                codep = next(tab, codep);
                tp -= 1;
                op0[tp] = ent(tab, codep).value;
                codep = next(tab, codep);
                tp -= 1;
                op0[tp] = ent(tab, codep).value;
                while tp > op {
                    codep = next(tab, codep);
                    tp -= 1;
                    op0[tp] = ent(tab, codep).value;
                }

                op += len as usize;
                occ -= len as TmSize;
                if occ == 0 {
                    break 'top Exit::AfterLoop;
                }
                continue 'begin;
            }
            if code < 256 {
                /* code_below_256: */
                if codep > free_entp {
                    break 'top Exit::ErrorCode;
                }
                let old = ent(tab, oldcodep);
                let Some(fe) = usize::try_from(free_entp).ok().and_then(|i| tab.get_mut(i)) else {
                    break 'top Exit::ErrorCode;
                };
                fe.next = oldcodep as i32;
                fe.firstchar = old.firstchar;
                fe.length = old.length.wrapping_add(1);
                fe.value = code as u8;
                fe.repeated = old.repeated & (old.value as i64 == code);
                free_entp += 1;
                if free_entp > maxcodep {
                    nbits += 1;
                    if nbits > BITS_MAX {
                        /* should not happen for a conformant encoder */
                        nbits = BITS_MAX;
                    }
                    nbitsmask = maxcode(nbits);
                    maxcodep = nbitsmask - 1;
                    if free_entp >= CSIZE {
                        /* At that point, the next valid states are either EOI or a */
                        /* CODE_CLEAR. If a regular code is read, at the next */
                        /* attempt at registering a new entry, we will error out */
                        /* due to setting free_entp before any valid code */
                        free_entp = -1;
                    }
                }
                oldcodep = codep;
                op0[op] = code as u8;
                op += 1;
                occ -= 1;
                if occ == 0 {
                    break 'top Exit::AfterLoop;
                }
                continue 'begin;
            }
            if code == CODE_EOI {
                break 'top Exit::AfterLoop;
            }
            /* code_clear: */
            free_entp = CODE_FIRST;
            nbits = BITS_MIN;
            nbitsmask = maxcode(BITS_MIN);
            maxcodep = nbitsmask - 1;
            let code = loop {
                let Ok(code) = get_next_code_lzw!() else {
                    break 'top Exit::NoEoi;
                };
                if code as i64 != CODE_CLEAR {
                    break code as i64;
                }
            }; /* consecutive CODE_CLEAR codes */
            if code == CODE_EOI {
                break 'top Exit::AfterLoop;
            }
            if code > CODE_EOI {
                break 'top Exit::ErrorCode;
            }
            op0[op] = code as u8;
            op += 1;
            occ -= 1;
            oldcodep = code;
            if occ == 0 {
                break 'top Exit::AfterLoop;
            }
        }
    };

    match exit {
        Exit::TooShortBuffer => {
            /*
             * String is too long for decode buffer,
             * locate portion that will fit, copy to
             * the decode buffer, and setup restart
             * logic for the next decoding call.
             */
            sp.dec_codep = codep;
            loop {
                codep = next(tab, codep);
                if codep < 0 || !(ent(tab, codep).length as TmSize > occ) {
                    break;
                }
            }

            sp.dec_restart = occ;
            let mut tp = op + occ as usize;
            loop {
                tp -= 1;
                op0[tp] = ent(tab, codep).value;
                codep = next(tab, codep);
                occ -= 1;
                if occ == 0 || tp == 0 {
                    break;
                }
            }
        }
        Exit::AfterLoop => {}
        Exit::NoEoi => {
            op0[op..op + occ as usize].fill(0);
            sp.read_error = 1;
            tiff_error_ext_r!(
                MODULE,
                "LZWDecode: Strip {} not terminated with EOI code",
                tif.tif_dir.td_curstrip
            );
            return 0;
        }
        Exit::ErrorCode => {
            op0[op..op + occ as usize].fill(0);
            sp.read_error = 1;
            tiff_error_ext_r!(tif.tif_name, "Using code not yet in table");
            return 0;
        }
    }

    /* after_loop: */
    tif.tif_rawcc -= (bp - tif.tif_rawcp) as TmSize;
    tif.tif_rawcp = bp;
    sp.old_tif_rawcc = tif.tif_rawcc;
    sp.dec_bitsleft = dec_bitsleft;
    sp.lzw_nbits = nbits as u16;
    sp.lzw_nextdata = nextdata;
    sp.lzw_nextbits = nextbits;
    sp.dec_nbitsmask = nbitsmask;
    sp.dec_oldcodep = oldcodep;
    sp.dec_free_entp = free_entp;
    sp.dec_maxcodep = maxcodep;

    if occ > 0 {
        op0[op..op + occ as usize].fill(0);
        sp.read_error = 1;
        tiff_error_ext_r!(
            MODULE,
            "Not enough data at scanline {} (short {} bytes)",
            tif.tif_dir.td_row,
            occ as u64
        );
        return 0;
    }
    1
}

/// Translation of `LZWDecodeCompat()`: Decode a "hunk of data" for old
/// images.
fn lzw_decode_compat(tif: &mut Tiff<'_>, op0: &mut [u8], occ0: TmSize, _s: u16) -> i32 {
    let mut data = std::mem::take(&mut tif.tif_data);
    let r = match &mut data {
        TifData::Lzw(sp) => match sp.dec_codetab.take() {
            Some(mut tab) => {
                let r = lzw_decode_compat_inner(tif, sp, &mut tab, op0, occ0);
                sp.dec_codetab = Some(tab);
                r
            }
            None => 0,
        },
        _ => 0,
    };
    tif.tif_data = data;
    r
}

/// The body of `LZWDecodeCompat()`.
fn lzw_decode_compat_inner(
    tif: &mut Tiff<'_>,
    sp: &mut LZWCodecState,
    tab: &mut [CodeEnt],
    op0: &mut [u8],
    occ0: TmSize,
) -> i32 {
    const MODULE: &str = "LZWDecodeCompat";
    let mut op: usize = 0;
    let mut occ: TmSize = occ0.min(op0.len() as TmSize);

    let ent = |tab: &[CodeEnt], i: i64| -> CodeEnt {
        if i >= 0 {
            tab.get(i as usize).copied().unwrap_or_default()
        } else {
            CodeEnt::default()
        }
    };
    let next = |tab: &[CodeEnt], i: i64| -> i64 { ent(tab, i).next as i64 };

    /*
     * Restart interrupted output operation.
     */
    if sp.dec_restart != 0 {
        let mut codep = sp.dec_codep;
        let mut residue = ent(tab, codep).length as TmSize - sp.dec_restart;
        if residue > occ {
            /*
             * Residue from previous decode is sufficient
             * to satisfy decode request.  Skip to the
             * start of the decoded string, place decoded
             * values in the output buffer, and return.
             */
            sp.dec_restart += occ;
            loop {
                codep = next(tab, codep);
                residue -= 1;
                if !(residue > occ) || codep < 0 {
                    break;
                }
            }
            let mut tp = op + occ as usize;
            while tp > 0 {
                tp -= 1;
                op0[tp] = ent(tab, codep).value;
                codep = next(tab, codep);
                occ -= 1;
                if occ == 0 {
                    break;
                }
            }
            return 1;
        }
        /*
         * Residue satisfies only part of the decode request.
         */
        if residue < 0 {
            residue = 0;
        }
        op += residue as usize;
        occ -= residue;
        let mut tp = op;
        while residue > 0 && tp > 0 {
            tp -= 1;
            op0[tp] = ent(tab, codep).value;
            codep = next(tab, codep);
            residue -= 1;
        }
        sp.dec_restart = 0;
    }

    let mut bp = tif.tif_rawcp;

    sp.dec_bitsleft = sp
        .dec_bitsleft
        .wrapping_add(((tif.tif_rawcc as u64).wrapping_sub(sp.old_tif_rawcc as u64)) << 3);
    let mut dec_bitsleft = sp.dec_bitsleft;

    let mut nbits = sp.lzw_nbits as i64;
    let mut nextdata = sp.lzw_nextdata;
    let mut nextbits = sp.lzw_nextbits;
    let mut nbitsmask = sp.dec_nbitsmask;
    let mut oldcodep = sp.dec_oldcodep;
    let mut free_entp = sp.dec_free_entp;
    let mut maxcodep = sp.dec_maxcodep;
    let curstrip = tif.tif_dir.td_curstrip;
    let row = tif.tif_dir.td_row;

    let raw = &tif.tif_rawdata;
    let byte = |i: usize| raw.get(i).copied().unwrap_or(0);

    /// Translation of `NextCode()` with `GetNextCodeCompat()`: This check
    /// shouldn't be necessary because each strip is suppose to be
    /// terminated with CODE_EOI.
    macro_rules! next_code {
        () => {{
            if dec_bitsleft < nbits as u64 {
                tiff_warning_ext_r!(
                    MODULE,
                    "LZWDecode: Strip {} not terminated with EOI code",
                    curstrip
                );
                CODE_EOI
            } else {
                nextdata |= (byte(bp) as WordType) << nextbits;
                bp += 1;
                nextbits += 8;
                if nextbits < nbits {
                    nextdata |= (byte(bp) as WordType) << nextbits;
                    bp += 1;
                    nextbits += 8;
                }
                let code = (nextdata & nbitsmask as WordType) as u16 as i64;
                nextdata >>= nbits;
                nextbits -= nbits;
                dec_bitsleft -= nbits as u64;
                code
            }
        }};
    }

    while occ > 0 {
        let mut code = next_code!();
        if code == CODE_EOI {
            break;
        }
        if code == CODE_CLEAR {
            loop {
                free_entp = CODE_FIRST;
                for e in &mut tab[CODE_FIRST as usize..CSIZE as usize] {
                    *e = CodeEnt::default();
                }
                nbits = BITS_MIN;
                nbitsmask = maxcode(BITS_MIN);
                maxcodep = nbitsmask;
                code = next_code!();
                if code != CODE_CLEAR {
                    break;
                }
            } /* consecutive CODE_CLEAR codes */
            if code == CODE_EOI {
                break;
            }
            if code > CODE_CLEAR {
                tiff_error_ext_r!(
                    tif.tif_name,
                    "LZWDecode: Corrupted LZW table at scanline {}",
                    row
                );
                return 0;
            }
            op0[op] = code as u8;
            op += 1;
            occ -= 1;
            oldcodep = code;
            continue;
        }
        let mut codep = code;

        /*
         * Add the new entry to the code table.
         */
        if free_entp < 0 || free_entp >= CSIZE {
            tiff_error_ext_r!(MODULE, "Corrupted LZW table at scanline {}", row);
            return 0;
        }

        tab[free_entp as usize].next = oldcodep as i32;
        if oldcodep < 0 || oldcodep >= CSIZE {
            tiff_error_ext_r!(MODULE, "Corrupted LZW table at scanline {}", row);
            return 0;
        }
        let old = ent(tab, oldcodep);
        let fe = &mut tab[free_entp as usize];
        fe.firstchar = old.firstchar;
        fe.length = old.length.wrapping_add(1);
        let firstchar = fe.firstchar;
        tab[free_entp as usize].value = if codep < free_entp {
            ent(tab, codep).firstchar
        } else {
            firstchar
        };
        free_entp += 1;
        if free_entp > maxcodep {
            nbits += 1;
            if nbits > BITS_MAX {
                /* should not happen */
                nbits = BITS_MAX;
            }
            nbitsmask = maxcode(nbits);
            maxcodep = nbitsmask;
        }
        oldcodep = codep;
        if code >= 256 {
            /*
             * Code maps to a string, copy string
             * value to output (written in reverse).
             */
            let length = ent(tab, codep).length as TmSize;
            if length == 0 {
                tiff_error_ext_r!(
                    MODULE,
                    "Wrong length of decoded string: data probably corrupted at scanline {}",
                    row
                );
                return 0;
            }
            if length > occ {
                /*
                 * String is too long for decode buffer,
                 * locate portion that will fit, copy to
                 * the decode buffer, and setup restart
                 * logic for the next decoding call.
                 */
                sp.dec_codep = codep;
                loop {
                    codep = next(tab, codep);
                    if codep < 0 || !(ent(tab, codep).length as TmSize > occ) {
                        break;
                    }
                }
                sp.dec_restart = occ;
                let mut tp = op + occ as usize;
                loop {
                    tp -= 1;
                    op0[tp] = ent(tab, codep).value;
                    codep = next(tab, codep);
                    occ -= 1;
                    if occ == 0 || tp == 0 {
                        break;
                    }
                }
                break;
            }
            let len = length as usize;
            let mut tp = op + len;
            loop {
                tp -= 1;
                op0[tp] = ent(tab, codep).value;
                codep = next(tab, codep);
                if !(codep >= 0 && tp > op) {
                    break;
                }
            }
            op += len;
            occ -= len as TmSize;
        } else {
            op0[op] = code as u8;
            op += 1;
            occ -= 1;
        }
    }

    tif.tif_rawcc -= (bp - tif.tif_rawcp) as TmSize;
    tif.tif_rawcp = bp;

    sp.old_tif_rawcc = tif.tif_rawcc;
    sp.dec_bitsleft = dec_bitsleft;

    sp.lzw_nbits = nbits as u16;
    sp.lzw_nextdata = nextdata;
    sp.lzw_nextbits = nextbits;
    sp.dec_nbitsmask = nbitsmask;
    sp.dec_oldcodep = oldcodep;
    sp.dec_free_entp = free_entp;
    sp.dec_maxcodep = maxcodep;

    if occ > 0 {
        tiff_error_ext_r!(
            MODULE,
            "Not enough data at scanline {} (short {} bytes)",
            tif.tif_dir.td_row,
            occ as u64
        );
        return 0;
    }
    1
}

/// Translation of `LZWCleanup()`.
fn lzw_cleanup(tif: &mut Tiff<'_>) {
    let _ = tiff_predictor_cleanup(tif);

    tif.tif_data = TifData::None;

    _tiff_set_default_compression_state(tif);
}

/// Translation of `LZWGetMaxCompressionRatio()`.
fn lzw_get_max_compression_ratio(_tif: &Tiff<'_>) -> u64 {
    /* See README_for_libtiff_developpers.md for raw data used to estimate
     * the maximum compression rate. */

    /* 1024x1024: 562 */
    /* 4096x4096: 1243 */
    /* 16383x16383: 1353 */
    /* 65536x65536: 1362 */

    1400
}

/// Translation of `TIFFInitLZW()`.
pub(crate) fn tiff_init_lzw(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    /*
     * Allocate state block so tag methods have storage to record values.
     */
    let sp = LZWCodecState {
        predict: TIFFPredictorState::new(tif),
        lzw_nbits: 0,
        lzw_maxcode: 0,
        lzw_nextdata: 0,
        lzw_nextbits: 0,
        rw_mode: tif.tif_mode,
        dec_nbitsmask: 0,
        dec_restart: 0,
        dec_bitsleft: 0,
        old_tif_rawcc: 0,
        dec_decode: None,
        dec_codep: 0,
        dec_oldcodep: 0,
        dec_free_entp: 0,
        dec_maxcodep: 0,
        dec_codetab: None,
        read_error: 0,
    };
    let _ = sp.rw_mode;
    let _ = sp.lzw_maxcode;
    tif.tif_data = TifData::Lzw(Box::new(sp));

    /*
     * Install codec methods.
     */
    tif.tif_fixuptags = lzw_fixup_tags;
    tif.tif_setupdecode = lzw_setup_decode;
    tif.tif_predecode = lzw_pre_decode;
    tif.tif_decoderow = lzw_decode;
    tif.tif_decodestrip = lzw_decode;
    tif.tif_decodetile = lzw_decode;
    tif.tif_getmaxcompressionratio = lzw_get_max_compression_ratio;
    tif.tif_cleanup = lzw_cleanup;
    /*
     * Setup predictor setup.
     */
    let _ = tiff_predictor_init(tif);
    1
}
