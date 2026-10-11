// Rust translation of src/type1/t1parse.c and src/type1/t1parse.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 1996-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Type 1 parser (body).
//!
//! The Type 1 parser is in charge of the following:
//!
//! - provide an implementation of a growing sequence of objects called
//!   a `T1_Table' (used to build various tables needed by the loader).
//!
//! - opening .pfb and .pfa files to extract their top-level and private
//!   dictionaries.
//!
//! - read numbers, arrays & strings from any dictionary.
//!
//! See `t1load.c' to see how data is loaded from the font file.
//!
//! The dictionaries are owned buffers (a memory stream's base dictionary
//! is a copy of its bytes, which C points to); the root parser's
//! positions are offsets into the dictionary `dict` names. The stream is
//! the caller's, given to the functions reading it.

use super::super::base::ftmemory::ft_qalloc;
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;
use super::super::psaux::psobjs::*;

/// The dictionary the root parser parses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum T1Dict {
    #[default]
    Base,
    Private,
}

/// `T1_ParserRec`: a `PS_ParserRec` is an object used to parse a Type 1
/// fonts very quickly.
///
/// * `root`: the root parser.
/// * `base_dict`: the top-level dictionary.
/// * `base_len`: the length in bytes of the top dictionary.
/// * `private_dict`: the private dictionary.
/// * `private_len`: the length in bytes of the private dictionary.
/// * `in_pfb`: a boolean.  Indicates that we are handling a PFB file.
/// * `in_memory`: a boolean.  Indicates a memory-based stream.
/// * `single_block`: a boolean.  Indicates that the private dictionary
///   is stored in lieu of the base dictionary.
/// * `dict`: the dictionary `root` parses.
#[derive(Debug, Clone, Default)]
pub struct T1ParserRec {
    pub root: PsParserRec,
    pub base_dict: Vec<u8>,
    pub base_len: FtULong,
    pub private_dict: Vec<u8>,
    pub private_len: FtULong,
    pub in_pfb: bool,
    pub in_memory: bool,
    pub single_block: bool,
    pub dict: T1Dict,
}

impl T1ParserRec {
    /// The root parser and the bytes it parses.
    pub fn split(&mut self) -> (&mut PsParserRec, &[u8]) {
        let b = match self.dict {
            T1Dict::Base => &self.base_dict[..],
            T1Dict::Private => &self.private_dict[..],
        };
        (&mut self.root, b)
    }

    /// The bytes the root parser parses.
    pub fn bytes(&self) -> &[u8] {
        match self.dict {
            T1Dict::Base => &self.base_dict[..],
            T1Dict::Private => &self.private_dict[..],
        }
    }
}

/// `T1_Add_Table`
pub fn t1_add_table(table: &mut PsTableRec, i: FtInt, o: &[u8], l: FtUInt) -> FtResult<()> {
    ps_table_add(table, i, o, l)
}

/// `T1_Release_Table`
pub fn t1_release_table(table: &mut PsTableRec) {
    ps_table_release(table);
}

/// `T1_Skip_Spaces`
pub fn t1_skip_spaces(p: &mut T1ParserRec) {
    let (root, b) = p.split();
    ps_parser_skip_spaces(root, b);
}

/// `T1_Skip_PS_Token`
pub fn t1_skip_ps_token(p: &mut T1ParserRec) {
    let (root, b) = p.split();
    ps_parser_skip_ps_token(root, b);
}

/// `T1_ToInt`
pub fn t1_to_int(p: &mut T1ParserRec) -> FtLong {
    let (root, b) = p.split();
    ps_parser_to_int(root, b)
}

/// `T1_ToFixed`
pub fn t1_to_fixed(p: &mut T1ParserRec, t: FtInt) -> FtFixed {
    let (root, b) = p.split();
    ps_parser_to_fixed(root, b, t)
}

/// `T1_ToCoordArray`
pub fn t1_to_coord_array(p: &mut T1ParserRec, m: FtInt, c: Option<&mut [FtShort]>) -> FtInt {
    let (root, b) = p.split();
    ps_parser_to_coord_array(root, b, m, c)
}

/// `T1_ToFixedArray`
pub fn t1_to_fixed_array(
    p: &mut T1ParserRec,
    m: FtInt,
    f: Option<&mut [FtFixed]>,
    t: FtInt,
) -> FtInt {
    let (root, b) = p.split();
    ps_parser_to_fixed_array(root, b, m, f, t)
}

/// `T1_ToToken`
pub fn t1_to_token(p: &mut T1ParserRec, t: &mut T1TokenRec) {
    let (root, b) = p.split();
    ps_parser_to_token(root, b, t);
}

/// `T1_ToTokenArray`
pub fn t1_to_token_array(
    p: &mut T1ParserRec,
    t: Option<&mut [T1TokenRec]>,
    m: FtUInt,
    c: &mut FtInt,
) {
    let (root, b) = p.split();
    ps_parser_to_token_array(root, b, t, m, c);
}

/// `T1_Load_Field`
pub fn t1_load_field<R>(
    p: &mut T1ParserRec,
    f: &T1FieldRec<R>,
    o: &mut [T1Object<'_>],
    m: FtUInt,
) -> FtResult<()> {
    let (root, b) = p.split();
    ps_parser_load_field(root, b, f, f.type_, 0, o, m)
}

/// `T1_Load_Field_Table`
pub fn t1_load_field_table<R>(
    p: &mut T1ParserRec,
    f: &T1FieldRec<R>,
    o: &mut [T1Object<'_>],
    m: FtUInt,
) -> FtResult<()> {
    let (root, b) = p.split();
    ps_parser_load_field_table(root, b, f, o, m)
}

/*************************************************************************/
/*************************************************************************/
/*************************************************************************/
/*****                                                               *****/
/*****                   INPUT STREAM PARSER                         *****/
/*****                                                               *****/
/*************************************************************************/
/*************************************************************************/
/*************************************************************************/

/* see Adobe Technical Note 5040.Download_Fonts.pdf */

/// `read_pfb_tag`
fn read_pfb_tag(
    stream: &mut FtStreamRec,
    atag: &mut FtUShort,
    asize: &mut FtULong,
) -> FtResult<()> {
    *atag = 0;
    *asize = 0;

    let tag = stream.read_ushort()?;
    if tag == 0x8001 || tag == 0x8002 {
        let size = stream.read_ulong_le()?;
        *asize = size as FtULong;
    }

    *atag = tag;

    Ok(())
}

/// `check_type1_format`
fn check_type1_format(stream: &mut FtStreamRec, header_string: &[u8]) -> FtResult<()> {
    let header_length = header_string.len();
    let mut tag: FtUShort = 0;
    let mut dummy: FtULong = 0;

    stream.seek(0)?;

    read_pfb_tag(stream, &mut tag, &mut dummy)?;

    /* We assume that the first segment in a PFB is always encoded as   */
    /* text.  This might be wrong (and the specification doesn't insist */
    /* on that), but we have never seen a counterexample.               */
    if tag != 0x8001 {
        stream.seek(0)?;
    }

    stream.enter_frame(header_length as FtULong)?;

    let mut error = Ok(());

    if stream.frame_data().get(..header_length) != Some(header_string) {
        error = Err(FT_ERR_UNKNOWN_FILE_FORMAT);
    }

    stream.exit_frame();

    /* Exit: */
    error
}

/// `T1_New_Parser`
pub fn t1_new_parser(parser: &mut T1ParserRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let mut tag: FtUShort = 0;
    let mut size: FtULong = 0;

    ps_parser_init(&mut parser.root, 0, 0);

    parser.base_len = 0;
    parser.base_dict = Vec::new();
    parser.private_len = 0;
    parser.private_dict = Vec::new();
    parser.in_pfb = false;
    parser.in_memory = false;
    parser.single_block = false;
    parser.dict = T1Dict::Base;

    let error = (|| -> FtResult<()> {
        /* check the header format */
        if let Err(error) = check_type1_format(stream, b"%!PS-AdobeFont") {
            if error != FT_ERR_UNKNOWN_FILE_FORMAT {
                return Err(error);
            }

            check_type1_format(stream, b"%!FontType")?;
        }

        /*******************************************************************
         *
         * Here a short summary of what is going on:
         *
         *   When creating a new Type 1 parser, we try to locate and load
         *   the base dictionary if this is possible (i.e., for PFB
         *   files).  Otherwise, we load the whole font into memory.
         *
         *   When `loading' the base dictionary, we only setup pointers
         *   in the case of a memory-based stream.  Otherwise, we
         *   allocate and load the base dictionary in it.
         *
         *   parser->in_pfb is set if we are in a binary (`.pfb') font.
         *   parser->in_memory is set if we have a memory stream.
         */

        /* try to compute the size of the base dictionary;     */
        /* look for a Postscript binary file tag, i.e., 0x8001 */
        stream.seek(0)?;

        read_pfb_tag(stream, &mut tag, &mut size)?;

        if tag != 0x8001 {
            /* assume that this is a PFA file for now; an error will */
            /* be produced later when more things are checked        */
            stream.seek(0)?;
            size = stream.size;
        } else {
            parser.in_pfb = true;
        }

        /* now, try to load `size' bytes of the `base' dictionary we */
        /* found previously                                          */

        /* if it is a memory-based resource, set up pointers */
        if stream.read.is_none() {
            parser.in_memory = true;

            /* check that the `size' field is valid */
            let pos = stream.pos();
            stream.skip(size as FtLong)?;

            /* (a copy of the stream's bytes) */
            let base = stream
                .memory_base()
                .cloned()
                .unwrap_or_else(|| Vec::new().into());
            let start = (pos as usize).min(base.len());
            let end = start + (size as usize).min(base.len() - start);
            let mut dict = ft_qalloc((end - start) as FtLong)?;
            dict.copy_from_slice(&base[start..end]);
            parser.base_dict = dict;
            parser.base_len = size;
        } else {
            /* read segment in memory -- this is clumsy, but so does the format */
            /* (a read past the stream's end fails before C's allocation of   */
            /* the bytes is used; it is not made then)                         */
            if size > stream.size.saturating_sub(stream.pos()) && size != 0 {
                return Err(FT_ERR_INVALID_STREAM_OPERATION);
            }
            let mut dict = ft_qalloc(size as FtLong)?;
            stream.read(&mut dict)?;
            parser.base_dict = dict;
            parser.base_len = size;
        }

        parser.root.base = 0;
        parser.root.cursor = 0;
        parser.root.limit = parser.root.cursor + parser.base_len as usize;

        Ok(())
    })();

    /* Exit: */
    if error.is_err() && !parser.in_memory {
        parser.base_dict = Vec::new();
    }

    error
}

/// `T1_Finalize_Parser`
pub fn t1_finalize_parser(parser: &mut T1ParserRec) {
    /* always free the private dictionary */
    parser.private_dict = Vec::new();

    /* free the base dictionary only when we have a disk stream */
    /* (the copy of a memory stream's dictionary goes, too)     */
    parser.base_dict = Vec::new();

    ps_parser_done(&mut parser.root);
}

/// `T1_Get_Private_Dict`
pub fn t1_get_private_dict(parser: &mut T1ParserRec, stream: &mut FtStreamRec) -> FtResult<()> {
    let mut size: FtULong = 0;

    if parser.in_pfb {
        /* in the case of the PFB format, the private dictionary can be  */
        /* made of several segments.  We thus first read the number of   */
        /* segments to compute the total size of the private dictionary  */
        /* then re-read them into memory.                                */
        let start_pos = stream.pos();
        let mut tag: FtUShort = 0;

        parser.private_len = 0;
        loop {
            read_pfb_tag(stream, &mut tag, &mut size)?;

            if tag != 0x8002 {
                break;
            }

            parser.private_len = parser.private_len.wrapping_add(size);

            stream.skip(size as FtLong)?;
        }

        /* Check that we have a private dictionary there */
        /* and allocate private dictionary buffer        */
        if parser.private_len == 0 {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        stream.seek(start_pos)?;
        parser.private_dict = ft_qalloc(parser.private_len as FtLong)?;

        parser.private_len = 0;
        loop {
            if read_pfb_tag(stream, &mut tag, &mut size).is_err() || tag != 0x8002 {
                break;
            }

            let start = parser.private_len as usize;
            let Some(dest) = parser.private_dict.get_mut(start..start + size as usize) else {
                return Err(FT_ERR_INVALID_STREAM_OPERATION);
            };
            stream.read(dest)?;

            parser.private_len += size;
        }
    } else {
        /* We have already `loaded' the whole PFA font file into memory; */
        /* if this is a memory resource, allocate a new block to hold    */
        /* the private dict.  Otherwise, simply overwrite into the base  */
        /* dictionary block in the heap.                                 */

        /* First look for the `eexec' keyword. Ensure `eexec' is real -- */
        /* it could be in a comment or string (as e.g. in u003043t.gsf   */
        /* from ghostscript).                                            */
        parser.dict = T1Dict::Base;
        parser.root.cursor = 0;
        parser.root.limit = parser.base_len as usize;

        let mut cur = parser.root.cursor;
        let mut limit = parser.root.limit;

        let mut found = false;
        while cur < limit {
            let b = &parser.base_dict;
            /* 9 = 5 letters for `eexec' + whitespace + 4 chars */
            if b[cur] == b'e'
                && cur + 9 < limit
                && b[cur + 1] == b'e'
                && b[cur + 2] == b'x'
                && b[cur + 3] == b'e'
                && b[cur + 4] == b'c'
            {
                found = true;
                break;
            }

            t1_skip_ps_token(parser);
            if parser.root.error != 0 {
                break;
            }
            t1_skip_spaces(parser);
            cur = parser.root.cursor;
        }

        if !found {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        /* now determine where to write the _encrypted_ binary private  */
        /* dictionary.  We overwrite the base dictionary for disk-based */
        /* resources and allocate a new block otherwise                 */

        /* Found: */
        parser.root.limit = parser.base_len as usize;

        t1_skip_ps_token(parser);
        cur = parser.root.cursor;
        limit = parser.root.limit;

        /* According to the Type 1 spec, the first cipher byte must not be */
        /* an ASCII whitespace character code (blank, tab, carriage return */
        /* or line feed).  We have seen Type 1 fonts with two line feed    */
        /* characters...  So skip now all whitespace character codes.      */
        /*                                                                 */
        /* On the other hand, Adobe's Type 1 parser handles fonts just     */
        /* fine that are violating this limitation, so we add a heuristic  */
        /* test to stop at \r only if it is not used for EOL.              */

        let b = &parser.base_dict;
        let rest = b.get(cur.min(limit)..limit).unwrap_or(&[]);
        let pos_lf = rest.iter().position(|&c| c == b'\n');
        let pos_cr = rest.iter().position(|&c| c == b'\r');
        /* (C compares the pointers, NULL being the lowest) */
        let test_cr = match (pos_lf, pos_cr) {
            (None, _) => true,
            (Some(_), None) => true,
            (Some(lf), Some(cr)) => lf > cr,
        };

        while cur < limit
            && (b[cur] == b' '
                || b[cur] == b'\t'
                || (test_cr && b[cur] == b'\r')
                || b[cur] == b'\n')
        {
            cur += 1;
        }
        if cur >= limit {
            return Err(FT_ERR_INVALID_FILE_FORMAT);
        }

        size = parser.base_len - cur as FtULong;

        if parser.in_memory {
            /* note that we allocate one more byte to put a terminating `0' */
            parser.private_dict = ft_qalloc(size as FtLong + 1)?;
            parser.private_len = size;
        } else {
            parser.single_block = true;
            parser.private_dict = std::mem::take(&mut parser.base_dict);
            parser.private_len = size;
            parser.base_dict = Vec::new();
            parser.base_len = 0;
        }

        /* now determine whether the private dictionary is encoded in binary */
        /* or hexadecimal ASCII format -- decode it accordingly              */

        /* we need to access the next 4 bytes (after the final whitespace */
        /* following the `eexec' keyword); if they all are hexadecimal    */
        /* digits, then we have a case of ASCII storage                   */

        /* (the bytes after `eexec' are in the private dictionary's block */
        /* when it is the base dictionary's)                              */
        let src: &[u8] = if parser.single_block {
            &parser.private_dict
        } else {
            &parser.base_dict
        };

        if cur + 3 < limit
            && src[cur].is_ascii_hexdigit()
            && src[cur + 1].is_ascii_hexdigit()
            && src[cur + 2].is_ascii_hexdigit()
            && src[cur + 3].is_ascii_hexdigit()
        {
            /* ASCII hexadecimal encoding */
            let mut len: FtULong = 0;

            /* (C decodes in place in a single block; the source is copied) */
            let source: Vec<u8> = if parser.single_block {
                let mut copy = ft_qalloc(limit as FtLong)?;
                copy.copy_from_slice(&src[..limit]);
                copy
            } else {
                Vec::new()
            };
            let source: &[u8] = if parser.single_block {
                &source
            } else {
                &parser.base_dict
            };

            let mut root = parser.root;
            root.cursor = cur;
            let private_len = parser.private_len as usize;
            let _ = ps_parser_to_bytes(
                &mut root,
                source,
                &mut parser.private_dict,
                private_len,
                &mut len,
                false,
            );
            parser.root = root;
            parser.private_len = len;

            /* put a safeguard */
            if let Some(c) = parser.private_dict.get_mut(len as usize) {
                *c = b'\0';
            }
        } else {
            /* binary encoding -- copy the private dict */
            if parser.single_block {
                parser.private_dict.copy_within(cur..cur + size as usize, 0);
            } else {
                parser.private_dict[..size as usize]
                    .copy_from_slice(&parser.base_dict[cur..cur + size as usize]);
            }
        }
    }

    /* we now decrypt the encoded binary private dictionary */
    let private_len = parser.private_len as usize;
    t1_decrypt(&mut parser.private_dict, private_len, 55665);

    if parser.private_len < 4 {
        return Err(FT_ERR_INVALID_FILE_FORMAT);
    }

    /* replace the four random bytes at the beginning with whitespace */
    parser.private_dict[0] = b' ';
    parser.private_dict[1] = b' ';
    parser.private_dict[2] = b' ';
    parser.private_dict[3] = b' ';

    parser.dict = T1Dict::Private;
    parser.root.base = 0;
    parser.root.cursor = 0;
    parser.root.limit = parser.root.cursor + parser.private_len as usize;

    /* Fail: */
    /* Exit: */
    Ok(())
}
