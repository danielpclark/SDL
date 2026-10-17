/*
    id3: ID3v2.3 and ID3v2.4 parsing (a relevant subset)

    copyright 2006-2023 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Thomas Orgis

    WIP: Handling of multiple ID3 tags in a stream.

    1. With update flag: Add non-unique data, replace unique.
    - Only one TALB, TPE1, etc.
    - Only one TXXX with a certain description.
    - Only one COMM with certain language and description.
    - Only one APIC with certain type and description, generally only one
      of type 1 and 2 each.
    2. Without update flag: wipe whole data and only store new stuff.

    BIG BAD BUT: How to properly handle seeks in a stream that make
    the parser encounter the same tags again in random order? Is
    there even a correct way to handle that without storing an
    ordered list of all tags? I could simplify the code here and just
    communicate that a frame should be an update to previous, and
    at which stream position the frame was encountered. But since
    libmpg123 is driven by MPEG frames, there could be multiple
    ID3v2 tags in direct succession treated by the parser without
    the library user being able to interfere.

    This is severely fucked. All that complexity also doesn't matter
    in practice, as streams use ICY and individual files have just one
    ID3v2 tag (relevant for libmpg123).  It's an academic problem. But
    for seekable files, I could implement some jumping logic to find
    and parse all ID3v2 for once and then set a flag that only jumps
    the frames on seeks. That covers all local disk playback. For
    streams, seeking is no issue (seeking back, at least), so the
    update/replace logic works.

    (The quote of the ID3v2 standard on tag location is left out here.)

    For seekable streams, I simply need to implement explicit ID3v2 search along
    that recommendation and keep the complete information. Streams that continue
    growing during playback will not recognize added ID3v2 tags. So be it.
    For non-seekable streams, a tag is always parsed when encountered, assuming
    the order of update tags always matches.

    First step for the 1.26 release shall be the implementaton of the update
    logic and glossing over the theoretical problem of re-parsing update
    frames in the wrong order by ignoring it. They are not that relevant.

    TODO: Cave in and add the missing frames from the spec. Not that far to go.
    But need another data structure to communicate those ...
*/
// Modified 2026-10-17: translated into Rust from id3.h and id3.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! ID3v2 parsing, as libmpg123 does it while it looks for frames: the
//! whole tag is read and its text, comment, TXXX, USLT, RVA2 (and, with
//! `MPG123_PICTURE`, APIC) frames are stored; the RVA values go to the
//! handle's `rva`. (SDL_mixer reads the tags itself and never asks
//! libmpg123 for them; this is what [`Mpg123Handle::id3`] returns.)

use sdl3::stdlib::string::strtod;

use crate::frame::Mpg123Handle;
use crate::mpg123::*;
use crate::readers::{rd_read_frame_body, rd_read_frame_body_vec, rd_skip_bytes};
use crate::stringbuf::*;

/// Translation of `mpg123_text`: an ID3v2 text field (including
/// comments).
#[derive(Clone, Debug, Default)]
pub struct Mpg123Text {
    /// Three-letter language code (not terminated).
    pub lang: [u8; 3],
    /// The ID3v2 text field id, like TALB, TPE2, ... (4 characters, no string termination).
    pub id: [u8; 4],
    /// Empty for the generic comment...
    pub description: Mpg123String,
    /// ...
    pub text: Mpg123String,
}

/// Translation of `mpg123_picture`: an ID3v2 APIC field.
#[derive(Clone, Debug, Default)]
pub struct Mpg123Picture {
    /// mpg123_id3_pic_type value
    pub type_: u8,
    /// description string
    pub description: Mpg123String,
    /// MIME type
    pub mime_type: Mpg123String,
    /// size in bytes
    pub size: usize,
    /// the image data
    pub data: Option<Vec<u8>>,
}

/// Where one of the `mpg123_id3v2` links points (`title`, `artist`, ...):
/// an element of `text` or of `comment_list`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Id3Link {
    /// `&text[i].text`
    Text(usize),
    /// `&comment_list[i].text`
    Comment(usize),
}

/// Translation of `mpg123_id3v2`.
#[derive(Clone, Debug, Default)]
pub struct Mpg123Id3v2 {
    /// 3 or 4 for ID3v2.3 or ID3v2.4.
    pub version: u8,
    /// Title string (pointer into text_list).
    pub title: Option<Id3Link>,
    /// Artist string (pointer into text_list).
    pub artist: Option<Id3Link>,
    /// Album string (pointer into text_list).
    pub album: Option<Id3Link>,
    /// The year as a string (pointer into text_list).
    pub year: Option<Id3Link>,
    /// Genre String (pointer into text_list).
    pub genre: Option<Id3Link>,
    /// Pointer to last encountered comment text with empty description.
    pub comment: Option<Id3Link>,
    /// Array of comments.
    pub comment_list: Vec<Mpg123Text>,
    /// Array of ID3v2 text fields (including USLT)
    pub text: Vec<Mpg123Text>,
    /// The array of extra (TXXX) fields.
    pub extra: Vec<Mpg123Text>,
    /// Array of ID3v2 pictures fields (APIC). Only populated if MPG123_PICTURE flag is set!
    pub picture: Vec<Mpg123Picture>,
}

impl Mpg123Id3v2 {
    /// The string a link points to.
    pub fn get(&self, link: Option<Id3Link>) -> Option<&Mpg123String> {
        match link? {
            Id3Link::Text(i) => self.text.get(i).map(|t| &t.text),
            Id3Link::Comment(i) => self.comment_list.get(i).map(|t| &t.text),
        }
    }
}

/// Translation of `mpg123_id3v1`: the last 128 bytes of a file.
pub type Mpg123Id3v1 = [u8; 128];

/* We know the usual text frames plus some specifics. */
const KNOWN_FRAMES: usize = 5;
static FRAME_TYPE: [&[u8; 4]; KNOWN_FRAMES] = [b"COMM", b"TXXX", b"RVA2", b"USLT", b"APIC"];
#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameTypes {
    Unknown = -2,
    Text = -1,
    Comment = 0,
    Extra,
    Rva2,
    Uslt,
    Picture,
}
const ALL_TYPES: [FrameTypes; KNOWN_FRAMES] = [
    FrameTypes::Comment,
    FrameTypes::Extra,
    FrameTypes::Rva2,
    FrameTypes::Uslt,
    FrameTypes::Picture,
];

/* UTF support definitions */

type TextConverter = fn(&mut Mpg123String, &[u8], bool);

static TEXT_CONVERTERS: [TextConverter; 4] = [
    convert_latin1,
    /* We always check for (multiple) BOM in 16bit unicode. Without BOM, UTF16 BE is the default.
    Errors in encoding are detected anyway. */
    convert_utf16bom,
    convert_utf16bom,
    convert_utf8,
];

static ENCODING_WIDTHS: [usize; 4] = [1, 2, 2, 1];

/// The bytes of a C string: up to its first zero byte.
fn c_str(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// `strcasecmp(a, b) == 0` on C strings.
fn c_strcaseeq(a: &[u8], b: &[u8]) -> bool {
    c_str(a).eq_ignore_ascii_case(c_str(b))
}

/// `strncasecmp(a, b, n) == 0` on C strings.
fn c_strncaseeq(a: &[u8], b: &[u8], n: usize) -> bool {
    let a = c_str(a);
    let b = c_str(b);
    let a = &a[..a.len().min(n)];
    let b = &b[..b.len().min(n)];
    a.eq_ignore_ascii_case(b)
}

/* the code starts here... */

fn null_id3_links(fr: &mut Mpg123Handle<'_>) {
    fr.id3v2.title = None;
    fr.id3v2.artist = None;
    fr.id3v2.album = None;
    fr.id3v2.year = None;
    fr.id3v2.genre = None;
    fr.id3v2.comment = None;
}

/// Translation of `INT123_init_id3()`.
pub(crate) fn init_id3(fr: &mut Mpg123Handle<'_>) {
    fr.id3v2.version = 0; /* nothing there */
    null_id3_links(fr);
    fr.id3v2.comment_list = Vec::new();
    fr.id3v2.text = Vec::new();
    fr.id3v2.extra = Vec::new();
    fr.id3v2.picture = Vec::new();
}

/* Managing of the text, comment and extra lists. */

/* Initialize one element. */
fn init_mpg123_text(txt: &mut Mpg123Text) {
    mpg123_init_string(&mut txt.text);
    mpg123_init_string(&mut txt.description);
    txt.id = [0; 4];
    txt.lang = [0; 3];
}

fn init_mpg123_picture(pic: &mut Mpg123Picture) {
    mpg123_init_string(&mut pic.mime_type);
    mpg123_init_string(&mut pic.description);
    pic.type_ = 0;
    pic.size = 0;
    pic.data = None;
}

/* Free memory of one element. */
fn free_mpg123_text(txt: &mut Mpg123Text) {
    mpg123_free_string(&mut txt.text);
    mpg123_free_string(&mut txt.description);
}

fn free_mpg123_picture(pic: &mut Mpg123Picture) {
    mpg123_free_string(&mut pic.mime_type);
    mpg123_free_string(&mut pic.description);
    pic.data = None;
}

/* Add items to the list. */

/// The lists `add_id3_text()` works on.
#[derive(Clone, Copy)]
enum TextList {
    Comment,
    Text,
}

fn text_list<'b>(fr: &'b mut Mpg123Handle<'_>, list: TextList) -> &'b mut Vec<Mpg123Text> {
    match list {
        TextList::Comment => &mut fr.id3v2.comment_list,
        TextList::Text => &mut fr.id3v2.text,
    }
}

/// Translation of `add_id3_text()`: the index of the entry to (over)write.
fn add_id3_text(
    list: &mut Vec<Mpg123Text>,
    id: Option<&[u8; 4]>,
    lang: Option<&[u8; 3]>,
    description: Option<&Mpg123String>,
) -> Option<usize> {
    if lang.is_some() && description.is_none() {
        return None; // no lone language intended
    }
    if id.is_some() || description.is_some() {
        // Look through list of existing texts and return an existing entry
        // if it should be overwritten.
        for (i, entry) in list.iter().enumerate() {
            if let Some(description) = description {
                // Overwrite entry with same description and same ID and language.
                if id.is_none_or(|id| *id == entry.id)
                    && lang.is_none_or(|lang| entry.lang == *lang)
                    && mpg123_same_string(&entry.description, description)
                {
                    return Some(i);
                }
            } else if id.is_some_and(|id| *id == entry.id) {
                return Some(i); // Just ovewrite because of same ID.
            }
        }
    }
    // Nothing found, add new one.
    if list.try_reserve(1).is_err() {
        return None; /* bad */
    }

    let mut t = Mpg123Text::default();
    init_mpg123_text(&mut t);
    list.push(t);

    Some(list.len() - 1) /* Return pointer to the added text. */
}

fn add_id3_picture(
    list: &mut Vec<Mpg123Picture>,
    type_: u8,
    description: &Mpg123String,
) -> Option<usize> {
    // Return entry to overwrite, if appropriate.
    for (i, entry) in list.iter().enumerate() {
        if type_ == entry.type_
            && (type_ == 1 || type_ == 2 || mpg123_same_string(&entry.description, description))
        {
            return Some(i);
        }
    }
    // Append a new one.
    if list.try_reserve(1).is_err() {
        return None; /* bad */
    }

    let mut p = Mpg123Picture::default();
    init_mpg123_picture(&mut p);
    list.push(p);

    Some(list.len() - 1) /* Return pointer to the added picture. */
}

/* OK, back to the higher level functions. */

/// Translation of `INT123_exit_id3()`.
pub(crate) fn exit_id3(fr: &mut Mpg123Handle<'_>) {
    fr.id3v2.picture = Vec::new();
    fr.id3v2.comment_list = Vec::new();
    fr.id3v2.extra = Vec::new();
    fr.id3v2.text = Vec::new();
}

/// Translation of `INT123_reset_id3()`.
pub(crate) fn reset_id3(fr: &mut Mpg123Handle<'_>) {
    exit_id3(fr);
    init_id3(fr);
}

/* Set the id3v2.artist id3v2.title ... links to elements of the array. */
/// Translation of `INT123_id3_link()`.
pub(crate) fn id3_link(fr: &mut Mpg123Handle<'_>) {
    null_id3_links(fr);
    let v2 = &mut fr.id3v2;
    for (i, entry) in v2.text.iter().enumerate() {
        if &entry.id == b"TIT2" {
            v2.title = Some(Id3Link::Text(i));
        } else if &entry.id == b"TALB" {
            v2.album = Some(Id3Link::Text(i));
        } else if &entry.id == b"TPE1" {
            v2.artist = Some(Id3Link::Text(i));
        } else if &entry.id == b"TYER" {
            v2.year = Some(Id3Link::Text(i));
        } else if &entry.id == b"TCON" {
            v2.genre = Some(Id3Link::Text(i));
        }
    }
    for (i, entry) in v2.comment_list.iter().enumerate() {
        if entry.description.fill == 0 || entry.description.p[0] == 0 {
            v2.comment = Some(Id3Link::Comment(i));
        }
    }
    /* When no generic comment found, use the last non-generic one. */
    if v2.comment.is_none() && !v2.comment_list.is_empty() {
        v2.comment = Some(Id3Link::Comment(v2.comment_list.len() - 1));
    }
}

/*
    Store ID3 text data in an mpg123_string; either verbatim copy or
    everything translated to UTF-8 encoding.
    Preserve the zero string separator (I don't need strlen for the total size).

    Since we can overwrite strings with ID3 update frames, don't free
    memory, just grow strings.
*/
fn store_id3_text(
    sb: &mut Mpg123String,
    encoding: u8,
    source: &[u8],
    noquiet: bool,
    notranslate: bool,
) {
    // Always overwrite, even with nothing.
    sb.fill = 0;
    let source_size = source.len();
    if source_size == 0 {
        // Empty id3 data!
        return;
    }

    /* We shall just copy the data. Client wants to decode itself. */
    if notranslate {
        /* Future: Add a path for ID3 errors. */
        if !mpg123_grow_string(sb, source_size + 1) {
            // Cannot resize target string, out of memory?
            return;
        }
        sb.p[0] = encoding;
        sb.p[1..1 + source_size].copy_from_slice(source);
        sb.fill = source_size + 1;
        return;
    }

    if encoding > MPG123_ID3_ENC_MAX {
        // Unknown text encoding %u, I take no chances, sorry!
        return;
    }
    id3_to_utf8(sb, encoding, source, noquiet);

    // (unable to convert string to UTF-8 (out of memory, junk input?)! on failure)
}

/* On error, sb->size is 0. */
/* Also, encoding has been checked already! */
/// Translation of `INT123_id3_to_utf8()`.
pub(crate) fn id3_to_utf8(sb: &mut Mpg123String, encoding: u8, source: &[u8], noquiet: bool) {
    let mut source = source;
    sb.fill = 0;
    /* A note: ID3v2.3 uses UCS-2 non-variable 16bit encoding, v2.4 uses UTF16.
    UTF-16 uses a reserved/private range in UCS-2 to add the magic, so we just always treat it as UTF. */
    let bwidth = ENCODING_WIDTHS[encoding as usize];
    /* Hack! I've seen a stray zero byte before BOM. Is that supposed to happen? */
    if encoding != MPG123_ID3_UTF16BE {
        /* UTF16be _can_ beging with a null byte! */
        while source.len() > bwidth && source[0] == 0 {
            source = &source[1..];
        }
    }
    let mut source_size = source.len();
    if source_size % bwidth != 0 {
        /* When we need two bytes for a character, it's strange to have an uneven bytestream length. */
        source_size -= source_size % bwidth;
    }
    TEXT_CONVERTERS[encoding as usize](sb, &source[..source_size], noquiet);
}

/* You have checked encoding to be in the range already. */
/// Translation of `next_text()`: the offset (from `prev`) of the text after
/// the zero-terminated one at `prev`, or `None`.
fn next_text(prev: &[u8], encoding: u8, limit: usize) -> Option<usize> {
    let mut text = 0usize;
    let width = ENCODING_WIDTHS[encoding as usize];

    if limit > isize::MAX as usize {
        return None;
    }

    /* So I go lengths to find zero or double zero...
    Remember bug 2834636: Only check for aligned NULLs! */
    while text < limit {
        if prev[text] == 0 {
            if width <= limit - text {
                let mut i = 1;
                while i < width {
                    if prev[text + i] != 0 {
                        break;
                    }
                    i += 1;
                }

                if i == width {
                    /* found a null wide enough! */
                    text += width;
                    break;
                }
            } else {
                return None; /* No full character left? This text is broken */
            }
        }

        text += width;
    }
    if text >= limit {
        None
    } else {
        Some(text)
    }
}

/// `realdata` (the frame data, with what follows it in its buffer) and
/// `realsize` (the frame's size) of a frame being processed.
fn process_text(fr: &mut Mpg123Handle<'_>, realdata: &[u8], realsize: usize, id: &[u8; 4]) {
    /* Text encoding          $xx */
    /* The text (encoded) ... */
    if realsize < 1 {
        // Not even an encoding byte?
        return;
    }
    let noquiet = fr.noquiet();
    let plain = fr.p.flags & MPG123_PLAIN_ID3TEXT != 0;
    let Some(t) = add_id3_text(&mut fr.id3v2.text, Some(id), None, None) else {
        // Unable to attach new text!
        return;
    };
    let t = &mut fr.id3v2.text[t];
    t.id = *id;
    store_id3_text(
        &mut t.text,
        realdata[0],
        &realdata[1..realsize],
        noquiet,
        plain,
    );
}

fn process_picture(fr: &mut Mpg123Handle<'_>, realdata: &[u8], realsize: usize) {
    let mut mime = Mpg123String::default();
    let mut description = Mpg123String::default();
    if realsize < 1 {
        // Empty id3 data!
        return;
    }
    let noquiet = fr.noquiet();
    let encoding = realdata[0];
    let mut realdata = &realdata[1..];
    let mut realsize = realsize - 1;
    if encoding > MPG123_ID3_ENC_MAX {
        // Unknown text encoding %u, I take no chances, sorry!
        return;
    }

    /* get mime type (encoding is always latin-1) */
    let Some(workpoint) = next_text(realdata, 0, realsize) else {
        // Unable to get mime type for picture; skipping picture.
        return;
    };
    id3_to_utf8(&mut mime, 0, &realdata[..workpoint], noquiet);
    realsize -= workpoint;
    realdata = &realdata[workpoint..];
    /* get picture type */
    let image_type = realdata[0];
    realdata = &realdata[1..];
    realsize = realsize.wrapping_sub(1);
    /* get description (encoding is encoding) */
    let Some(workpoint) = next_text(realdata, encoding, realsize) else {
        // Unable to get description for picture; skipping picture.
        return;
    };
    id3_to_utf8(&mut description, encoding, &realdata[..workpoint], noquiet);
    realsize -= workpoint;
    let mut image_data = Vec::new();
    if realsize == 0 || image_data.try_reserve_exact(realsize).is_err() {
        // No picture data or malloc failure; skipping picture.
        return;
    }
    image_data.extend_from_slice(&realdata[workpoint..workpoint + realsize]);

    // All data ready now, append to/replace in list.
    let Some(i) = add_id3_picture(&mut fr.id3v2.picture, image_type, &description) else {
        // Unable to attach new picture!
        return;
    };

    // Either this is a fresh image, or one to be replaced.
    // We hand over memory, so old storage needs to be freed.
    let i = &mut fr.id3v2.picture[i];
    free_mpg123_picture(i);
    i.type_ = image_type;
    i.size = realsize;
    i.data = Some(image_data);
    mpg123_move_string(&mut mime, &mut i.mime_type);
    mpg123_move_string(&mut description, &mut i.description);
}

/* Store a new comment that perhaps is a RVA / RVA_ALBUM/AUDIOPHILE / RVA_MIX/RADIO one
Special gimmik: It also stores USLT to the texts. Stucture is the same as for comments. */
fn process_comment(
    fr: &mut Mpg123Handle<'_>,
    tt: FrameTypes,
    realdata: &[u8],
    realsize: usize,
    rva_level: i32,
    id: &[u8; 4],
) {
    /* Text encoding          $xx */
    /* Language               $xx xx xx */
    /* Short description (encoded!)      <text> $00 (00) */
    /* Then the comment text (encoded) ... */
    let encoding = realdata[0];
    let mut lang = [0u8; 3]; // realdata + 1
    let descr = 4usize;
    let mut localcom = Mpg123Text::default(); // UTF-8 variant for local processing, remember to clean up!
    init_mpg123_text(&mut localcom);
    let noquiet = fr.noquiet();
    let plain = fr.p.flags & MPG123_PLAIN_ID3TEXT != 0;

    if realsize < descr {
        // Invalid frame size of %zu (too small for anything).
        return;
    }
    if encoding > MPG123_ID3_ENC_MAX {
        // Unknown text encoding %u, I take no chances, sorry!
        return;
    }
    lang.copy_from_slice(&realdata[1..4]);
    /* Be careful with finding the end of description, I have to honor encoding here. */
    let Some(text) = next_text(&realdata[descr..], encoding, realsize - descr) else {
        // No comment text / valid description?
        return;
    };
    let text = descr + text;
    let xcom;
    {
        // just vor variable scope
        let mut description = Mpg123String::default();
        // Store the text, with desired encoding, but for comments always a local copy in UTF-8.
        store_id3_text(
            &mut description,
            encoding,
            &realdata[descr..text],
            noquiet,
            plain,
        );
        if tt == FrameTypes::Comment {
            store_id3_text(
                &mut localcom.description,
                encoding,
                &realdata[descr..text],
                noquiet,
                false,
            );
        }
        let list = if tt == FrameTypes::Uslt {
            TextList::Text
        } else {
            TextList::Comment
        };
        // (add_uslt() passes id, add_comment() NULL)
        let xid = if tt == FrameTypes::Uslt {
            Some(id)
        } else {
            None
        };
        let Some(x) = add_id3_text(text_list(fr, list), xid, Some(&lang), Some(&description))
        else {
            // Unable to attach new comment!
            return;
        };
        let entry = &mut text_list(fr, list)[x];
        entry.id = *id;
        entry.lang = lang;
        // That takes over the description allocation.
        mpg123_move_string(&mut description, &mut entry.description);
        xcom = (list, x);
    }

    let entry = &mut text_list(fr, xcom.0)[xcom.1];
    store_id3_text(
        &mut entry.text,
        encoding,
        &realdata[text..realsize],
        noquiet,
        plain,
    );
    /* Remember: I will probably decode the above (again) for rva comment checking. So no messing around, please. */

    /* Look out for RVA info only when we really deal with a straight comment. */
    if tt == FrameTypes::Comment && localcom.description.fill > 0 {
        let mut rva_mode = -1; /* mix / album */
        let d = &localcom.description.p;
        if c_strcaseeq(d, b"rva")
            || c_strcaseeq(d, b"rva_mix")
            || c_strcaseeq(d, b"rva_track")
            || c_strcaseeq(d, b"rva_radio")
        {
            rva_mode = 0;
        } else if c_strcaseeq(d, b"rva_album")
            || c_strcaseeq(d, b"rva_audiophile")
            || c_strcaseeq(d, b"rva_user")
        {
            rva_mode = 1;
        }
        if (rva_mode > -1) && (fr.rva.level[rva_mode as usize] <= rva_level) {
            /* Only translate the contents in here where we really need them. */
            store_id3_text(
                &mut localcom.text,
                encoding,
                &realdata[text..realsize],
                noquiet,
                false,
            );
            if localcom.text.fill > 0 {
                fr.rva.gain[rva_mode as usize] = strtod(c_str(&localcom.text.p)).0 as f32;
                fr.rva.peak[rva_mode as usize] = 0.0;
                fr.rva.level[rva_mode as usize] = rva_level;
            }
        }
    }
    /* Make sure to free the local memory... */
    free_mpg123_text(&mut localcom);
}

fn process_extra(
    fr: &mut Mpg123Handle<'_>,
    realdata: &[u8],
    realsize: usize,
    rva_level: i32,
    id: &[u8; 4],
) {
    /* Text encoding          $xx */
    /* Description        ... $00 (00) */
    /* Text ... */
    let encoding = realdata[0];
    let descr = 1usize; /* remember, the encoding is descr[-1] */
    let noquiet = fr.noquiet();
    let plain = fr.p.flags & MPG123_PLAIN_ID3TEXT != 0;

    if (realsize as i32) < descr as i32 {
        // Invalid frame size of %lu (too small for anything).
        return;
    }
    if encoding > MPG123_ID3_ENC_MAX {
        // Unknown text encoding %u, I take no chances, sorry!
        return;
    }
    let text = next_text(&realdata[descr..], encoding, realsize - descr);
    let Some(text) = text else {
        // No extra frame text / valid description?
        return;
    };
    let text = descr + text;
    let xex;
    {
        // just vor variable scope
        let mut description = Mpg123String::default();
        /* The outside storage gets reencoded to UTF-8 only if not requested otherwise.
        Remember that we really need the -1 here to hand in the encoding byte!*/
        store_id3_text(
            &mut description,
            encoding,
            &realdata[descr..text],
            noquiet,
            plain,
        );
        xex = add_id3_text(&mut fr.id3v2.extra, None, None, Some(&description));
        if let Some(x) = xex {
            mpg123_move_string(&mut description, &mut fr.id3v2.extra[x].description);
        } else {
            mpg123_free_string(&mut description);
        }
    }
    let Some(xex) = xex else {
        // Unable to attach new extra text!
        return;
    };
    fr.id3v2.extra[xex].id = *id;
    let mut localex = Mpg123Text::default();
    init_mpg123_text(&mut localex); /* For our local copy. */

    /* Our local copy is always stored in UTF-8! */
    store_id3_text(
        &mut localex.description,
        encoding,
        &realdata[descr..text],
        noquiet,
        false,
    );
    /* At first, only store the outside copy of the payload. We may not need the local copy. */
    store_id3_text(
        &mut fr.id3v2.extra[xex].text,
        encoding,
        &realdata[text..realsize],
        noquiet,
        plain,
    );

    /* Now check if we would like to interpret this extra info for RVA. */
    if localex.description.fill > 0 {
        let mut is_peak = false;
        let mut rva_mode = -1; /* mix / album */
        let d = &localex.description.p;

        if c_strncaseeq(d, b"replaygain_track_", 17) {
            rva_mode = 0;
            if c_strcaseeq(d, b"replaygain_track_peak") {
                is_peak = true;
            } else if !c_strcaseeq(d, b"replaygain_track_gain") {
                rva_mode = -1;
            }
        } else if c_strncaseeq(d, b"replaygain_album_", 17) {
            rva_mode = 1;
            if c_strcaseeq(d, b"replaygain_album_peak") {
                is_peak = true;
            } else if !c_strcaseeq(d, b"replaygain_album_gain") {
                rva_mode = -1;
            }
        }
        if (rva_mode > -1) && (fr.rva.level[rva_mode as usize] <= rva_level) {
            /* Now we need the translated copy of the data. */
            store_id3_text(
                &mut localex.text,
                encoding,
                &realdata[text..realsize],
                noquiet,
                false,
            );
            if localex.text.fill > 0 {
                let v = strtod(c_str(&localex.text.p)).0 as f32;
                if is_peak {
                    fr.rva.peak[rva_mode as usize] = v;
                } else {
                    fr.rva.gain[rva_mode as usize] = v;
                }
                fr.rva.level[rva_mode as usize] = rva_level;
            }
        }
    }

    free_mpg123_text(&mut localex);
}

/* Make a ID3v2.3+ 4-byte ID from a ID3v2.2 3-byte ID
Note that not all frames survived to 2.4; the mapping goes to 2.3 .
A notable miss is the old RVA frame, which is very unspecific anyway.
This function returns -1 when a not known 3 char ID was encountered, 0 otherwise. */
fn promote_framename(id: &mut [u8; 5]) -> i32 {
    static OLD: [&[u8; 3]; 37] = [
        b"COM", b"TAL", b"TBP", b"TCM", b"TCO", b"TCR", b"TDA", b"TDY", b"TEN", b"TFT", b"TIM",
        b"TKE", b"TLA", b"TLE", b"TMT", b"TOA", b"TOF", b"TOL", b"TOR", b"TOT", b"TP1", b"TP2",
        b"TP3", b"TP4", b"TPA", b"TPB", b"TRC", b"TDA", b"TRK", b"TSI", b"TSS", b"TT1", b"TT2",
        b"TT3", b"TXT", b"TXX", b"TYE",
    ];
    static NEW: [&[u8; 4]; 37] = [
        b"COMM", b"TALB", b"TBPM", b"TCOM", b"TCON", b"TCOP", b"TDAT", b"TDLY", b"TENC", b"TFLT",
        b"TIME", b"TKEY", b"TLAN", b"TLEN", b"TMED", b"TOPE", b"TOFN", b"TOLY", b"TORY", b"TOAL",
        b"TPE1", b"TPE2", b"TPE3", b"TPE4", b"TPOS", b"TPUB", b"TSRC", b"TRDA", b"TRCK", b"TSIZ",
        b"TSSE", b"TIT1", b"TIT2", b"TIT3", b"TEXT", b"TXXX", b"TYER",
    ];
    for i in 0..OLD.len() {
        // (strncmp(id, old[i], 3): the id's characters have been checked to be no zeros)
        if id[..3] == OLD[i][..] {
            id[..4].copy_from_slice(NEW[i]);
            return 0;
        }
    }
    -1
}

fn store_id3v2(fr: &mut Mpg123Handle<'_>, first4bytes: u64, buf: &[u8; 6], length: u64) -> i32 {
    let mut ret = 1;
    let fullen = 10 + length;
    fr.id3v2_raw = None;
    fr.id3v2_size = 0;
    /* Allocate one byte more for a closing zero as safety catch for strlen(). */
    let mut raw = Vec::new();
    if raw.try_reserve_exact(fullen as usize + 1).is_err() {
        fr.err = MPG123_OUT_OF_MEM;
        // ID3v2: Arrg! Unable to allocate %lu bytes for ID3v2 data - trying to skip instead.
        let ret2 = rd_skip_bytes(fr, length as i64);
        if ret2 < 0 {
            ret = ret2 as i32;
        } else {
            ret = 0;
        }
    } else {
        raw.push(((first4bytes >> 24) & 0xff) as u8);
        raw.push(((first4bytes >> 16) & 0xff) as u8);
        raw.push(((first4bytes >> 8) & 0xff) as u8);
        raw.push((first4bytes & 0xff) as u8);
        raw.extend_from_slice(buf);
        let ret2 = rd_read_frame_body_vec(fr, &mut raw, length as usize);
        if ret2 < 0 {
            ret = ret2;
        } else {
            /* Closing with a zero for paranoia. */
            raw.push(0);
            fr.id3v2_raw = Some(raw);
            fr.id3v2_size = fullen as usize;
        }
    }
    ret
}

/*
    trying to parse ID3v2.3 and ID3v2.4 tags...

    returns:  0: bad or just unparseable tag
              1: good, (possibly) new tag info
             <0: reader error (may need more data feed, try again)
*/
/// Translation of `INT123_parse_new_id3()`.
pub(crate) fn parse_new_id3(fr: &mut Mpg123Handle<'_>, first4bytes: u64) -> i32 {
    const UNSYNC_FLAG: u8 = 128;
    const EXTHEAD_FLAG: u8 = 64; /* ID3v2.3+ */
    const COMPRESS_FLAG: u8 = 64; /* ID3v2.2 */
    const FOOTER_FLAG: u8 = 16;
    const EXT_UPDATE_FLAG: u8 = 64; /* ID3v2.4 only: extended header update flag */
    const UNKNOWN_FLAGS: u8 = 15; /* 00001111*/
    let mut buf = [0u8; 6];
    let mut length: u64 = 0;
    let mut ret = 1;
    let mut storetag = false;
    let mut footlen = 0u64;
    let mut skiptag = false;
    let major = (first4bytes & 0xff) as u8;

    if major == 0xff {
        return 0; /* Invalid... */
    }
    let ret2 = rd_read_frame_body(fr, &mut buf); /* read more header information */
    if ret2 < 0 {
        return ret2;
    }

    if buf[0] == 0xff {
        return 0; /* Revision, will never be 0xff. */
    }

    if fr.p.flags & MPG123_STORE_RAW_ID3 != 0 {
        storetag = true;
    }
    /* second new byte are some nice flags, if these are invalid skip the whole thing */
    let flags = buf[1];
    /* use 4 bytes from buf to construct 28bit uint value and return 1; return 0 if bytes are not synchsafe */
    let synchsafe_to_long = |buf: &[u8], res: &mut u64| -> bool {
        if (buf[0] | buf[1] | buf[2] | buf[3]) & 0x80 != 0 {
            false
        } else {
            *res = ((buf[0] as u64) << 21)
                | ((buf[1] as u64) << 14)
                | ((buf[2] as u64) << 7)
                | (buf[3] as u64);
            true
        }
    };
    /* id3v2.3 does not store synchsafe frame sizes, but synchsafe tag size - doh! */
    /* Remember: bytes_to_long() can yield ULONG_MAX on 32 bit platforms! */
    let bytes_to_long = |buf: &[u8], res: &mut u64| -> bool {
        if major == 3 {
            *res = ((buf[0] as u64) << 24)
                | ((buf[1] as u64) << 16)
                | ((buf[2] as u64) << 8)
                | (buf[3] as u64);
            true
        } else {
            synchsafe_to_long(buf, res)
        }
    };
    /* for id3v2.2 only */
    let threebytes_to_long =
        |buf: &[u8]| -> u64 { ((buf[0] as u64) << 16) | ((buf[1] as u64) << 8) | (buf[2] as u64) };

    /* length-10 or length-20 (footer present); 4 synchsafe integers == 28 bit number  */
    /* we have already read 10 bytes, so left are length or length+10 bytes belonging to tag */
    /* Note: This is an 28 bit value in 32 bit storage, plenty of space for */
    /* length+x for reasonable x. */
    if !synchsafe_to_long(&buf[2..], &mut length) {
        // Bad tag length (not synchsafe): 0x%02x%02x%02x%02x; You got a bad ID3 tag here.
        return 0;
    }
    if flags & FOOTER_FLAG != 0 {
        footlen = 10;
    }
    /* skip if unknown version/scary flags, parse otherwise */
    if fr.p.flags & MPG123_SKIP_ID3V2 != 0 {
        // Note: Skipping ID3v2 tag per user request.
        skiptag = true;
    }
    if (flags & UNKNOWN_FLAGS) != 0 || (major > 4) || (major < 2) {
        // ID3v2: Won't parse the ID3v2 tag with major version %u and flags 0x%xu - some extra code may be needed
        skiptag = true;
    }
    // Standard says that compressed tags should be ignored as there isn't an agreed
    // compressoion scheme.
    if major == 2 && flags & COMPRESS_FLAG != 0 {
        // ID3v2: ignoring compressed ID3v2.2 tag
        skiptag = true;
    }
    if length < 10 {
        // ID3v2: unrealistic small tag lengh %lu, skipping
        skiptag = true;
    }
    if !skiptag {
        storetag = true;
    }
    if storetag {
        /* Stores whole tag with footer and an additonal trailing zero. */
        let ret2 = store_id3v2(fr, first4bytes, &buf, length + footlen);
        if ret2 <= 0 {
            return ret2;
        }
    }
    if skiptag {
        // Note: skipped tag clearing possibly existing ID3v2 data
        reset_id3(fr); // Old data is invalid.
        if !storetag {
            let ret2 = rd_skip_bytes(fr, (length + footlen) as i64);
            if ret2 < 0 {
                ret = ret2 as i32;
            }
        }
    } else {
        null_id3_links(fr); // Could be invalidated on frame update, null for hygiene.
        let raw = fr.id3v2_raw.take().unwrap_or_default();
        let tagdata = &raw[10..];
        /* try to interpret that beast */
        'tagparse_cleanup: {
            if length > 0 {
                let mut extflags: u8 = 0;
                let mut tagpos: u64 = 0;
                /* bytes of frame title and of framesize value */
                let head_part: u64 = if major > 2 { 4 } else { 3 };
                let flag_part: u64 = if major > 2 { 2 } else { 0 };
                /* The amount of bytes that are unconditionally read for each frame: */
                /* ID, size, flags. */
                let framebegin = head_part + head_part + flag_part;
                if flags & EXTHEAD_FLAG != 0 {
                    if !bytes_to_long(tagdata, &mut tagpos) || tagpos >= length {
                        ret = 0;
                        // Bad (non-synchsafe/too large) tag offset from extended header:0x%02x%02x%02x%02x
                    } else if tagpos < 6 {
                        ret = 0;
                        // Extended header too small (%lu).
                    }
                    if major == 3 {
                        tagpos += 4; // The size itself is not included.
                        if tagpos >= length {
                            ret = 0;
                            // Too much extended v2.3 header.
                        }
                    } else if ret != 0 {
                        // v2.4 and at least got my 6 bytes of ext header
                        // Only v4 knows update frames, check for that.
                        // Need to step back. Header is 4 bytes length, one byte flag size,
                        // one byte flags. Flag size has to be 1!
                        if tagdata[4] == 1 && tagdata[5] & EXT_UPDATE_FLAG != 0 {
                            extflags |= EXT_UPDATE_FLAG;
                        }
                    }
                }
                if extflags & EXT_UPDATE_FLAG == 0 {
                    // Note: non-update tag replacing existing ID3v2 data
                    reset_id3(fr);
                }
                if ret > 0 {
                    let mut id = [0u8; 5];
                    let mut framesize: u64 = 0;
                    let mut fflags: u64; /* need 16 bits, actually */
                    id[4] = 0;
                    fr.id3v2.version = major;
                    /* Pos now advanced after ext head, now a frame has to follow. */
                    /* Note: tagpos <= length, which is 28 bit integer, so both */
                    /* far away from overflow for adding known small values. */
                    /* I want to read at least one full header now. */
                    while length >= tagpos + framebegin {
                        let mut pos = tagpos;
                        /* level 1,2,3 - 0 is info from lame/info tag! */
                        /* rva tags with ascending significance, then general frames */
                        let mut tt = FrameTypes::Unknown;
                        /* we may have entered the padding zone or any other strangeness: check if we have valid frame id characters */
                        for i in 0..head_part {
                            let c = tagdata[(tagpos + i) as usize];
                            if !((c > 47 && c < 58) || (c > 64 && c < 91)) {
                                /* This is no hard error... let's just hope that we got something meaningful already (ret==1 in that case). */
                                break 'tagparse_cleanup; /* Need to escape two loops here. */
                            }
                        }
                        if ret > 0 {
                            /* 4 or 3 bytes id */
                            // (strncpy(): the bytes up to a zero, which the check above excludes)
                            id[..head_part as usize].copy_from_slice(
                                &tagdata[pos as usize..(pos + head_part) as usize],
                            );
                            id[head_part as usize] = 0; /* terminate for 3 or 4 bytes */
                            pos += head_part;
                            tagpos += head_part;
                            /* size as 32 bits or 28 bits */
                            if fr.id3v2.version == 2 {
                                framesize = threebytes_to_long(&tagdata[pos as usize..]);
                            } else if !bytes_to_long(&tagdata[pos as usize..], &mut framesize) {
                                /* Just assume that up to now there was some good data. */
                                // ID3v2: non-syncsafe size of %s frame, skipping the remainder of tag
                                break;
                            }
                            tagpos += head_part;
                            pos += head_part;
                            if fr.id3v2.version > 2 {
                                fflags = ((tagdata[pos as usize] as u64) << 8)
                                    | (tagdata[pos as usize + 1] as u64);
                                pos += 2;
                                tagpos += 2;
                            } else {
                                fflags = 0;
                            }

                            if length - tagpos < framesize {
                                // Whoa! ID3v2 frame claims to be larger than the whole rest of the tag.
                                break;
                            }
                            tagpos += framesize; /* the important advancement in whole tag */
                            /* for sanity, after full parsing tagpos should be == pos */
                            /* v2.4: %0abc0000 %0h00kmnp */
                            /* v2.3: %abc00000 %ijk00000 */
                            /* v2.2: just zero */
                            let v3 = major == 3;
                            let bad_fflags: u64 = if v3 { 7967 } else { 36784 };
                            let group_fflag: u64 = if v3 { 32 } else { 64 };
                            let compr_fflag: u64 = if v3 { 128 } else { 8 };
                            let encr_fflag: u64 = if v3 { 64 } else { 4 };
                            let unsync_fflag: u64 = if v3 { 0 } else { 2 };
                            let datlen_fflag: u64 = if v3 { 0 } else { 1 };
                            if head_part < 4 && promote_framename(&mut id) != 0 {
                                continue;
                            }

                            /* shall not or want not handle these */
                            if fflags & (bad_fflags | compr_fflag | encr_fflag) != 0 {
                                // ID3v2: skipping invalid/unsupported frame
                                continue;
                            }

                            for (i, ft) in FRAME_TYPE.iter().enumerate() {
                                if id[..4] == ft[..] {
                                    tt = ALL_TYPES[i];
                                    break;
                                }
                            }

                            if id[0] == b'T' && tt != FrameTypes::Extra {
                                tt = FrameTypes::Text;
                            }

                            if tt != FrameTypes::Unknown {
                                let mut realsize = framesize as usize;
                                // (realdata: the frame data with the rest of its buffer, for the
                                // strlen() of the RVA2 identification)
                                let unsyncbuffer: Vec<u8>;
                                let mut realdata: &[u8] = &tagdata[pos as usize..];
                                if ((flags & UNSYNC_FLAG != 0) || (fflags & unsync_fflag != 0))
                                    && framesize > 0
                                {
                                    /* de-unsync: FF00 -> FF; real FF00 is simply represented as FF0000 ... */
                                    /* damn, that means I have to delete bytes from withing the data block... thus need temporal storage */
                                    /* standard mandates that de-unsync should always be safe if flag is set */
                                    let mut ub = Vec::new(); /* will need <= bytes, plus a safety zero */
                                    if ub.try_reserve_exact(framesize as usize + 1).is_err() {
                                        // ID3v2: unable to allocate working buffer for de-unsync
                                        continue;
                                    }
                                    /* now going byte per byte through the data... */
                                    let p0 = pos as usize;
                                    ub.push(tagdata[p0]);
                                    for ipos in p0 + 1..p0 + framesize as usize {
                                        if !((tagdata[ipos] == 0) && (tagdata[ipos - 1] == 0xff)) {
                                            ub.push(tagdata[ipos]);
                                        }
                                    }
                                    realsize = ub.len();
                                    /* Append a zero to keep strlen() safe. */
                                    ub.push(0);
                                    unsyncbuffer = ub;
                                    realdata = &unsyncbuffer;
                                }
                                // The spec says there is a group byte, without explicitly saying that it is
                                // the first thing following the header. I just assume so, because of the
                                // ordering of the flags.
                                if fflags & group_fflag != 0 {
                                    // Just skip group byte.
                                    if realsize != 0 {
                                        realsize -= 1;
                                        realdata = &realdata[1..];
                                    } else {
                                        // Grouped frame without group byte, even.
                                    }
                                }
                                if fflags & datlen_fflag != 0 {
                                    // Spec says the original (without compression or unsync) data length follows,
                                    // so it should match de-unsynced data now.
                                    if realsize >= 4 {
                                        let mut datlen: u64 = 0;
                                        if bytes_to_long(realdata, &mut datlen)
                                            && datlen == (realsize - 4) as u64
                                        {
                                            realsize -= 4;
                                            realdata = &realdata[4..];
                                        } else {
                                            // frame data length bad, skipping
                                            realsize = 0;
                                        }
                                    } else {
                                        realsize = 0;
                                        // frame truncated at frame data length, skipping
                                    }
                                }
                                pos = 0; /* now at the beginning again... */
                                /* Avoid reading over boundary, even if there is a */
                                /* zero byte of padding for safety. */
                                let fid: [u8; 4] = id[..4].try_into().unwrap();
                                if realsize != 0 {
                                    match tt {
                                        FrameTypes::Comment | FrameTypes::Uslt => process_comment(
                                            fr,
                                            tt,
                                            realdata,
                                            realsize,
                                            FrameTypes::Comment as i32 + 1,
                                            &fid,
                                        ),
                                        FrameTypes::Extra => {
                                            /* perhaps foobar2000's work */
                                            process_extra(
                                                fr,
                                                realdata,
                                                realsize,
                                                FrameTypes::Extra as i32 + 1,
                                                &fid,
                                            )
                                        }
                                        FrameTypes::Rva2 => {
                                            /* "the" RVA tag */
                                            /* starts with null-terminated identification */
                                            /* default: some individual value, mix mode */
                                            let mut rva_mode = 0usize;
                                            if c_strncaseeq(realdata, b"album", 5)
                                                || c_strncaseeq(realdata, b"audiophile", 10)
                                                || c_strncaseeq(realdata, b"user", 4)
                                            {
                                                rva_mode = 1;
                                            }
                                            if fr.rva.level[rva_mode] <= FrameTypes::Rva2 as i32 + 1
                                            {
                                                pos += c_str(realdata).len() as u64 + 1;
                                                // channel and two bytes for RVA value
                                                // pos possibly just past the safety zero, so one more than realsize
                                                let p = pos as usize;
                                                if p > realsize || realsize - p < 3 {
                                                    // bad RVA2 tag (truncated?)
                                                } else if realdata[p] == 1 {
                                                    let p = p + 1;
                                                    /* only handle master channel */
                                                    /* two bytes adjustment, one byte for bits representing peak - n bytes, eh bits, for peak */
                                                    /* 16 bit signed integer = dB * 512. Do not shift signed integers! Multiply instead.
                                                    Also no implementation-defined casting. Reinterpret the pointer to signed char, then do
                                                    proper casting. */
                                                    fr.rva.gain[rva_mode] =
                                                        ((realdata[p] as i8 as i16 as i32) * 256
                                                            + realdata[p + 1] as i16 as i32)
                                                            as f32
                                                            / 512.0;
                                                    /* heh, the peak value is represented by a number of bits - but in what manner? Skipping that part */
                                                    fr.rva.peak[rva_mode] = 0.0;
                                                    fr.rva.level[rva_mode] =
                                                        FrameTypes::Rva2 as i32 + 1;
                                                }
                                            }
                                        }
                                        /* non-rva metainfo, simply store... */
                                        FrameTypes::Text => {
                                            process_text(fr, realdata, realsize, &fid)
                                        }
                                        FrameTypes::Picture => {
                                            if fr.p.flags & MPG123_PICTURE != 0 {
                                                process_picture(fr, realdata, realsize);
                                            }
                                        }
                                        FrameTypes::Unknown => {
                                            // ID3v2: unknown frame type %i
                                        }
                                    }
                                }
                                let _ = pos;
                            }
                        } else {
                            break;
                        }
                    }
                } else {
                    // Note: faulty ID3v2 tag still clearing old data
                    reset_id3(fr);
                }
            } else {
                // No new data, but still there was a tag that invalidates old data.
                // Note: empty ID3v2 clearing old data
                reset_id3(fr);
            }
        }
        // tagparse_cleanup:
        /* Get rid of stored raw data that should not be kept. */
        if fr.p.flags & MPG123_STORE_RAW_ID3 == 0 {
            fr.id3v2_raw = None;
            fr.id3v2_size = 0;
        } else {
            fr.id3v2_raw = Some(raw);
        }
    }
    ret
}

fn convert_latin1(sb: &mut Mpg123String, s: &[u8], _noquiet: bool) {
    let l = s.len();
    let mut length = l;
    /* determine real length, a latin1 character can at most take 2  in UTF8 */
    for &c in s {
        if c >= 0x80 {
            length += 1;
        }
    }

    /* one extra zero byte for paranoia */
    if !mpg123_grow_string(sb, length + 1) {
        return;
    }

    let mut p = 0; /* Signedness doesn't matter but it shows I thought about the non-issue */
    for &c in s {
        if c < 0x80 {
            sb.p[p] = c;
            p += 1;
        } else {
            /* two-byte encoding */
            sb.p[p] = 0xc0 | (c >> 6);
            sb.p[p + 1] = 0x80 | (c & 0x3f);
            p += 2;
        }
    }

    sb.p[length] = 0;
    sb.fill = length + 1;
}

/*
    Check if we have a byte oder mark(s) there, return:
    -1: little endian
     0: no BOM
     1: big endian

    This modifies source and len to indicate the data _after_ the BOM(s).
    Note on nasty data: The last encountered BOM determines the endianness.
    I have seen data with multiple BOMS, namely from "the" id3v2 program.
    Not nice, but what should I do?
*/
fn check_bom(source: &mut &[u8]) -> i32 {
    let mut last_bom = 0;

    while source.len() >= 2 {
        let mut this_bom = 0;
        if source[0] == 0xff && source[1] == 0xfe {
            this_bom = -1;
        }
        if source[0] == 0xfe && source[1] == 0xff {
            this_bom = 1;
        }
        if this_bom == 0 {
            break;
        }
        /* Skip the detected BOM. */
        last_bom = this_bom;
        *source = &source[2..];
    }
    last_bom
}

fn fullpoint(f: u64, s: u64) -> u64 {
    ((f & 0x3ff) << 10) + (s & 0x3ff) + 0x10000
}
/* Remember: There's a limit at 0x1ffff. */
fn utf8len(x: u64) -> usize {
    if x < 0x80 {
        1
    } else if x < 0x800 {
        2
    } else if x < 0x10000 {
        3
    } else {
        4
    }
}

fn convert_utf16bom(sb: &mut Mpg123String, s: &[u8], _noquiet: bool) {
    let mut s = s;
    let mut length = 0; /* the resulting UTF-8 length */
    /* Determine real length... extreme case can be more than utf-16 length. */
    let mut high = 0;
    let mut low = 1;

    let bom_endian = check_bom(&mut s);
    let l = s.len();

    if bom_endian == -1 {
        /* little-endian */
        high = 1; /* The second byte is the high byte. */
        low = 0; /* The first byte is the low byte. */
    }

    let mut n = (l / 2) * 2; /* number bytes that make up full pairs */

    /* first: get length, check for errors -- stop at first one */
    let mut i = 0;
    while i < n {
        let mut point = ((s[i + high] as u64) << 8) + s[i + low] as u64;
        if (point & 0xfc00) == 0xd800 {
            /* lead surrogate */
            let second: u16 = if i + 3 < l {
                ((s[i + 2 + high] as u16) << 8) + s[i + 2 + low] as u16
            } else {
                0
            };
            if (second & 0xfc00) == 0xdc00 {
                /* good... */
                point = fullpoint(point, second as u64);
                length += utf8len(point); /* possibly 4 bytes */
                i += 2; /* We overstepped one word. */
            } else {
                /* if no valid pair, break here */
                // Invalid UTF16 surrogate pair at %li (0x%04lx).
                n = i; /* Forget the half pair, END! */
                break;
            }
        } else {
            length += utf8len(point); /* 1,2 or 3 bytes */
        }
        i += 2;
    }

    if !mpg123_grow_string(sb, length + 1) {
        return;
    }

    /* Now really convert, skip checks as these have been done just before. */
    let mut p = 0; /* Signedness doesn't matter but it shows I thought about the non-issue */
    let mut i = 0;
    while i < n {
        let mut codepoint = ((s[i + high] as u64) << 8) + s[i + low] as u64;
        if (codepoint & 0xfc00) == 0xd800 {
            /* lead surrogate */
            let second: u16 = ((s[i + 2 + high] as u16) << 8) + s[i + 2 + low] as u16;
            codepoint = fullpoint(codepoint, second as u64);
            i += 2; /* We overstepped one word. */
        }
        if codepoint < 0x80 {
            sb.p[p] = codepoint as u8;
            p += 1;
        } else if codepoint < 0x800 {
            sb.p[p] = (0xc0 | (codepoint >> 6)) as u8;
            sb.p[p + 1] = (0x80 | (codepoint & 0x3f)) as u8;
            p += 2;
        } else if codepoint < 0x10000 {
            sb.p[p] = (0xe0 | (codepoint >> 12)) as u8;
            sb.p[p + 1] = (0x80 | ((codepoint >> 6) & 0x3f)) as u8;
            sb.p[p + 2] = (0x80 | (codepoint & 0x3f)) as u8;
            p += 3;
        } else if codepoint < 0x200000 {
            sb.p[p] = (0xf0 | codepoint >> 18) as u8;
            sb.p[p + 1] = (0x80 | ((codepoint >> 12) & 0x3f)) as u8;
            sb.p[p + 2] = (0x80 | ((codepoint >> 6) & 0x3f)) as u8;
            sb.p[p + 3] = (0x80 | (codepoint & 0x3f)) as u8;
            p += 4;
        } /* ignore bigger ones (that are not possible here anyway) */
        i += 2;
    }
    sb.p[length] = 0;
    sb.fill = length + 1;
}

fn convert_utf8(sb: &mut Mpg123String, source: &[u8], _noquiet: bool) {
    let len = source.len();
    if mpg123_grow_string(sb, len + 1) {
        sb.p[..len].copy_from_slice(source);
        sb.p[len] = 0;
        sb.fill = len + 1;
    }
}
