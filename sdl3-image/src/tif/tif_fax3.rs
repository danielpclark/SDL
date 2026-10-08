// Rust translation of libtiff/tif_fax3.c and libtiff/tif_fax3.h from libtiff
// (https://gitlab.com/libtiff/libtiff, 4.7.2 as SDL_image's external/libtiff
// pins it).
// Copyright (c) 1990-1997 Sam Leffler
// Copyright (c) 1991-1997 Silicon Graphics, Inc.
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CCITT Group 3 (T.4) and Group 4 (T.6) Compression Support.
//!
//! This file contains support for decoding and encoding TIFF
//! compression algorithms 2, 3, 4, and 32771.
//!
//! Decoder support is derived, with permission, from the code
//! in Frank Cringle's viewfax program;
//!      Copyright (C) 1990, 1995  Frank D. Cringle.
//!
//! The decoders (the encoders are left out). The C's decoder macros
//! (`NeedBits8()`, `LOOKUP16()`, `SETVALUE()`, `SYNC_EOL()`,
//! `EXPAND1D()`, `EXPAND2D()`, ...) work on the decoding function's
//! local variables; here those are a [`Dec`] and the macros its methods,
//! the C's `goto`s their results. The run arrays are one vector with the
//! current and reference rows at offsets into it (`runs`, `curruns` and
//! `refruns` share one allocation in the C too). The fill function
//! pseudo-tag (`TIFFTAG_FAXFILLFUNC`) can't be set: a file's tags can't
//! name it, and SDL_image doesn't.

use super::tif_compress::_tiff_set_default_compression_state;
use super::tif_dir::{
    tiff_set_field, tiff_set_field_bit, Gv, TIFFVGetMethod, TIFFVSetMethod, Va, VaList,
    FIELD_CODEC, FIELD_PSEUDO, TIFF_SETGET_INT, TIFF_SETGET_OTHER, TIFF_SETGET_UINT16,
    TIFF_SETGET_UINT32,
};
use super::tif_dirinfo::{_tiff_merge_fields, tiff_field_with_tag};
use super::tif_error::{tiff_error_ext_r, tiff_warning_ext_r};
use super::tif_fax3sm::{TIFF_FAX_BLACK_TABLE, TIFF_FAX_MAIN_TABLE, TIFF_FAX_WHITE_TABLE};
use super::tif_strip::tiff_scanline_size;
use super::tif_swab::tiff_get_bit_rev_table;
use super::tiff::*;
use super::tiffio::{field, TIFFField, TIFF_ANY};
use super::tiffiop::{
    tiff_roundup_32, try_vec, TifData, Tiff, TmSize, O_RDONLY, TIFF_DIRTYDIRECT, TIFF_NOBITREV,
};

/*
 * To override the default routine used to image decoded
 * spans one can use the pseudo tag TIFFTAG_FAXFILLFUNC.
 * The routine must have the type signature given below;
 * for example:
 *
 * fillruns(unsigned char* buf, uint32_t* runs, uint32_t* erun, uint32_t lastx)
 *
 * where buf is place to set the bits, runs is the array of b&w run
 * lengths (white then black), erun is the last run in the array, and
 * lastx is the width of the row in pixels.  Fill routines can assume
 * the run array has room for at least lastx runs and can overwrite
 * data in the run array as needed (e.g. to append zero runs to bring
 * the count up to a nice multiple).
 */
/// `TIFFFaxFillFunc`: the runs are `runs[start..erun]` of the array.
pub(crate) type TIFFFaxFillFunc = fn(&mut [u8], &mut [u32], usize, usize, u32);

/* finite state machine codes */
const S_NULL: u8 = 0;
const S_PASS: u8 = 1;
const S_HORIZ: u8 = 2;
const S_V0: u8 = 3;
const S_VR: u8 = 4;
const S_VL: u8 = 5;
const S_EXT: u8 = 6;
const S_TERM_W: u8 = 7;
const S_TERM_B: u8 = 8;
const S_MAKE_UP_W: u8 = 9;
const S_MAKE_UP_B: u8 = 10;
const S_MAKE_UP: u8 = 11;
const S_EOL: u8 = 12;

/* WARNING: do not change the layout of this structure as the HylaFAX software
 */
/* really depends on it. See http://bugzilla.maptools.org/show_bug.cgi?id=2636
 */
/// Translation of `TIFFFaxTabEnt`: state table entry
#[allow(non_snake_case)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct TIFFFaxTabEnt {
    pub(crate) State: u8, /* see above */
    pub(crate) Width: u8, /* width of code in bits */
    pub(crate) Param: u32, /* unsigned 32-bit run length in bits (holds on 16 bit
                          actually, but cannot be changed. See above warning) */
}

/* Arbitrary threshold to avoid corrupted single-strip files with extremely
 * large imageheight to cause apparently endless looping, such as in
 * https://gitlab.com/libtiff/libtiff/-/issues/583
 */
const EOF_REACHED_COUNT_THRESHOLD: i32 = 8192;

/// Translation of `Fax3CodecState` (with its `Fax3BaseState`, the
/// ``base state'' compression+decompression state blocks are derived
/// from; the encoder's state left out, but for the reference line the
/// setup allocates).
#[allow(non_snake_case)]
pub(crate) struct Fax3CodecState {
    /* Fax3BaseState b */
    rw_mode: i32,               /* O_RDONLY for decode, else encode */
    mode: i32,                  /* operating mode */
    rowbytes: TmSize,           /* bytes in a decoded scanline */
    rowpixels: u32,             /* pixels in a scanline */
    cleanfaxdata: u16,          /* CleanFaxData tag */
    badfaxrun: u32,             /* BadFaxRun tag */
    badfaxlines: u32,           /* BadFaxLines tag */
    groupoptions: u32,          /* Group 3/4 options tag */
    vgetparent: TIFFVGetMethod, /* super-class method */
    vsetparent: TIFFVSetMethod, /* super-class method */

    /* Decoder state info */
    bitmap: &'static [u8; 256], /* bit reversal table */
    data: u32,                  /* current i/o byte/word */
    bit: i32,                   /* current i/o bit in byte */
    EOLcnt: i32,                /* count of EOL codes recognized */
    eofReachedCount: i32,       /* number of times decode has been called with
                                EOF already reached */
    eolReachedCount: i32, /* number of times decode has been called with
                          EOL already reached */
    unexpectedReachedCount: i32, /* number of times decode has been called with
                                 "unexpedted" already reached */
    fill: TIFFFaxFillFunc,  /* fill routine */
    runs: Option<Vec<u32>>, /* b&w runs for current/previous row */
    nruns: u32,             /* size of the refruns / curruns arrays */
    refruns: Option<usize>, /* runs for reference line */
    curruns: usize,         /* runs for current line */

    /* Encoder state info */
    refline: Option<Vec<u8>>, /* reference line for 2d decoding */

    line: i32,
}

/// Translation of `is2DEncoding()`.
fn is_2d_encoding(sp: &Fax3CodecState) -> bool {
    (sp.groupoptions & GROUP3OPT_2DENCODING) != 0
}

/// Translation of `DecoderState()`.
fn decoder_state<'s>(tif: &'s mut Tiff<'_>) -> Option<&'s mut Fax3CodecState> {
    match &mut tif.tif_data {
        TifData::Fax3(sp) => Some(sp),
        _ => None,
    }
}

/*
 * Group 3 and Group 4 Decoding.
 */

/// Translation of `Fax3PreDecode()`: Setup state for decoding a strip.
fn fax3_pre_decode(tif: &mut Tiff<'_>, _s: u16) -> i32 {
    let fillorder = tif.tif_dir.td_fillorder;
    let Some(sp) = decoder_state(tif) else {
        return 0;
    };
    sp.bit = 0; /* force initial read */
    sp.data = 0;
    sp.EOLcnt = 0; /* force initial scan for EOL */
    sp.eofReachedCount = 0;
    sp.eolReachedCount = 0;
    sp.unexpectedReachedCount = 0;
    /*
     * Decoder assumes lsb-to-msb bit order.  Note that we select
     * this here rather than in Fax3SetupState so that viewers can
     * hold the image open, fiddle with the FillOrder tag value,
     * and then re-decode the image.  Otherwise they'd need to close
     * and open the image to get the state reset.
     */
    sp.bitmap = tiff_get_bit_rev_table(fillorder != FILLORDER_LSB2MSB);
    sp.curruns = 0;
    if sp.refruns.is_some() {
        /* init reference line to white */
        let r = sp.nruns as usize;
        sp.refruns = Some(r);
        let rowpixels = sp.rowpixels;
        if let Some(runs) = sp.runs.as_mut() {
            if r + 1 < runs.len() {
                runs[r] = rowpixels;
                runs[r + 1] = 0;
            }
        }
    }
    sp.line = 0;
    1
}

/// Where the messages say the line is: "tile"/"strip" and its number.
#[derive(Clone, Copy)]
struct Ctx {
    module: &'static str,
    tiled: bool,
    strile: u32,
}

impl Ctx {
    fn new(tif: &Tiff<'_>, module: &'static str) -> Ctx {
        Ctx {
            module,
            tiled: tif.is_tiled(),
            strile: if tif.is_tiled() {
                tif.tif_dir.td_curtile
            } else {
                tif.tif_dir.td_curstrip
            },
        }
    }

    fn what(&self) -> &'static str {
        if self.tiled {
            "tile"
        } else {
            "strip"
        }
    }
}

/*
 * Routine for handling various errors/conditions.
 * Note how they are "glued into the decoder" by
 * overriding the definitions used by the decoder.
 */

/// Translation of `Fax3Unexpected()`.
fn fax3_unexpected(ctx: Ctx, line: u32, a0: u32) {
    tiff_error_ext_r!(
        ctx.module,
        "Bad code word at line {} of {} {} (x {})",
        line,
        ctx.what(),
        ctx.strile,
        a0
    );
}

/// Translation of `Fax3Extension()`.
fn fax3_extension(ctx: Ctx, line: u32, a0: u32) {
    tiff_error_ext_r!(
        ctx.module,
        "Uncompressed data (not supported) at line {} of {} {} (x {})",
        line,
        ctx.what(),
        ctx.strile,
        a0
    );
}

/// Translation of `Fax3BadLength()`.
fn fax3_bad_length(ctx: Ctx, line: u32, a0: u32, lastx: u32) {
    tiff_warning_ext_r!(
        ctx.module,
        "{} at line {} of {} {} (got {}, expected {})",
        if a0 < lastx {
            "Premature EOL"
        } else {
            "Line length mismatch"
        },
        line,
        ctx.what(),
        ctx.strile,
        a0,
        lastx
    );
}

/// Translation of `Fax3PrematureEOF()`.
fn fax3_premature_eof(ctx: Ctx, line: u32, a0: u32) {
    tiff_warning_ext_r!(
        ctx.module,
        "Premature EOF at line {} of {} {} (x {})",
        line,
        ctx.what(),
        ctx.strile,
        a0
    );
}

/// Translation of `Fax3TryG3WithoutEOL()`.
fn fax3_try_g3_without_eol(ctx: Ctx, line: u32, a0: u32) {
    tiff_warning_ext_r!(
        ctx.module,
        "Try to decode (read) fax Group 3 data without EOL at line {} of {} {} (x {}). Please check result",
        line,
        ctx.what(),
        ctx.strile,
        a0
    );
}

/// Translation of `CheckReachedCounters()`.
fn check_reached_counters(ctx: Ctx, sp: &Fax3CodecState) -> i32 {
    if sp.eofReachedCount >= EOF_REACHED_COUNT_THRESHOLD {
        tiff_error_ext_r!(
            ctx.module,
            "End of file (EOF) has already been reached {} times within that {}.",
            sp.eofReachedCount,
            ctx.what()
        );
        return -1;
    }
    if sp.eolReachedCount >= EOF_REACHED_COUNT_THRESHOLD {
        tiff_error_ext_r!(
            ctx.module,
            "Bad line length (EOL) has already been reached {} times within that {}",
            sp.eolReachedCount,
            ctx.what()
        );
        return -1;
    }
    if sp.unexpectedReachedCount >= EOF_REACHED_COUNT_THRESHOLD {
        tiff_error_ext_r!(
            ctx.module,
            "Bad code word (unexpected) has already been reached {} times within that {}",
            sp.unexpectedReachedCount,
            ctx.what()
        );
        return -1;
    }
    0
}

/// The decoder's `return (-1)` (from `SETVALUE()` and the overflow
/// checks).
struct Ret;

/// The local state of a decoding function (`DECLARE_STATE()` and
/// `DECLARE_STATE_2D()`), its decoding macros as methods.
#[allow(non_snake_case)]
struct Dec<'r> {
    ctx: Ctx,
    a0: i32,                    /* reference element */
    lastx: i32,                 /* last element in row */
    BitAcc: u32,                /* bit accumulator */
    BitsAvail: i32,             /* # valid bits in BitAcc */
    RunLength: i32,             /* length of current run */
    cp: usize,                  /* next byte of input data */
    ep: usize,                  /* end of input data */
    raw: &'r [u8],              /* (the raw data cp and ep index) */
    pa: usize,                  /* place to stuff next run */
    thisrun: usize,             /* current row's run array */
    EOLcnt: i32,                /* # EOL codes recognized */
    bitmap: &'static [u8; 256], /* input data bit reverser */
    b1: i32,                    /* next change on prev line */
    pb: isize,                  /* next run in reference line */
}

impl Dec<'_> {
    /// `CACHE_STATE()`: Load any state that may be changed during
    /// decoding.
    fn cache_state(&mut self, tif_rawcp: usize, tif_rawcc: TmSize, sp: &Fax3CodecState) {
        self.BitAcc = sp.data;
        self.BitsAvail = sp.bit;
        self.EOLcnt = sp.EOLcnt;
        self.cp = tif_rawcp;
        self.ep = self.cp + tif_rawcc.max(0) as usize;
    }

    /// `EndOfData()`
    fn end_of_data(&self) -> bool {
        self.cp >= self.ep
    }

    /// The next input byte, bit reversed as needed (`bitmap[*cp++]`).
    fn next_byte(&mut self) -> u32 {
        let b = self.raw.get(self.cp).copied().unwrap_or(0);
        self.cp += 1;
        self.bitmap[b as usize] as u32
    }

    /// `NeedBits8(n, eoflab)`: `false` is `goto eoflab`.
    fn need_bits8(&mut self, n: i32) -> bool {
        if self.BitsAvail < n {
            if self.end_of_data() {
                if self.BitsAvail == 0 {
                    /* no valid bits */
                    return false;
                }
                self.BitsAvail = n; /* pad with zeros */
            } else {
                self.BitAcc |= self.next_byte() << self.BitsAvail;
                self.BitsAvail += 8;
            }
        }
        true
    }

    /// `NeedBits16(n, eoflab)`: `false` is `goto eoflab`.
    fn need_bits16(&mut self, n: i32) -> bool {
        if self.BitsAvail < n {
            if self.end_of_data() {
                if self.BitsAvail == 0 {
                    /* no valid bits */
                    return false;
                }
                self.BitsAvail = n; /* pad with zeros */
            } else {
                self.BitAcc |= self.next_byte() << self.BitsAvail;
                self.BitsAvail += 8;
                if self.BitsAvail < n {
                    if self.end_of_data() {
                        /* NB: we know BitsAvail is non-zero here */
                        self.BitsAvail = n; /* pad with zeros */
                    } else {
                        self.BitAcc |= self.next_byte() << self.BitsAvail;
                        self.BitsAvail += 8;
                    }
                }
            }
        }
        true
    }

    /// `GetBits(n)`
    fn get_bits(&self, n: i32) -> u32 {
        self.BitAcc & ((1u32 << n) - 1)
    }

    /// `ClrBits(n)`
    fn clr_bits(&mut self, n: i32) {
        self.BitsAvail -= n;
        self.BitAcc = self.BitAcc.checked_shr(n as u32).unwrap_or(0);
    }

    /// `LOOKUP8(wid, tab, eoflab)`: `None` is `goto eoflab`.
    fn lookup8(&mut self, wid: i32, tab: &[TIFFFaxTabEnt]) -> Option<TIFFFaxTabEnt> {
        if !self.need_bits8(wid) {
            return None;
        }
        let te = tab[self.get_bits(wid) as usize];
        self.clr_bits(te.Width as i32);
        Some(te)
    }

    /// `LOOKUP16(wid, tab, eoflab)`: `None` is `goto eoflab`.
    fn lookup16(&mut self, wid: i32, tab: &[TIFFFaxTabEnt]) -> Option<TIFFFaxTabEnt> {
        if !self.need_bits16(wid) {
            return None;
        }
        let te = tab[self.get_bits(wid) as usize];
        self.clr_bits(te.Width as i32);
        Some(te)
    }

    /// The overflow error of `SETVALUE()` and its siblings.
    fn overflow(&self, sp: &Fax3CodecState) -> Ret {
        tiff_error_ext_r!(
            self.ctx.module,
            "Buffer overflow at line {} of {} {}",
            sp.line,
            self.ctx.what(),
            self.ctx.strile
        );
        Ret
    }

    /// `SETVALUE(x)`: Append a run to the run length array for the
    /// current row and reset decoding state.
    fn setvalue(&mut self, sp: &mut Fax3CodecState, x: u32) -> Result<(), Ret> {
        if self.pa >= self.thisrun + sp.nruns as usize {
            return Err(self.overflow(sp));
        }
        let v = (self.RunLength as u32).wrapping_add(x);
        if let Some(r) = sp.runs.as_mut().and_then(|r| r.get_mut(self.pa)) {
            *r = v;
        }
        self.pa += 1;
        self.a0 = self.a0.wrapping_add(x as i32);
        self.RunLength = 0;
        Ok(())
    }

    /// `runs[i]` (0 outside the array, which the C reads out of bounds).
    fn run_at(sp: &Fax3CodecState, i: isize) -> u32 {
        if i < 0 {
            return 0;
        }
        sp.runs
            .as_ref()
            .and_then(|r| r.get(i as usize).copied())
            .unwrap_or(0)
    }

    /// `unexpected(table, a0)`
    fn unexpected(&self, sp: &mut Fax3CodecState) {
        fax3_unexpected(self.ctx, sp.line as u32, self.a0 as u32);
        sp.unexpectedReachedCount += 1;
    }

    /// `badlength(a0, lastx)`
    fn badlength(&self, sp: &mut Fax3CodecState) {
        fax3_bad_length(self.ctx, sp.line as u32, self.a0 as u32, self.lastx as u32);
        sp.eolReachedCount += 1;
    }

    /// `prematureEOF(a0)`
    fn premature_eof(&self, sp: &mut Fax3CodecState) {
        fax3_premature_eof(self.ctx, sp.line as u32, self.a0 as u32);
        sp.eofReachedCount += 1;
    }

    /// `SYNC_EOL(eoflab, retrywithouteol)`: Synchronize input decoding at
    /// the start of each row by scanning for an EOL (if appropriate) and
    /// skipping any trash data that might be present after a decoding
    /// error.  Note that the decoding done elsewhere that recognizes an
    /// EOL only consumes 11 consecutive zero bits.  This means that if
    /// EOLcnt is non-zero then we still need to scan for the final flag
    /// bit that is part of the EOL code.
    fn sync_eol(&mut self, sp: &mut Fax3CodecState) -> Sync {
        if (sp.mode & FAXMODE_NOEOL) == 0 {
            /* skip EOL, if not present */
            if self.EOLcnt == 0 {
                loop {
                    if !self.need_bits16(11) {
                        return Sync::Eof;
                    }
                    if self.get_bits(11) == 0 {
                        break; /* EOL found */
                    }
                    self.clr_bits(1);
                }
            }
            /* Now move after EOL or detect missing EOL. */
            loop {
                if !self.need_bits8(8) {
                    /* noEOLFound: */
                    sp.mode |= FAXMODE_NOEOL;
                    fax3_try_g3_without_eol(self.ctx, sp.line as u32, self.a0 as u32);
                    return Sync::Retry;
                }
                if self.get_bits(8) != 0 {
                    break;
                }
                self.clr_bits(8);
            }
            while self.get_bits(1) == 0 {
                self.clr_bits(1);
            }
            self.clr_bits(1); /* EOL bit */
            self.EOLcnt = 0; /* reset EOL counter/flag */
        }
        Sync::Ok
    }

    /// `CLEANUP_RUNS()`: Cleanup the array of runs after decoding a row.
    /// We adjust final runs to insure the user buffer is not overwritten
    /// and/or undecoded area is white filled.
    fn cleanup_runs(&mut self, sp: &mut Fax3CodecState) -> Result<(), Ret> {
        if self.RunLength != 0 {
            self.setvalue(sp, 0)?;
        }
        if self.a0 != self.lastx {
            self.badlength(sp);
            while self.a0 > self.lastx && self.pa > self.thisrun {
                self.pa -= 1;
                self.a0 = self
                    .a0
                    .wrapping_sub(Self::run_at(sp, self.pa as isize) as i32);
            }
            if self.a0 < self.lastx {
                if self.a0 < 0 {
                    self.a0 = 0;
                }
                if ((self.pa - self.thisrun) & 1) != 0 {
                    self.setvalue(sp, 0)?;
                }
                self.setvalue(sp, (self.lastx - self.a0) as u32)?;
            } else if self.a0 > self.lastx {
                self.setvalue(sp, self.lastx as u32)?;
                self.setvalue(sp, 0)?;
            }
        }
        Ok(())
    }

    /// `EXPAND1D(eoflab)`: Decode a line of 1D-encoded data; `Ok(true)` is
    /// `goto eoflab`.
    ///
    /// Note that unlike the original version we have to explicitly test for
    /// a0 >= lastx after each black/white run is decoded.  This is because
    /// the original code depended on the input data being zero-padded to
    /// insure the decoder recognized an EOL before running out of data.
    fn expand1d(&mut self, sp: &mut Fax3CodecState) -> Result<bool, Ret> {
        let eof = 'body: {
            loop {
                loop {
                    let Some(te) = self.lookup16(12, &TIFF_FAX_WHITE_TABLE) else {
                        break 'body true;
                    };
                    match te.State {
                        S_EOL => {
                            self.EOLcnt = 1;
                            break 'body false;
                        }
                        S_TERM_W => {
                            self.setvalue(sp, te.Param)?;
                            break; /* doneWhite1d */
                        }
                        S_MAKE_UP_W | S_MAKE_UP => {
                            self.a0 = (self.a0 as u32).wrapping_add(te.Param) as i32;
                            self.RunLength = (self.RunLength as u32).wrapping_add(te.Param) as i32;
                        }
                        _ => {
                            self.unexpected(sp);
                            break 'body false;
                        }
                    }
                }
                /* doneWhite1d: */
                if self.a0 >= self.lastx {
                    break 'body false;
                }
                loop {
                    let Some(te) = self.lookup16(13, &TIFF_FAX_BLACK_TABLE) else {
                        break 'body true;
                    };
                    match te.State {
                        S_EOL => {
                            self.EOLcnt = 1;
                            break 'body false;
                        }
                        S_TERM_B => {
                            self.setvalue(sp, te.Param)?;
                            break; /* doneBlack1d */
                        }
                        S_MAKE_UP_B | S_MAKE_UP => {
                            self.a0 = self.a0.wrapping_add(te.Param as i32);
                            self.RunLength = self.RunLength.wrapping_add(te.Param as i32);
                        }
                        _ => {
                            self.unexpected(sp);
                            break 'body false;
                        }
                    }
                }
                /* doneBlack1d: */
                if self.a0 >= self.lastx {
                    break 'body false;
                }
                if Self::run_at(sp, self.pa as isize - 1) == 0
                    && Self::run_at(sp, self.pa as isize - 2) == 0
                {
                    self.pa -= 2;
                }
            }
        };
        if eof {
            /* eof1d: */
            self.premature_eof(sp);
            self.cleanup_runs(sp)?;
            return Ok(true);
        }
        /* done1d: */
        self.cleanup_runs(sp)?;
        Ok(false)
    }

    /// `CHECK_b1`: Update the value of b1 using the array of runs for the
    /// reference line.
    fn check_b1(&mut self, sp: &Fax3CodecState) -> Result<(), Ret> {
        if self.pa != self.thisrun {
            let refruns = sp.refruns.unwrap_or(0) as isize;
            while self.b1 <= self.a0 && self.b1 < self.lastx {
                if self.pb + 1 >= refruns + sp.nruns as isize {
                    return Err(self.overflow(sp));
                }
                self.b1 = self.b1.wrapping_add(
                    (Self::run_at(sp, self.pb).wrapping_add(Self::run_at(sp, self.pb + 1))) as i32,
                );
                self.pb += 2;
            }
        }
        Ok(())
    }

    /// One color's codes in the horizontal mode of `EXPAND2D()`:
    /// `Ok(None)` is the terminating code, `Ok(Some(bad))` a bad code
    /// (`goto bad*2d`) or the end of data (`goto eof2d`).
    fn horiz_run(&mut self, sp: &mut Fax3CodecState, black: bool) -> Result<Option<Exit2D>, Ret> {
        loop {
            let te = if black {
                self.lookup16(13, &TIFF_FAX_BLACK_TABLE)
            } else {
                self.lookup16(12, &TIFF_FAX_WHITE_TABLE)
            };
            let Some(te) = te else {
                return Ok(Some(Exit2D::Eof));
            };
            match te.State {
                S_TERM_B if black => {
                    self.setvalue(sp, te.Param)?;
                    return Ok(None);
                }
                S_TERM_W if !black => {
                    self.setvalue(sp, te.Param)?;
                    return Ok(None);
                }
                S_MAKE_UP_B if black => {
                    self.a0 = (self.a0 as u32).wrapping_add(te.Param) as i32;
                    self.RunLength = (self.RunLength as u32).wrapping_add(te.Param) as i32;
                }
                S_MAKE_UP_W if !black => {
                    self.a0 = (self.a0 as u32).wrapping_add(te.Param) as i32;
                    self.RunLength = (self.RunLength as u32).wrapping_add(te.Param) as i32;
                }
                S_MAKE_UP => {
                    self.a0 = (self.a0 as u32).wrapping_add(te.Param) as i32;
                    self.RunLength = (self.RunLength as u32).wrapping_add(te.Param) as i32;
                }
                _ => {
                    return Ok(Some(if black {
                        Exit2D::BadBlack
                    } else {
                        Exit2D::BadWhite
                    }));
                }
            }
        }
    }

    /// `EXPAND2D(eoflab)`: Expand a row of 2D-encoded data; `Ok(true)` is
    /// `goto eoflab`.
    fn expand2d(&mut self, sp: &mut Fax3CodecState) -> Result<bool, Ret> {
        let exit = 'body: {
            while self.a0 < self.lastx {
                if self.pa >= self.thisrun + sp.nruns as usize {
                    return Err(self.overflow(sp));
                }
                let Some(te) = self.lookup8(7, &TIFF_FAX_MAIN_TABLE) else {
                    break 'body Exit2D::Eof;
                };
                let refruns = sp.refruns.unwrap_or(0) as isize;
                match te.State {
                    S_PASS => {
                        self.check_b1(sp)?;
                        if self.pb + 1 >= refruns + sp.nruns as isize {
                            return Err(self.overflow(sp));
                        }
                        self.b1 = self.b1.wrapping_add(Self::run_at(sp, self.pb) as i32);
                        self.pb += 1;
                        self.RunLength = (self.RunLength as u32)
                            .wrapping_add(self.b1.wrapping_sub(self.a0) as u32)
                            as i32;
                        self.a0 = self.b1;
                        self.b1 = self.b1.wrapping_add(Self::run_at(sp, self.pb) as i32);
                        self.pb += 1;
                    }
                    S_HORIZ => {
                        let black_first = ((self.pa - self.thisrun) & 1) != 0;
                        if let Some(exit) = self.horiz_run(sp, black_first)? {
                            break 'body exit;
                        }
                        if let Some(exit) = self.horiz_run(sp, !black_first)? {
                            break 'body exit;
                        }
                        self.check_b1(sp)?;
                    }
                    S_V0 => {
                        self.check_b1(sp)?;
                        self.setvalue(sp, self.b1.wrapping_sub(self.a0) as u32)?;
                        if self.pb >= refruns + sp.nruns as isize {
                            return Err(self.overflow(sp));
                        }
                        self.b1 = self.b1.wrapping_add(Self::run_at(sp, self.pb) as i32);
                        self.pb += 1;
                    }
                    S_VR => {
                        self.check_b1(sp)?;
                        self.setvalue(
                            sp,
                            (self.b1.wrapping_sub(self.a0) as u32).wrapping_add(te.Param),
                        )?;
                        if self.pb >= refruns + sp.nruns as isize {
                            return Err(self.overflow(sp));
                        }
                        self.b1 = self.b1.wrapping_add(Self::run_at(sp, self.pb) as i32);
                        self.pb += 1;
                    }
                    S_VL => {
                        self.check_b1(sp)?;
                        if self.b1 < (self.a0 as u32).wrapping_add(te.Param) as i32 {
                            self.unexpected(sp);
                            break 'body Exit2D::Eol;
                        }
                        self.setvalue(
                            sp,
                            (self.b1.wrapping_sub(self.a0) as u32).wrapping_sub(te.Param),
                        )?;
                        self.pb -= 1;
                        self.b1 = self.b1.wrapping_sub(Self::run_at(sp, self.pb) as i32);
                    }
                    S_EXT => {
                        let v = self.lastx.wrapping_sub(self.a0) as u32;
                        if let Some(r) = sp.runs.as_mut().and_then(|r| r.get_mut(self.pa)) {
                            *r = v;
                        }
                        self.pa += 1;
                        fax3_extension(self.ctx, sp.line as u32, self.a0 as u32);
                        break 'body Exit2D::Eol;
                    }
                    S_EOL => {
                        let v = self.lastx.wrapping_sub(self.a0) as u32;
                        if let Some(r) = sp.runs.as_mut().and_then(|r| r.get_mut(self.pa)) {
                            *r = v;
                        }
                        self.pa += 1;
                        if !self.need_bits8(4) {
                            break 'body Exit2D::Eof;
                        }
                        if self.get_bits(4) != 0 {
                            self.unexpected(sp);
                        }
                        self.clr_bits(4);
                        self.EOLcnt = 1;
                        break 'body Exit2D::Eol;
                    }
                    _ => break 'body Exit2D::BadMain,
                }
            }
            if self.RunLength != 0 {
                if self.RunLength.wrapping_add(self.a0) < self.lastx {
                    /* expect a final V0 */
                    if !self.need_bits8(1) {
                        break 'body Exit2D::Eof;
                    }
                    if self.get_bits(1) == 0 {
                        break 'body Exit2D::BadMain;
                    }
                    self.clr_bits(1);
                }
                self.setvalue(sp, 0)?;
            }
            Exit2D::Eol
        };
        match exit {
            Exit2D::BadMain => {
                /* badMain2d: */
                self.unexpected(sp);
            }
            Exit2D::BadBlack | Exit2D::BadWhite => {
                /* badBlack2d: badWhite2d: */
                self.unexpected(sp);
            }
            Exit2D::Eof => {
                /* eof2d: */
                self.premature_eof(sp);
                self.cleanup_runs(sp)?;
                return Ok(true);
            }
            Exit2D::Eol => {}
        }
        /* eol2d: */
        self.cleanup_runs(sp)?;
        Ok(false)
    }

    /// `UNCACHE_STATE()`: Save state possibly changed during decoding.
    fn uncache_state(&self, sp: &mut Fax3CodecState) -> (usize, TmSize) {
        sp.bit = self.BitsAvail;
        sp.data = self.BitAcc;
        sp.EOLcnt = self.EOLcnt;
        (self.cp, (self.ep as TmSize - self.cp as TmSize))
    }
}

/// What `SYNC_EOL()` does.
enum Sync {
    Ok,
    /// `goto eoflab`
    Eof,
    /// `goto retrywithouteol`
    Retry,
}

/// Where `EXPAND2D()`'s loop goes.
enum Exit2D {
    Eol,
    Eof,
    BadMain,
    BadBlack,
    BadWhite,
}

/// Run a decoding function over the handle with the state taken out of
/// it; `UNCACHE_STATE()`'s update of the raw data position is the
/// returned `(cp, cc)` applied here.
fn with_state(
    tif: &mut Tiff<'_>,
    module: &'static str,
    f: fn(&mut Dec<'_>, &mut Fax3CodecState, &mut [u8], TmSize) -> (i32, bool),
    buf: &mut [u8],
    occ: TmSize,
) -> i32 {
    let ctx = Ctx::new(tif, module);
    let mut data = std::mem::take(&mut tif.tif_data);
    let TifData::Fax3(sp) = &mut data else {
        tif.tif_data = data;
        return -1;
    };
    let (cp, cc, r, uncached);
    {
        let mut d = Dec {
            ctx,
            a0: 0,
            lastx: sp.rowpixels as i32,
            BitAcc: 0,
            BitsAvail: 0,
            RunLength: 0,
            cp: 0,
            ep: 0,
            raw: &tif.tif_rawdata,
            pa: 0,
            thisrun: 0,
            EOLcnt: 0,
            bitmap: sp.bitmap,
            b1: 0,
            pb: 0,
        };
        d.cp = tif.tif_rawcp;
        d.ep = tif.tif_rawcp;
        let base = (tif.tif_rawcp, tif.tif_rawcc);
        let (ret, did_uncache) = run_cached(&mut d, sp, base, f, buf, occ);
        r = ret;
        uncached = did_uncache;
        cp = d.cp;
        cc = d.ep as TmSize - d.cp as TmSize;
    }
    if uncached {
        tif.tif_rawcc -= cp as TmSize - tif.tif_rawcp as TmSize;
        tif.tif_rawcp = cp;
        let _ = cc;
    }
    tif.tif_data = data;
    r
}

/// `CACHE_STATE()` then the decoding function.
fn run_cached(
    d: &mut Dec<'_>,
    sp: &mut Fax3CodecState,
    base: (usize, TmSize),
    f: fn(&mut Dec<'_>, &mut Fax3CodecState, &mut [u8], TmSize) -> (i32, bool),
    buf: &mut [u8],
    occ: TmSize,
) -> (i32, bool) {
    d.cache_state(base.0, base.1, sp);
    f(d, sp, buf, occ)
}

/// Translation of `Fax3Decode1D()`: Decode the requested amount of G3
/// 1D-encoded data.
/// @param buf destination buffer
/// @param occ available bytes in destination buffer
/// @param s number of planes (ignored)
/// @returns 1 for success, -1 in case of error
fn fax3_decode1d(tif: &mut Tiff<'_>, buf: &mut [u8], occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "Fax3Decode1D";
    let Some(sp) = decoder_state(tif) else {
        return -1;
    };
    if sp.rowbytes <= 0 || occ % sp.rowbytes != 0 {
        tiff_error_ext_r!(MODULE, "Fractional scanlines cannot be read");
        return -1;
    }
    let ctx = Ctx::new(tif, MODULE);
    if let Some(sp) = decoder_state(tif) {
        if check_reached_counters(ctx, sp) != 0 {
            return -1;
        }
    }
    with_state(tif, MODULE, decode1d_body, buf, occ)
}

/// The loop of `Fax3Decode1D()` (from `RETRY_WITHOUT_EOL_1D`); returns
/// the result and whether the state was uncached.
fn decode1d_body(
    d: &mut Dec<'_>,
    sp: &mut Fax3CodecState,
    buf: &mut [u8],
    mut occ: TmSize,
) -> (i32, bool) {
    let base = (d.cp, d.ep as TmSize - d.cp as TmSize);
    let (data0, bit0, eolcnt0) = (sp.data, sp.bit, sp.EOLcnt);
    let mut off = 0usize;
    'retry: loop {
        /* RETRY_WITHOUT_EOL_1D: */
        d.BitAcc = data0;
        d.BitsAvail = bit0;
        d.EOLcnt = eolcnt0;
        d.cp = base.0;
        d.ep = base.0 + base.1.max(0) as usize;
        let _ = (sp.data, sp.bit);
        d.thisrun = sp.curruns;
        while occ > 0 {
            d.a0 = 0;
            d.RunLength = 0;
            d.pa = d.thisrun;
            let eof_a = match d.sync_eol(sp) {
                Sync::Retry => continue 'retry,
                Sync::Eof => {
                    /* EOF1D: premature EOF */
                    if d.cleanup_runs(sp).is_err() {
                        return (-1, false);
                    }
                    true
                }
                Sync::Ok => match d.expand1d(sp) {
                    Err(Ret) => return (-1, false),
                    Ok(eof) => eof,
                },
            };
            let row = buf.get_mut(off..).unwrap_or(&mut []);
            fill(sp, row, d.thisrun, d.pa, d.lastx as u32);
            if eof_a {
                /* EOF1Da: premature EOF */
                d.uncache_state(sp);
                return (-1, true);
            }
            off += sp.rowbytes as usize;
            occ -= sp.rowbytes;
            sp.line += 1;
        }
        d.uncache_state(sp);
        return (1, true);
    }
}

/// `(*sp->fill)(buf, thisrun, pa, lastx)`
fn fill(sp: &mut Fax3CodecState, buf: &mut [u8], thisrun: usize, pa: usize, lastx: u32) {
    let f = sp.fill;
    if let Some(runs) = sp.runs.as_mut() {
        f(buf, runs, thisrun, pa, lastx);
    }
}

/// Translation of `Fax3Decode2D()`: Decode the requested amount of G3
/// 2D-encoded data.
fn fax3_decode2d(tif: &mut Tiff<'_>, buf: &mut [u8], occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "Fax3Decode2D";
    let Some(sp) = decoder_state(tif) else {
        return -1;
    };
    if sp.rowbytes <= 0 || occ % sp.rowbytes != 0 {
        tiff_error_ext_r!(MODULE, "Fractional scanlines cannot be read");
        return -1;
    }
    let ctx = Ctx::new(tif, MODULE);
    if let Some(sp) = decoder_state(tif) {
        if check_reached_counters(ctx, sp) != 0 {
            return -1;
        }
    }
    with_state(tif, MODULE, decode2d_body, buf, occ)
}

/// The loop of `Fax3Decode2D()` (from `RETRY_WITHOUT_EOL_2D`).
fn decode2d_body(
    d: &mut Dec<'_>,
    sp: &mut Fax3CodecState,
    buf: &mut [u8],
    mut occ: TmSize,
) -> (i32, bool) {
    let base = (d.cp, d.ep as TmSize - d.cp as TmSize);
    let (data0, bit0, eolcnt0) = (sp.data, sp.bit, sp.EOLcnt);
    let mut off = 0usize;
    'retry: loop {
        /* RETRY_WITHOUT_EOL_2D: */
        d.BitAcc = data0;
        d.BitsAvail = bit0;
        d.EOLcnt = eolcnt0;
        d.cp = base.0;
        d.ep = base.0 + base.1.max(0) as usize;
        while occ > 0 {
            d.a0 = 0;
            d.RunLength = 0;
            d.thisrun = sp.curruns;
            d.pa = d.thisrun;
            let eof_a = 'row: {
                match d.sync_eol(sp) {
                    Sync::Retry => continue 'retry,
                    Sync::Eof => {
                        /* EOF2D: premature EOF */
                        if d.cleanup_runs(sp).is_err() {
                            return (-1, false);
                        }
                        break 'row true;
                    }
                    Sync::Ok => {}
                }
                if !d.need_bits8(1) {
                    if d.cleanup_runs(sp).is_err() {
                        return (-1, false);
                    }
                    break 'row true;
                }
                let is1d = d.get_bits(1); /* 1D/2D-encoding tag bit */
                d.clr_bits(1);
                d.pb = sp.refruns.unwrap_or(0) as isize;
                d.b1 = Dec::run_at(sp, d.pb) as i32;
                d.pb += 1;
                let r = if is1d != 0 {
                    d.expand1d(sp)
                } else {
                    d.expand2d(sp)
                };
                match r {
                    Err(Ret) => return (-1, false),
                    Ok(eof) => eof,
                }
            };
            let row = buf.get_mut(off..).unwrap_or(&mut []);
            fill(sp, row, d.thisrun, d.pa, d.lastx as u32);
            if eof_a {
                /* EOF2Da: premature EOF */
                d.uncache_state(sp);
                return (-1, true);
            }
            if d.pa < d.thisrun + sp.nruns as usize && d.setvalue(sp, 0).is_err() {
                /* imaginary change for reference */
                return (-1, false);
            }
            let cur = sp.curruns;
            sp.curruns = sp.refruns.unwrap_or(0);
            sp.refruns = Some(cur);
            off += sp.rowbytes as usize;
            occ -= sp.rowbytes;
            sp.line += 1;
        }
        d.uncache_state(sp);
        return (1, true);
    }
}

/// Translation of `_TIFFFax3fillruns()`: Bit-fill a row according to the
/// white/black runs generated during G3/G4 decoding. (The C fills whole
/// 64-bit words where it can: the same bytes.)
pub(crate) fn _tiff_fax3_fillruns(
    buf: &mut [u8],
    runs_arr: &mut [u32],
    runs0: usize,
    erun0: usize,
    lastx: u32,
) {
    static FILLMASKS: [u8; 9] = [0x00, 0x80, 0xc0, 0xe0, 0xf0, 0xf8, 0xfc, 0xfe, 0xff];
    let mut runs = runs0;
    let mut erun = erun0;

    if ((erun - runs) & 1) != 0 {
        if let Some(r) = runs_arr.get_mut(erun) {
            *r = 0;
        }
        erun += 1;
    }
    let get = |a: &[u32], i: usize| a.get(i).copied().unwrap_or(0);
    let mut x: u32 = 0;
    while runs < erun {
        for color in 0..2 {
            let mut run = get(runs_arr, runs + color);
            if x.wrapping_add(run) > lastx || run > lastx {
                run = lastx.wrapping_sub(x);
                if let Some(r) = runs_arr.get_mut(runs + color) {
                    *r = run;
                }
            }
            if run != 0 {
                let mut cp = (x >> 3) as usize;
                let bx = x & 7;
                let mut put = |i: usize, f: &dyn Fn(u8) -> u8| {
                    if let Some(b) = buf.get_mut(i) {
                        *b = f(*b);
                    }
                };
                if run > 8 - bx {
                    if bx != 0 {
                        /* align to byte boundary */
                        if color == 0 {
                            put(cp, &|b| b & (0xffu32 << (8 - bx)) as u8);
                        } else {
                            put(cp, &|b| b | (0xffu32 >> bx) as u8);
                        }
                        cp += 1;
                        run -= 8 - bx;
                    }
                    let n = (run >> 3) as usize;
                    if n != 0 {
                        /* multiple bytes to fill */
                        let v = if color == 0 { 0x00 } else { 0xff };
                        for i in 0..n {
                            put(cp + i, &|_| v);
                        }
                        cp += n;
                        run &= 7;
                    }
                    if run != 0 {
                        if color == 0 {
                            put(cp, &|b| b & (0xffu32 >> run) as u8);
                        } else {
                            /* Explicit 0xff masking to make icc -check=conversions happy */
                            put(cp, &|b| ((b as u32 | (0xff00u32 >> run)) & 0xff) as u8);
                        }
                    }
                } else if color == 0 {
                    put(cp, &|b| b & !(FILLMASKS[run as usize] >> bx));
                } else {
                    put(cp, &|b| b | (FILLMASKS[run as usize] >> bx));
                }
                x = x.wrapping_add(get(runs_arr, runs + color));
            }
        }
        runs += 2;
    }
}

/// Translation of `Fax3FixupTags()`.
fn fax3_fixup_tags(_tif: &mut Tiff<'_>) -> i32 {
    1
}

/// Translation of `Fax3SetupState()`: Setup G3/G4-related
/// compression/decompression state before data is processed.  This
/// routine is called once per image -- it sets up different state based on
/// whether or not decoding or encoding is being done and whether or not
/// 1D- or 2D-encoded data is involved.
fn fax3_setup_state(tif: &mut Tiff<'_>) -> i32 {
    const MODULE: &str = "Fax3SetupState";

    if tif.tif_dir.td_bitspersample != 1 {
        tiff_error_ext_r!(
            MODULE,
            "Bits/sample must be 1 for Group 3/4 encoding/decoding"
        );
        return 0;
    }
    if tif.tif_dir.td_samplesperpixel != 1 && tif.tif_dir.td_planarconfig != PLANARCONFIG_SEPARATE {
        tiff_error_ext_r!(
            MODULE,
            "Samples/pixel shall be 1 for Group 3/4 encoding/decoding, or PlanarConfiguration must be set to Separate."
        );
        return 0;
    }
    /*
     * Calculate the scanline/tile widths.
     */
    let rowbytes: TmSize;
    let rowpixels: u32;
    if tif.is_tiled() {
        rowbytes = super::tif_tile::tiff_tile_row_size(tif);
        rowpixels = tif.tif_dir.td_tilewidth;
    } else {
        rowbytes = tiff_scanline_size(tif);
        rowpixels = tif.tif_dir.td_imagewidth;
    }
    if (rowbytes as i64) < (rowpixels as i64 + 7) / 8 {
        tiff_error_ext_r!(
            MODULE,
            "Inconsistent number of bytes per row : rowbytes={} rowpixels={}",
            rowbytes as i64,
            rowpixels
        );
        return 0;
    }
    let compression = tif.tif_dir.td_compression;
    let tif_name = tif.tif_name.clone();
    let Some(dsp) = decoder_state(tif) else {
        return 0;
    };
    dsp.rowbytes = rowbytes;
    dsp.rowpixels = rowpixels;
    /*
     * Allocate any additional space required for decoding/encoding.
     */
    let needs_ref_line =
        (dsp.groupoptions & GROUP3OPT_2DENCODING) != 0 || compression == COMPRESSION_CCITTFAX4;

    /*
      Assure that allocation computations do not overflow.

      TIFFroundup and TIFFSafeMultiply return zero on integer overflow
    */
    dsp.runs = None;
    dsp.nruns = tiff_roundup_32(rowpixels.wrapping_add(1), 32);
    if needs_ref_line {
        dsp.nruns = dsp.nruns.checked_mul(2).unwrap_or(0);
    }
    if (dsp.nruns == 0) || dsp.nruns.checked_mul(2).unwrap_or(0) == 0 {
        tiff_error_ext_r!(
            tif_name,
            "Row pixels integer overflow (rowpixels {})",
            rowpixels
        );
        return 0;
    }
    let n = dsp.nruns as usize * 2;
    let Some(runs) = try_vec::<u32>(n) else {
        tiff_error_ext_r!(
            tif_name,
            "Failed to allocate memory for {} ({} elements of {} bytes each)",
            "for Group 3/4 run arrays",
            n,
            4
        );
        return 0;
    };
    dsp.runs = Some(runs);
    dsp.curruns = 0;
    if needs_ref_line {
        dsp.refruns = Some(dsp.nruns as usize);
    } else {
        dsp.refruns = None;
    }
    let two_d = compression == COMPRESSION_CCITTFAX3 && is_2d_encoding(dsp);

    if needs_ref_line {
        /* 2d encoding */
        /*
         * 2d encoding requires a scanline
         * buffer for the ``reference line''; the
         * scanline against which delta encoding
         * is referenced.  The reference line must
         * be initialized to be ``white'' (done elsewhere).
         */
        dsp.refline = None;
        match try_vec::<u8>(rowbytes.max(0) as usize) {
            Some(r) => dsp.refline = Some(r),
            None => {
                tiff_error_ext_r!(MODULE, "No space for Group 3/4 reference line");
                return 0;
            }
        }
    } else {
        /* 1d encoding */
        dsp.refline = None;
    }
    if two_d {
        /* NB: default is 1D routine */
        tif.tif_decoderow = fax3_decode2d;
        tif.tif_decodestrip = fax3_decode2d;
        tif.tif_decodetile = fax3_decode2d;
    }

    1
}

/// Translation of `Fax3Cleanup()`.
fn fax3_cleanup(tif: &mut Tiff<'_>) {
    if let Some(sp) = decoder_state(tif) {
        let (vget, vset) = (sp.vgetparent, sp.vsetparent);
        tif.tif_tagmethods.vgetfield = vget;
        tif.tif_tagmethods.vsetfield = vset;
    }

    tif.tif_data = TifData::None;

    _tiff_set_default_compression_state(tif);
}

const FIELD_BADFAXLINES: u16 = FIELD_CODEC;
const FIELD_CLEANFAXDATA: u16 = FIELD_CODEC + 1;
const FIELD_BADFAXRUN: u16 = FIELD_CODEC + 2;

const FIELD_OPTIONS: u16 = FIELD_CODEC + 7;

static FAX_FIELDS: [TIFFField; 5] = [
    field(
        TIFFTAG_FAXMODE,
        0,
        0,
        TIFF_ANY,
        0,
        TIFF_SETGET_INT,
        FIELD_PSEUDO,
        0,
        0,
        "FaxMode",
    ),
    field(
        TIFFTAG_FAXFILLFUNC,
        0,
        0,
        TIFF_ANY,
        0,
        TIFF_SETGET_OTHER,
        FIELD_PSEUDO,
        0,
        0,
        "FaxFillFunc",
    ),
    field(
        TIFFTAG_BADFAXLINES,
        1,
        1,
        TIFF_LONG,
        0,
        TIFF_SETGET_UINT32,
        FIELD_BADFAXLINES,
        1,
        0,
        "BadFaxLines",
    ),
    field(
        TIFFTAG_CLEANFAXDATA,
        1,
        1,
        TIFF_SHORT,
        0,
        TIFF_SETGET_UINT16,
        FIELD_CLEANFAXDATA,
        1,
        0,
        "CleanFaxData",
    ),
    field(
        TIFFTAG_CONSECUTIVEBADFAXLINES,
        1,
        1,
        TIFF_LONG,
        0,
        TIFF_SETGET_UINT32,
        FIELD_BADFAXRUN,
        1,
        0,
        "ConsecutiveBadFaxLines",
    ),
];
static FAX3_FIELDS: [TIFFField; 1] = [field(
    TIFFTAG_GROUP3OPTIONS,
    1,
    1,
    TIFF_LONG,
    0,
    TIFF_SETGET_UINT32,
    FIELD_OPTIONS,
    0,
    0,
    "Group3Options",
)];
static FAX4_FIELDS: [TIFFField; 1] = [field(
    TIFFTAG_GROUP4OPTIONS,
    1,
    1,
    TIFF_LONG,
    0,
    TIFF_SETGET_UINT32,
    FIELD_OPTIONS,
    0,
    0,
    "Group4Options",
)];

/// Translation of `Fax3VSetField()`.
fn fax3_vset_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut VaList<'_>) -> i32 {
    let compression = tif.tif_dir.td_compression;
    let Some(sp) = decoder_state(tif) else {
        return 0;
    };

    match tag {
        TIFFTAG_FAXMODE => {
            sp.mode = ap.int() as i32;
            return 1; /* NB: pseudo tag */
        }
        TIFFTAG_FAXFILLFUNC => {
            // (a function can't be passed: the default fill stays)
            return 1; /* NB: pseudo tag */
        }
        TIFFTAG_GROUP3OPTIONS => {
            /* XXX: avoid reading options if compression mismatches. */
            if compression == COMPRESSION_CCITTFAX3 {
                sp.groupoptions = ap.int() as u32;
            }
        }
        TIFFTAG_GROUP4OPTIONS => {
            /* XXX: avoid reading options if compression mismatches. */
            if compression == COMPRESSION_CCITTFAX4 {
                sp.groupoptions = ap.int() as u32;
            }
        }
        TIFFTAG_BADFAXLINES => sp.badfaxlines = ap.int() as u32,
        TIFFTAG_CLEANFAXDATA => sp.cleanfaxdata = ap.int() as u16,
        TIFFTAG_CONSECUTIVEBADFAXLINES => sp.badfaxrun = ap.int() as u32,
        _ => {
            let parent = sp.vsetparent;
            return parent(tif, tag, ap);
        }
    }

    match tiff_field_with_tag(tif, tag) {
        Some(fip) => tiff_set_field_bit(tif, fip.field_bit),
        None => return 0,
    }

    tif.tif_flags |= TIFF_DIRTYDIRECT;
    1
}

/// Translation of `Fax3VGetField()` (the fill function can't be returned:
/// nothing is pushed for it).
fn fax3_vget_field(tif: &mut Tiff<'_>, tag: u32, ap: &mut Vec<Gv>) -> i32 {
    let Some(sp) = decoder_state(tif) else {
        return 0;
    };

    match tag {
        TIFFTAG_FAXMODE => ap.push(Gv::I32(sp.mode)),
        TIFFTAG_FAXFILLFUNC => {}
        TIFFTAG_GROUP3OPTIONS | TIFFTAG_GROUP4OPTIONS => ap.push(Gv::U32(sp.groupoptions)),
        TIFFTAG_BADFAXLINES => ap.push(Gv::U32(sp.badfaxlines)),
        TIFFTAG_CLEANFAXDATA => ap.push(Gv::U16(sp.cleanfaxdata)),
        TIFFTAG_CONSECUTIVEBADFAXLINES => ap.push(Gv::U32(sp.badfaxrun)),
        _ => {
            let parent = sp.vgetparent;
            return parent(tif, tag, ap);
        }
    }
    1
}

/// Translation of `Fax3GetMaxCompressionRatio()`.
fn fax3_get_max_compression_ratio(_tif: &Tiff<'_>) -> u64 {
    /* See README_for_libtiff_developpers.md for raw data used to estimate
     * the maximum compression rate. */
    /* 1024x1024: 36 */
    /* 4096x4096: 100 */
    /* 16383x16383: 163 */
    /* 65536x65536: 200 */
    /* 200000x200000: 208 */
    250
}

/// Translation of `InitCCITTFax3()`.
fn init_ccitt_fax3(tif: &mut Tiff<'_>) -> i32 {
    /*
     * Merge codec-specific tag information.
     */
    if _tiff_merge_fields(tif, &FAX_FIELDS) == 0 {
        tiff_error_ext_r!(
            "InitCCITTFax3",
            "Merging common CCITT Fax codec-specific tags failed"
        );
        return 0;
    }

    /*
     * Allocate state block so tag methods have storage to record values.
     */
    let sp = Fax3CodecState {
        rw_mode: tif.tif_mode,
        mode: 0,
        rowbytes: 0,
        rowpixels: 0,
        cleanfaxdata: 0,
        badfaxrun: 0,
        badfaxlines: 0,
        groupoptions: 0,
        /*
         * Override parent get/set field methods.
         */
        vgetparent: tif.tif_tagmethods.vgetfield,
        vsetparent: tif.tif_tagmethods.vsetfield,
        bitmap: tiff_get_bit_rev_table(false),
        data: 0,
        bit: 0,
        EOLcnt: 0,
        eofReachedCount: 0,
        eolReachedCount: 0,
        unexpectedReachedCount: 0,
        fill: _tiff_fax3_fillruns,
        runs: None,
        nruns: 0,
        refruns: None,
        curruns: 0,
        refline: None,
        line: 0,
    };
    let rw_mode = sp.rw_mode;
    tif.tif_data = TifData::Fax3(Box::new(sp));
    tif.tif_tagmethods.vgetfield = fax3_vget_field; /* hook for codec tags */
    tif.tif_tagmethods.vsetfield = fax3_vset_field; /* hook for codec tags */
    if rw_mode == O_RDONLY {
        /* FIXME: improve for in place update */
        tif.tif_flags |= TIFF_NOBITREV; /* decoder does bit reversal */
    }
    // (TIFFSetField(tif, TIFFTAG_FAXFILLFUNC, _TIFFFax3fillruns): the
    // state's fill is the default already)
    /*
     * Install codec methods.
     */
    tif.tif_fixuptags = fax3_fixup_tags;
    tif.tif_setupdecode = fax3_setup_state;
    tif.tif_predecode = fax3_pre_decode;
    tif.tif_decoderow = fax3_decode1d;
    tif.tif_decodestrip = fax3_decode1d;
    tif.tif_decodetile = fax3_decode1d;
    tif.tif_cleanup = fax3_cleanup;
    tif.tif_getmaxcompressionratio = fax3_get_max_compression_ratio;
    1
}

/// Translation of `TIFFInitCCITTFax3()`.
pub(crate) fn tiff_init_ccitt_fax3(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    if init_ccitt_fax3(tif) != 0 {
        /*
         * Merge codec-specific tag information.
         */
        if _tiff_merge_fields(tif, &FAX3_FIELDS) == 0 {
            tiff_error_ext_r!(
                "TIFFInitCCITTFax3",
                "Merging CCITT Fax 3 codec-specific tags failed"
            );
            return 0;
        }
        /*
         * The default format is Class/F-style w/o RTC.
         */
        tiff_set_field(tif, TIFFTAG_FAXMODE, &[Va::Int(FAXMODE_CLASSF as i64)])
    } else {
        1
    }
}

/*
 * CCITT Group 4 (T.6) Facsimile-compatible
 * Compression Scheme Support.
 */

/// Translation of `Fax4Decode()`: Decode the requested amount of
/// G4-encoded data.
fn fax4_decode(tif: &mut Tiff<'_>, buf: &mut [u8], occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "Fax4Decode";
    let Some(sp) = decoder_state(tif) else {
        return -1;
    };
    if sp.rowbytes <= 0 || occ % sp.rowbytes != 0 {
        tiff_error_ext_r!(MODULE, "Fractional scanlines cannot be read");
        return -1;
    }
    let ctx = Ctx::new(tif, MODULE);
    if let Some(sp) = decoder_state(tif) {
        if check_reached_counters(ctx, sp) != 0 {
            return -1;
        }
    }
    with_state(tif, MODULE, decode4_body, buf, occ)
}

/// The loop of `Fax4Decode()`.
fn decode4_body(
    d: &mut Dec<'_>,
    sp: &mut Fax3CodecState,
    buf: &mut [u8],
    mut occ: TmSize,
) -> (i32, bool) {
    let start = sp.line;
    let mut off = 0usize;
    while occ > 0 {
        d.a0 = 0;
        d.RunLength = 0;
        d.thisrun = sp.curruns;
        d.pa = d.thisrun;
        d.pb = sp.refruns.unwrap_or(0) as isize;
        d.b1 = Dec::run_at(sp, d.pb) as i32;
        d.pb += 1;
        let eof = match d.expand2d(sp) {
            Err(Ret) => return (-1, false),
            Ok(eof) => eof || d.EOLcnt != 0,
        };
        if eof {
            /* EOFG4: */
            let _ = d.need_bits16(13); /* BADG4: */
            d.clr_bits(13);
            if ((d.lastx + 7) >> 3) as TmSize > occ {
                /* check for buffer overrun */
                tiff_error_ext_r!(
                    d.ctx.module,
                    "Buffer overrun detected : {} bytes available, {} bits needed",
                    occ,
                    d.lastx
                );
                return (-1, false);
            }
            let row = buf.get_mut(off..).unwrap_or(&mut []);
            fill(sp, row, d.thisrun, d.pa, d.lastx as u32);
            d.uncache_state(sp);
            return (if sp.line != start { 1 } else { -1 }, true); /* don't error on badly-terminated strips */
        }
        if ((d.lastx + 7) >> 3) as TmSize > occ {
            /* check for buffer overrun */
            tiff_error_ext_r!(
                d.ctx.module,
                "Buffer overrun detected : {} bytes available, {} bits needed",
                occ,
                d.lastx
            );
            return (-1, false);
        }
        let row = buf.get_mut(off..).unwrap_or(&mut []);
        fill(sp, row, d.thisrun, d.pa, d.lastx as u32);
        if d.setvalue(sp, 0).is_err() {
            /* imaginary change for reference */
            return (-1, false);
        }
        let cur = sp.curruns;
        sp.curruns = sp.refruns.unwrap_or(0);
        sp.refruns = Some(cur);
        off += sp.rowbytes as usize;
        occ -= sp.rowbytes;
        sp.line += 1;
    }
    d.uncache_state(sp);
    (1, true)
}

/// Translation of `Fax4GetMaxCompressionRatio()`.
fn fax4_get_max_compression_ratio(tif: &Tiff<'_>) -> u64 {
    /* FAX4 can compress up to almost one byte per line, so the compression
     * ratio can be up to the tile/strip width.
     * See README_for_libtiff_developpers.md for raw data
     */
    if tif.is_tiled() {
        tif.tif_dir.td_tilewidth as u64
    } else {
        tif.tif_dir.td_imagewidth as u64
    }
}

/// Translation of `TIFFInitCCITTFax4()`.
pub(crate) fn tiff_init_ccitt_fax4(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    if init_ccitt_fax3(tif) != 0 {
        /* reuse G3 support */
        /*
         * Merge codec-specific tag information.
         */
        if _tiff_merge_fields(tif, &FAX4_FIELDS) == 0 {
            tiff_error_ext_r!(
                "TIFFInitCCITTFax4",
                "Merging CCITT Fax 4 codec-specific tags failed"
            );
            return 0;
        }
        tif.tif_decoderow = fax4_decode;
        tif.tif_decodestrip = fax4_decode;
        tif.tif_decodetile = fax4_decode;
        tif.tif_getmaxcompressionratio = fax4_get_max_compression_ratio;
        /*
         * Suppress RTC at the end of each strip.
         */
        tiff_set_field(tif, TIFFTAG_FAXMODE, &[Va::Int(FAXMODE_NORTC as i64)])
    } else {
        0
    }
}

/*
 * CCITT Group 3 1-D Modified Huffman RLE Compression Support.
 * (Compression algorithms 2 and 32771)
 */

/// Translation of `Fax3DecodeRLE()`: Decode the requested amount of
/// RLE-encoded data.
fn fax3_decode_rle(tif: &mut Tiff<'_>, buf: &mut [u8], occ: TmSize, _s: u16) -> i32 {
    const MODULE: &str = "Fax3DecodeRLE";
    let Some(sp) = decoder_state(tif) else {
        return -1;
    };
    if sp.rowbytes <= 0 || occ % sp.rowbytes != 0 {
        tiff_error_ext_r!(MODULE, "Fractional scanlines cannot be read");
        return -1;
    }
    let ctx = Ctx::new(tif, MODULE);
    if let Some(sp) = decoder_state(tif) {
        if check_reached_counters(ctx, sp) != 0 {
            return -1;
        }
    }
    with_state(tif, MODULE, decode_rle_body, buf, occ)
}

/// The loop of `Fax3DecodeRLE()`.
fn decode_rle_body(
    d: &mut Dec<'_>,
    sp: &mut Fax3CodecState,
    buf: &mut [u8],
    mut occ: TmSize,
) -> (i32, bool) {
    let mode = sp.mode;
    let mut off = 0usize;
    d.thisrun = sp.curruns;
    while occ > 0 {
        d.a0 = 0;
        d.RunLength = 0;
        d.pa = d.thisrun;
        let eof = match d.expand1d(sp) {
            Err(Ret) => return (-1, false),
            Ok(eof) => eof,
        };
        let row = buf.get_mut(off..).unwrap_or(&mut []);
        fill(sp, row, d.thisrun, d.pa, d.lastx as u32);
        if eof {
            /* EOFRLE: premature EOF */
            d.uncache_state(sp);
            return (-1, true);
        }
        /*
         * Cleanup at the end of the row.
         */
        if (mode & FAXMODE_BYTEALIGN) != 0 {
            let n = d.BitsAvail - (d.BitsAvail & !7);
            d.clr_bits(n);
        } else if (mode & FAXMODE_WORDALIGN) != 0 {
            let n = d.BitsAvail - (d.BitsAvail & !15);
            d.clr_bits(n);
            // (isAligned(cp, uint16_t): the raw buffer is aligned)
            if d.BitsAvail == 0 && (d.cp & 1) != 0 {
                d.cp += 1;
            }
        }
        off += sp.rowbytes as usize;
        occ -= sp.rowbytes;
        sp.line += 1;
    }
    d.uncache_state(sp);
    (1, true)
}

/// Translation of `Fax3RLEGetMaxCompressionRatio()`.
fn fax3_rle_get_max_compression_ratio(_tif: &Tiff<'_>) -> u64 {
    /* See README_for_libtiff_developpers.md for raw data used to estimate
     * the maximum compression rate. */
    /* 1024x1024: 43 */
    /* 4096x4096: 128 */
    /* 16383x16383: 171 */
    /* 65536x65536: 205 */
    /* 200000x200000: 211 */
    250
}

/// Translation of `TIFFInitCCITTRLE()`.
pub(crate) fn tiff_init_ccitt_rle(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    if init_ccitt_fax3(tif) != 0 {
        /* reuse G3 support */
        tif.tif_decoderow = fax3_decode_rle;
        tif.tif_decodestrip = fax3_decode_rle;
        tif.tif_decodetile = fax3_decode_rle;
        tif.tif_getmaxcompressionratio = fax3_rle_get_max_compression_ratio;
        /*
         * Suppress RTC+EOLs when encoding and byte-align data.
         */
        tiff_set_field(
            tif,
            TIFFTAG_FAXMODE,
            &[Va::Int(
                (FAXMODE_NORTC | FAXMODE_NOEOL | FAXMODE_BYTEALIGN) as i64,
            )],
        )
    } else {
        0
    }
}

/// Translation of `TIFFInitCCITTRLEW()`.
pub(crate) fn tiff_init_ccitt_rlew(tif: &mut Tiff<'_>, _scheme: i32) -> i32 {
    if init_ccitt_fax3(tif) != 0 {
        /* reuse G3 support */
        tif.tif_decoderow = fax3_decode_rle;
        tif.tif_decodestrip = fax3_decode_rle;
        tif.tif_decodetile = fax3_decode_rle;
        tif.tif_getmaxcompressionratio = fax3_rle_get_max_compression_ratio;
        /*
         * Suppress RTC+EOLs when encoding and word-align data.
         */
        tiff_set_field(
            tif,
            TIFFTAG_FAXMODE,
            &[Va::Int(
                (FAXMODE_NORTC | FAXMODE_NOEOL | FAXMODE_WORDALIGN) as i64,
            )],
        )
    } else {
        0
    }
}

#[allow(dead_code)]
const _UNUSED_STATES: [u8; 1] = [S_NULL];
