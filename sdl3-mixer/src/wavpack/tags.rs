// Rust translation of src/tags.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 1998 - 2013 Conifer Software.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Loading the ID3v1 or APEv2 tag that `OPEN_TAGS` asks for.
//!
//! SDL_mixer opens files with `OPEN_TAGS` but doesn't read the tag items
//! (decoder_wavpack.c: "WavPack files use other tag standards; [...] Since
//! we handle those elsewhere, I've removed the WavpackGetTagItem calls"):
//! its own parser has already cut the tags off the stream it hands over.
//! The tag still moves the reader around, and makes `WavpackGetMode()`
//! report `MODE_VALID_TAG`.

// This module provides support for reading metadata tags (either ID3v1 or
// APEv2) from WavPack files. No actual creation or manipulation of the tags
// is done in this module; this is just internal code to load the tags into
// memory. The high-level API functions are in the tag_utils.c module.

use super::open_utils::try_buffer;
use super::*;

// This function attempts to load an ID3v1 or APEv2 tag from the specified
// file into the specified M_Tag structure. The ID3 tag fits in completely,
// but an APEv2 tag is variable length and so space must be allocated here
// to accommodate the data, and this will need to be freed later. A return
// value of TRUE indicates a valid tag was found and loaded. Note that the
// file pointer is undefined when this function exits.

/// Translation of `load_tag()`.
pub(crate) fn load_tag(wpc: &mut WavpackContext<'_>) -> i32 {
    let ape_tag_length: i32;
    let ape_tag_items: i32;
    let reader = &mut *wpc.wv_in;
    let m_tag = &mut wpc.m_tag;

    *m_tag = MTag::default();

    // This is a loop because we can try up to three times to look for an APEv2 tag. In order, we look:
    //
    //  1. At the end of the file for a APEv2 footer (this is the preferred location)
    //  2. If there's instead an ID3v1 tag at the end of the file, try looking for an APEv2 footer right before that
    //  3. If all else fails, look for an APEv2 header the the beginning of the file (use is strongly discouraged)

    loop {
        // seek based on specific location that we are looking for tag (see above list)

        if m_tag.tag_begins_file != 0 {
            // case #3
            reader.set_pos_abs(0);
        } else if m_tag.id3_tag[0] == b'T' {
            // case #2
            reader.set_pos_rel(-((APE_TAG_HDR_SIZE + ID3_TAG_SIZE) as i64), SEEK_END);
        } else {
            // case #1
            reader.set_pos_rel(-(APE_TAG_HDR_SIZE as i64), SEEK_END);
        }

        // read a possible APEv2 tag header/footer and see if there's one there...

        if reader.read_bytes(&mut m_tag.ape_tag_hdr) == APE_TAG_HDR_SIZE as i32
            && &m_tag.ape_tag_hdr[..8] == b"APETAGEX"
        {
            if m_tag.ape_version() == 2000
                && m_tag.ape_item_count() != 0
                && m_tag.ape_length() > APE_TAG_HDR_SIZE as i32
                && m_tag.ape_length() <= APE_TAG_MAX_LENGTH
                && {
                    m_tag.ape_tag_data = try_buffer(m_tag.ape_length() as usize);
                    m_tag.ape_tag_data.is_some()
                }
            {
                ape_tag_items = m_tag.ape_item_count();
                ape_tag_length = m_tag.ape_length();

                // If this is a APEv2 footer (which is normal if we are searching at the end of the file)...

                if (m_tag.ape_flags() & APE_TAG_THIS_IS_HEADER) == 0 {
                    if m_tag.id3_tag[0] == b'T' {
                        m_tag.tag_file_pos = -(ID3_TAG_SIZE as i64);
                    } else {
                        m_tag.tag_file_pos = 0;
                    }

                    m_tag.tag_file_pos -= ape_tag_length as i64;

                    // if the footer claims there is a header present also, we will read that and use it
                    // instead of the footer (after verifying it, of course) for enhanced robustness

                    if (m_tag.ape_flags() & APE_TAG_CONTAINS_HEADER) != 0 {
                        m_tag.tag_file_pos -= APE_TAG_HDR_SIZE as i64;
                    }

                    reader.set_pos_rel(m_tag.tag_file_pos, SEEK_END);

                    if (m_tag.ape_flags() & APE_TAG_CONTAINS_HEADER) != 0 {
                        if reader.read_bytes(&mut m_tag.ape_tag_hdr) != APE_TAG_HDR_SIZE as i32
                            || &m_tag.ape_tag_hdr[..8] != b"APETAGEX"
                        {
                            *m_tag = MTag::default();
                            return FALSE; // something's wrong...
                        }

                        if m_tag.ape_version() != 2000
                            || m_tag.ape_item_count() != ape_tag_items
                            || m_tag.ape_length() != ape_tag_length
                        {
                            *m_tag = MTag::default();
                            return FALSE; // something's wrong...
                        }
                    }
                }

                let want = ape_tag_length as usize - APE_TAG_HDR_SIZE;
                let got = match m_tag.ape_tag_data.as_mut() {
                    Some(data) => reader.read_bytes(&mut data[..want]),
                    None => 0,
                };
                if got != want as i32 {
                    *m_tag = MTag::default();
                    return FALSE; // something's wrong...
                } else {
                    m_tag.id3_tag = [0; ID3_TAG_SIZE]; // ignore ID3v1 tag if we found APEv2 tag
                    return TRUE;
                }
            }
        }

        // we come here if the search for the APEv2 tag failed (otherwise we would have returned with it)

        if m_tag.id3_tag[0] == b'T' {
            // settle for the ID3v1 tag that we found
            m_tag.ape_tag_hdr = [0; APE_TAG_HDR_SIZE];
            return TRUE;
        }

        // if this was the search for the APEv2 tag at the beginning of the file (which is our
        // last resort) then we have nothing, so return failure

        if m_tag.tag_begins_file != 0 {
            *m_tag = MTag::default();
            return FALSE;
        }

        // If we get here, then we have failed the first APEv2 tag search (at end of file) and so now we
        // look for an ID3v1 tag at the same position. If that succeeds, then we'll loop back and look for
        // an APEv2 tag immediately before the ID3v1 tag, otherwise our last resort is to look for an
        // APEv2 tag at the beginning of the file. These are strongly discouraged (and not editable) but
        // they have been seen in the wild so we attempt to handle them here (at least well enough to
        // allow a proper transcoding).

        m_tag.tag_file_pos = -(ID3_TAG_SIZE as i64);
        reader.set_pos_rel(m_tag.tag_file_pos, SEEK_END);

        if reader.read_bytes(&mut m_tag.id3_tag) != ID3_TAG_SIZE as i32
            || &m_tag.id3_tag[..3] != b"TAG"
        {
            m_tag.tag_begins_file = 1; // failed ID3v1, so look for APEv2 at beginning of file
            m_tag.id3_tag = [0; ID3_TAG_SIZE];
        }
    }
}

// Return TRUE is a valid ID3v1 or APEv2 tag has been loaded.

/// Translation of `valid_tag()`.
pub(crate) fn valid_tag(m_tag: &MTag) -> i32 {
    if m_tag.ape_tag_hdr[0] == b'A' {
        b'A' as i32
    } else if m_tag.id3_tag[0] == b'T' {
        b'T' as i32
    } else {
        0
    }
}

// Return FALSE if a valid APEv2 tag was only found at the beginning of the file (these are read-only
// because they cannot be edited without possibly shifting the entire file)

/// Translation of `editable_tag()`.
pub(crate) fn editable_tag(m_tag: &MTag) -> i32 {
    (m_tag.tag_begins_file == 0) as i32
}
