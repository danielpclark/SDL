// Rust translation of src/cid/cidparse.c and src/cid/cidparse.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! CID-keyed Type1 parser (body).
//!
//! The PostScript section is the parser's buffer; the root parser's
//! positions are offsets into it. The stream is the caller's, given to
//! `cid_parser_new`.

use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::super::psaux::psconv::at;
use super::super::psaux::psobjs::*;

/// `CID_Parser`: a `CID_Parser` is an object used to parse a Type 1
/// fonts very quickly.
///
/// * `root`: the root `PS_ParserRec` fields.
/// * `postscript`: the data to be parsed.
/// * `postscript_len`: the length of the data to be parsed.
/// * `data_offset`: the start position of the binary data (i.e., the end
///   of the data to be parsed.
/// * `binary_length`: the length of the data after the `StartData'
///   command if the data format is hexadecimal.
/// * `num_dict`: the number of font dictionaries.
///
/// (C's `cid` is the face's, which the functions using it are given.)
#[derive(Debug, Clone, Default)]
pub struct CidParser {
    pub root: PsParserRec,
    pub postscript: Vec<u8>,
    pub postscript_len: FtULong,
    pub data_offset: FtULong,
    pub binary_length: FtULong,
    pub num_dict: FtUInt,
}

const STARTDATA: &[u8] = b"StartData";
const STARTDATA_LEN: usize = STARTDATA.len();
const SFNTS: &[u8] = b"/sfnts";
const SFNTS_LEN: usize = SFNTS.len();

/// `ft_strncmp( p, s, n ) == 0` for the null-terminated bytes at `p`.
fn strneq(b: &[u8], p: usize, s: &[u8]) -> bool {
    (0..s.len()).all(|i| at(b, p + i) == s[i])
}

/// `cid_parser_new`
pub fn cid_parser_new(parser: &mut CidParser, stream: &mut FtStreamRec) -> FtResult<()> {
    let mut offset: FtULong;

    *parser = CidParser::default();
    ps_parser_init(&mut parser.root, 0, 0);

    let base_offset = stream.pos();

    /* first of all, check the font format in the header */
    if stream.enter_frame(31).is_err() {
        return Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    let mut error = Ok(());
    if stream.frame_data().get(..31) != Some(&b"%!PS-Adobe-3.0 Resource-CIDFont"[..]) {
        error = Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    stream.exit_frame();
    error?;

    loop {
        /* Again: */
        /* now, read the rest of the file until we find */
        /* `StartData' or `/sfnts'                      */
        {
            /*
             * The algorithm is as follows (omitting the case with less than 256
             * bytes to fill for simplicity).
             *
             * 1. Fill the buffer with 256 + STARTDATA_LEN bytes.
             *
             * 2. Search for the STARTDATA and SFNTS strings at positions
             *    buffer[0], buffer[1], ...,
             *    buffer[255 + STARTDATA_LEN - SFNTS_LEN].
             *
             * 3. Move the last STARTDATA_LEN bytes to buffer[0].
             *
             * 4. Fill the buffer with 256 bytes, starting at STARTDATA_LEN.
             *
             * 5. Repeat with step 2.
             *
             */
            let mut buffer = [0u8; 256 + STARTDATA_LEN + 1];

            /* values for the first loop */
            let mut read_len: FtULong = (256 + STARTDATA_LEN) as FtULong;
            let mut read_offset: FtULong = 0;
            let mut p: usize = 0;

            offset = stream.pos();
            'found: loop {
                let stream_len: FtULong = stream.size.wrapping_sub(stream.pos());

                read_len = read_len.min(stream_len);
                stream.read(&mut buffer[p..p + read_len as usize])?;

                /* ensure that we do not compare with data beyond the buffer */
                buffer[p + read_len as usize] = b'\0';

                let limit = p as isize + read_len as isize - SFNTS_LEN as isize;

                p = 0;
                while (p as isize) < limit {
                    if buffer[p] == b'S' && strneq(&buffer, p, STARTDATA) {
                        /* save offset of binary data after `StartData' */
                        offset += (p + STARTDATA_LEN + 1) as FtULong;
                        break 'found;
                    } else if buffer[p + 1] == b's' && strneq(&buffer, p, SFNTS) {
                        offset += (p + SFNTS_LEN + 1) as FtULong;
                        break 'found;
                    }
                    p += 1;
                }

                if read_offset + read_len < STARTDATA_LEN as FtULong {
                    return Err(FT_ERR_INVALID_FILE_FORMAT);
                }

                let from = (read_offset + read_len) as usize - STARTDATA_LEN;
                buffer.copy_within(from..from + STARTDATA_LEN, 0);

                /* values for the next loop */
                read_len = 256;
                read_offset = STARTDATA_LEN as FtULong;
                p = read_offset as usize;

                offset += 256;
            }
        }

        /* Found: */
        /* We have found the start of the binary data or the `/sfnts' token. */
        /* Now rewind and extract the frame corresponding to this PostScript */
        /* section.                                                          */

        let ps_len: FtULong = offset.wrapping_sub(base_offset);
        stream.seek(base_offset)?;
        parser.postscript = stream.extract_frame(ps_len)?;

        parser.data_offset = offset;
        parser.postscript_len = ps_len;
        parser.root.base = 0;
        parser.root.cursor = 0;
        parser.root.limit = parser.root.cursor + ps_len as usize;
        parser.num_dict = FtUInt::MAX;

        /* Finally, we check whether `StartData' or `/sfnts' was real --  */
        /* it could be in a comment or string.  We also get the arguments */
        /* of `StartData' to find out whether the data is represented in  */
        /* binary or hex format.                                          */

        let b = std::mem::take(&mut parser.postscript);
        let r = cid_parser_check_start_data(parser, &b);
        parser.postscript = b;
        if let Some(r) = r {
            return r;
        }

        /* we haven't found the correct `StartData'; go back and continue */
        /* searching                                                      */
        parser.postscript = Vec::new();
        stream.seek(offset)?;
    }

    /* Exit: */
}

/// The check of `cid_parser_new` whether `StartData' or `/sfnts' was real
/// (`b` is the PostScript section): the result, or `None` to continue
/// searching.
fn cid_parser_check_start_data(parser: &mut CidParser, b: &[u8]) -> Option<FtResult<()>> {
    let mut arg1 = parser.root.cursor;
    ps_parser_skip_ps_token(&mut parser.root, b);
    ps_parser_skip_spaces(&mut parser.root, b);
    let mut arg2 = parser.root.cursor;
    ps_parser_skip_ps_token(&mut parser.root, b);
    ps_parser_skip_spaces(&mut parser.root, b);

    let limit = parser.root.limit;
    let mut cur = parser.root.cursor;

    while cur as isize <= limit as isize - SFNTS_LEN as isize {
        if parser.root.error != 0 {
            return Some(Err(parser.root.error));
        }

        if at(b, cur) == b'S'
            && cur as isize <= limit as isize - STARTDATA_LEN as isize
            && strneq(b, cur, STARTDATA)
        {
            let mut type_token = T1TokenRec::default();

            parser.root.cursor = arg1;
            ps_parser_to_token(&mut parser.root, b, &mut type_token);
            let mut error = Ok(());
            if type_token.limit().wrapping_sub(type_token.start()) == 5
                && b.get(type_token.start()..type_token.start() + 5) == Some(&b"(Hex)"[..])
            {
                parser.root.cursor = arg2;
                let binary_length: FtLong = ps_parser_to_int(&mut parser.root, b);
                if binary_length < 0 {
                    error = Err(FT_ERR_INVALID_FILE_FORMAT);
                } else {
                    parser.binary_length = binary_length as FtULong;
                }
            }

            return Some(error);
        } else if at(b, cur + 1) == b's' && strneq(b, cur, SFNTS) {
            return Some(Err(FT_ERR_UNKNOWN_FILE_FORMAT));
        }

        ps_parser_skip_ps_token(&mut parser.root, b);
        ps_parser_skip_spaces(&mut parser.root, b);
        arg1 = arg2;
        arg2 = cur;
        cur = parser.root.cursor;
    }

    None
}

/// `cid_parser_done`
pub fn cid_parser_done(parser: &mut CidParser) {
    /* always free the private dictionary */
    parser.postscript = Vec::new();
    ps_parser_done(&mut parser.root);
}

/*************************************************************************
 *
 *                           PARSING ROUTINES
 *
 */

/// `cid_parser_skip_spaces`
pub fn cid_parser_skip_spaces(p: &mut CidParser) {
    ps_parser_skip_spaces(&mut p.root, &p.postscript);
}

/// `cid_parser_skip_PS_token`
pub fn cid_parser_skip_ps_token(p: &mut CidParser) {
    ps_parser_skip_ps_token(&mut p.root, &p.postscript);
}

/// `cid_parser_to_int`
pub fn cid_parser_to_int(p: &mut CidParser) -> FtLong {
    ps_parser_to_int(&mut p.root, &p.postscript)
}

/// `cid_parser_to_fixed`
pub fn cid_parser_to_fixed(p: &mut CidParser, t: FtInt) -> FtFixed {
    ps_parser_to_fixed(&mut p.root, &p.postscript, t)
}

/// `cid_parser_to_fixed_array`
pub fn cid_parser_to_fixed_array(
    p: &mut CidParser,
    m: FtInt,
    f: Option<&mut [FtFixed]>,
    t: FtInt,
) -> FtInt {
    ps_parser_to_fixed_array(&mut p.root, &p.postscript, m, f, t)
}

/// `cid_parser_to_token`
pub fn cid_parser_to_token(p: &mut CidParser, t: &mut T1TokenRec) {
    ps_parser_to_token(&mut p.root, &p.postscript, t);
}

/// `cid_parser_load_field`
pub fn cid_parser_load_field<R>(
    p: &mut CidParser,
    f: &T1FieldRec<R>,
    o: &mut [T1Object<'_>],
) -> FtResult<()> {
    ps_parser_load_field(&mut p.root, &p.postscript, f, f.type_, 0, o, 0)
}

/// `cid_parser_load_field_table`
pub fn cid_parser_load_field_table<R>(
    p: &mut CidParser,
    f: &T1FieldRec<R>,
    o: &mut [T1Object<'_>],
) -> FtResult<()> {
    ps_parser_load_field_table(&mut p.root, &p.postscript, f, o, 0)
}
