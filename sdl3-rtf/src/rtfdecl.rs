// Rust translation of src/rtfdecl.h from SDL_rtf.
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * This file was adapted from Microsoft Rich Text Format Specification 1.6
 * http://msdn.microsoft.com/library/default.asp?url=/library/en-us/dnrtfspec/html/rtfspec.asp
 */

// The rtfactn.c and rtfreadr.c prototypes are the methods of `Context` in
// rtfactn.rs and rtfreadr.rs; the "custom rtfreader.c prototypes (defined
// per library)" are in sdl_rtfreadr.rs.

/* RTF parser error codes */

pub(crate) const EC_OK: i32 = 0; /* Everything's fine! */
pub(crate) const EC_STACK_UNDERFLOW: i32 = 1; /* Unmatched '}' */
pub(crate) const EC_STACK_OVERFLOW: i32 = 2; /* Too many '{' -- memory exhausted */
pub(crate) const EC_UNMATCHED_BRACE: i32 = 3; /* RTF ended during an open group. */
pub(crate) const EC_INVALID_HEX: i32 = 4; /* invalid hex character found in data */
pub(crate) const EC_BAD_TABLE: i32 = 5; /* RTF table (sym or prop) invalid */
pub(crate) const EC_ASSERTION: i32 = 6; /* Assertion failure */
pub(crate) const EC_END_OF_FILE: i32 = 7; /* End of file reached while reading RTF */
pub(crate) const EC_FONT_NOT_FOUND: i32 = 8; /* Couldn't find font for text */
