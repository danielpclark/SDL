// Rust translation of src/psaux/psread.c and src/psaux/psread.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright 2007-2013 Adobe Systems Incorporated.
//
// This software, and all works of authorship, whether in source or
// object code form as indicated by the copyright notice(s) included
// herein (collectively, the "Work") is made available, and may only be
// used, modified, and distributed under the FreeType Project License,
// LICENSE.TXT.  Additionally, subject to the terms and conditions of the
// FreeType Project License, each contributor to the Work hereby grants
// to any individual or legal entity exercising permissions granted by
// the FreeType Project License and this section (hereafter, "You" or
// "Your") a perpetual, worldwide, non-exclusive, no-charge,
// royalty-free, irrevocable (except as stated in this section) patent
// license to make, have made, use, offer to sell, sell, import, and
// otherwise transfer the Work, where such license applies only to those
// patent claims licensable by such contributor that are necessarily
// infringed by their contribution(s) alone or by combination of their
// contribution(s) with the Work to which such contribution(s) was
// submitted.  If You institute patent litigation against any entity
// (including a cross-claim or counterclaim in a lawsuit) alleging that
// the Work or a contribution incorporated within the Work constitutes
// direct or contributory patent infringement, then any patent licenses
// granted to You under this License for that Work shall terminate as of
// the date such litigation is filed.
//
// By using, modifying, or distributing the Work you indicate that you
// have read and understood the terms and conditions of the
// FreeType Project License as well as those provided in this section,
// and you accept them fully.
//
// This is an altered (translated) version of the original software; the
// FreeType Project License is in FTL.TXT (see also LICENSE.txt).

//! Adobe's code for stream handling (body).
//!
//! A buffer is a range (`start` to `end`, read at `ptr`) of shared bytes:
//! a charstring, a subroutine index's data, or a seac component. Its error
//! is the font instance's shared error, which the reads are given.

use std::sync::Arc;

use super::psfixed::Cf2Int;

/* Define CF2_IO_FAIL as 1 to enable random errors and random */
/* value errors in I/O.                                       */
/* (CF2_IO_FAIL is 0) */

/// `CF2_BufferRec`
#[derive(Debug, Clone)]
pub struct Cf2BufferRec {
    pub bytes: Arc<[u8]>,
    pub start: usize,
    pub end: usize,
    pub ptr: usize,
}

impl Default for Cf2BufferRec {
    fn default() -> Self {
        Cf2BufferRec {
            bytes: Arc::from(&[][..]),
            start: 0,
            end: 0,
            ptr: 0,
        }
    }
}

/* Region Buffer                                      */
/*                                                    */
/* Can be constructed from a copied buffer managed by */
/* `FCM_getDatablock'.                                */
/* Reads bytes with check for end of buffer.          */

/// `cf2_buf_readByte`: reading past the end of the buffer sets error and
/// returns zero
///
/// (Every buffer C makes is zeroed and never given an `error` pointer, so
/// `CF2_SET_ERROR( buf->error, Invalid_Stream_Operation )` sets nothing:
/// reading past the end only returns zero. The field is left out.)
pub fn cf2_buf_read_byte(buf: &mut Cf2BufferRec) -> Cf2Int {
    if buf.ptr < buf.end {
        let b = buf.bytes.get(buf.ptr).copied().unwrap_or(0);
        buf.ptr += 1;
        b as Cf2Int
    } else {
        /* CF2_SET_ERROR( buf->error, Invalid_Stream_Operation ); */
        0
    }
}

/// `cf2_buf_isEnd`: note: end condition can occur without error
pub fn cf2_buf_is_end(buf: &Cf2BufferRec) -> bool {
    buf.ptr >= buf.end
}
