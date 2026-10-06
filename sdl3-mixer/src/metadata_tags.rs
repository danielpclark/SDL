// Rust translation of src/SDL_mixer_metadata_tags.c from SDL_mixer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

// Functions to discard MP3 tags -
// written by O.Sezer <sezero@users.sourceforge.net>, put into public domain.

// Functions to parse some of MP3 tags -
// written by V.Novichkov <admin@wohlnet.ru>, put into public domain.

//! ID3v1, ID3v2, APE, Lyrics3 and MusicMatch tags (found and clamped off
//! the ends of any file), and Ogg comments (with their loop points).
//!
//! Strings are bytes here, as in C; a C string ends at its first NUL, which
//! [`cstr`] honors when a string becomes a property. Property values are
//! UTF-8 Rust strings: where upstream would store a string that isn't valid
//! UTF-8 (a broken "UTF-8" ID3v2 frame), the invalid bytes become U+FFFD.

use sdl3::io::IoWhence;
use sdl3::properties::Properties;
use sdl3::stdlib::iconv::iconv_string;
use sdl3::stdlib::string::{strcasecmp, strtod, strtoll};

use crate::internal::{IoClamp, OggLoop};
use crate::{
    PROP_METADATA_ALBUM_STRING, PROP_METADATA_ARTIST_STRING, PROP_METADATA_COPYRIGHT_STRING,
    PROP_METADATA_TITLE_STRING, PROP_METADATA_TOTAL_TRACKS_NUMBER, PROP_METADATA_TRACK_NUMBER,
    PROP_METADATA_YEAR_NUMBER,
};

const METADATA_TAGS_DEBUG_LOGGING: bool = false;
macro_rules! dbglog {
    ($($arg:tt)*) => {
        if METADATA_TAGS_DEBUG_LOGGING {
            sdl3::log!($($arg)*);
        }
    };
}

/// The bytes of a C string: up to the first NUL.
pub(crate) fn cstr(bytes: &[u8]) -> &[u8] {
    match bytes.iter().position(|&b| b == 0) {
        Some(n) => &bytes[..n],
        None => bytes,
    }
}

/// A C string as a property value (see the module docs).
pub(crate) fn cstring(bytes: &[u8]) -> String {
    String::from_utf8_lossy(cstr(bytes)).into_owned()
}

fn read_sint32le(data: &[u8]) -> i32 {
    let mut result = data[0] as u32;
    result |= (data[1] as u32) << 8;
    result |= (data[2] as u32) << 16;
    result |= (data[3] as u32) << 24;
    result as i32
}

fn read_sint24be(data: &[u8]) -> i32 {
    let mut result = data[2] as u32;
    result |= (data[1] as u32) << 8;
    result |= (data[0] as u32) << 16;
    result as i32
}

fn read_sint32be(data: &[u8]) -> i32 {
    let mut result = data[3] as u32;
    result |= (data[2] as u32) << 8;
    result |= (data[1] as u32) << 16;
    result |= (data[0] as u32) << 24;
    result as i32
}

const TAGS_INPUT_BUFFER_SIZE: usize = 128;

/********************************************************
 *                  ID3v1                               *
 ********************************************************/

const ID3V1_TAG_SIZE: usize = 128;
const ID3V1_SIZE_OF_FIELD: usize = 30;
const ID3V1_SIZE_OF_YEAR_FIELD: usize = 4;

const ID3V1_FIELD_TITLE: usize = 3;
const ID3V1_FIELD_ARTIST: usize = 33;
const ID3V1_FIELD_ALBUM: usize = 63;
const ID3V1_FIELD_YEAR: usize = 93;
const ID3V1_FIELD_COPYRIGHT: usize = 97;
const ID3V1_FIELD_TRACK: usize = 125;
#[allow(dead_code)]
const ID3V1_FIELD_GENRE: usize = 127;

fn is_id3v1(data: &[u8], length: usize) -> bool {
    // https://id3.org/ID3v1 :  3 bytes "TAG" identifier and 125 bytes tag data
    (length >= ID3V1_TAG_SIZE) && (&data[..3] == b"TAG")
}

// Parse ISO-8859-1 string and convert it into UTF-8
fn parse_id3v1_ansi_string(buffer: &[u8], mut src_len: usize) -> Option<Vec<u8>> {
    let mut src_buffer = buffer[..src_len].to_vec();
    src_buffer.push(0);

    // trim whitespace from end (some id3v1 tags pad out with space instead of nulls).
    if src_len > 0 {
        let mut i = src_len;
        loop {
            i -= 1;
            if src_buffer[i] == b' ' {
                src_buffer[i] = b'\0';
                src_len -= 1;
            } else {
                break;
            }
            if i == 0 {
                break;
            }
        }
    }

    iconv_string("UTF-8", "ISO-8859-1", &src_buffer[..src_len + 1]).ok()
}

fn id3v1_set_tag(props: &Properties, key: &str, buffer: &[u8], len: usize) {
    if !props.contains(key) {
        // in case there are multiple ID3v1 tags appended to a file, we'll take the last one, since we parse backwards from the end of file.
        if let Some(src_buf) = parse_id3v1_ansi_string(buffer, len) {
            if !cstr(&src_buf).is_empty() {
                let _ = props.set(key, cstring(&src_buf));
            }
        }
    }
}

// Parse content of ID3v1 tag
fn parse_id3v1(props: &Properties, buffer: &[u8]) {
    // ID3v1.1: if the second-to-last byte of the comment field is a zero (null terminator), treat the last byte as the track number (which should also be zero in ID3v1.0).
    let has_tracknum = (buffer[ID3V1_FIELD_TRACK] == 0) && (buffer[ID3V1_FIELD_TRACK + 1] != 0);

    id3v1_set_tag(
        props,
        "SDL_mixer.metadata.id3v1.title",
        &buffer[ID3V1_FIELD_TITLE..],
        ID3V1_SIZE_OF_FIELD,
    );
    id3v1_set_tag(
        props,
        "SDL_mixer.metadata.id3v1.artist",
        &buffer[ID3V1_FIELD_ARTIST..],
        ID3V1_SIZE_OF_FIELD,
    );
    id3v1_set_tag(
        props,
        "SDL_mixer.metadata.id3v1.album",
        &buffer[ID3V1_FIELD_ALBUM..],
        ID3V1_SIZE_OF_FIELD,
    );
    id3v1_set_tag(
        props,
        "SDL_mixer.metadata.id3v1.comment",
        &buffer[ID3V1_FIELD_COPYRIGHT..],
        ID3V1_SIZE_OF_FIELD - if has_tracknum { 2 } else { 0 },
    );
    id3v1_set_tag(
        props,
        "SDL_mixer.metadata.id3v1.year",
        &buffer[ID3V1_FIELD_YEAR..],
        ID3V1_SIZE_OF_YEAR_FIELD,
    );

    if has_tracknum {
        let s = format!("{}", buffer[ID3V1_FIELD_TRACK + 1] as u32);
        id3v1_set_tag(
            props,
            "SDL_mixer.metadata.id3v1.track",
            s.as_bytes(),
            s.len(),
        );
    }
}

/********************************************************
 *                       ID3v2                          *
 ********************************************************/

const ID3V2_BUFFER_SIZE: usize = 1024;

const ID3V2_HEADER_SIZE: usize = 10;

const ID3V2_FIELD_VERSION_MAJOR: usize = 3;
#[allow(dead_code)]
const ID3V2_FIELD_VERSION_MINOR: usize = 4;
const ID3V2_FIELD_HEAD_FLAGS: usize = 5;
const ID3V2_FIELD_TAG_LENGTH: usize = 6;
const ID3V2_FIELD_EXTRA_HEADER_LENGTH: usize = 10;

const ID3V2_FLAG_HAS_FOOTER: u8 = 0x10;
const ID3V2_FLAG_HAS_EXTRA_HEAD: u8 = 0x40;

const ID3V2_3_FRAME_HEADER_SIZE: usize = 10;
const ID3V2_2_FRAME_HEADER_SIZE: usize = 6;
const ID3V2_FIELD_FRAME_SIZE: usize = 4;
const ID3V2_FIELD_FRAME_SIZEV2: usize = 3;
#[allow(dead_code)]
const ID3V2_FIELD_FLAGS: usize = 8;

fn is_id3v2(data: &[u8], length: usize) -> bool {
    // ID3v2 header is 10 bytes:  https://id3.org/id3v2.4.0-structure
    // bytes 0-2: "ID3" identifier
    if (length < ID3V2_HEADER_SIZE) || (&data[..3] != b"ID3") {
        return false;
    }
    // bytes 3-4: version num (major,revision), each byte always less than 0xff.
    if data[3] == 0xff || data[4] == 0xff {
        return false;
    }
    // bytes 6-9 are the ID3v2 tag size: a 32 bit 'synchsafe' integer, i.e. the
    // highest bit 7 in each byte zeroed.  i.e.: 7 bit information in each byte ->
    // effectively a 28 bit value.
    if data[6] >= 0x80 || data[7] >= 0x80 || data[8] >= 0x80 || data[9] >= 0x80 {
        return false;
    }
    true
}

fn id3v2_synchsafe_decode(data: &[u8]) -> i32 {
    ((data[0] as i32) << 21) + ((data[1] as i32) << 14) + ((data[2] as i32) << 7) + data[3] as i32
}

fn get_id3v2_len(data: &[u8], length: usize) -> i32 {
    // size is a 'synchsafe' integer (see above)
    let mut size = id3v2_synchsafe_decode(&data[6..]) as usize;
    size += ID3V2_HEADER_SIZE; // header size
                               // ID3v2 header[5] is flags (bits 4-7 only, 0-3 are zero).
                               // bit 4 set: footer is present (a copy of the header but
                               // with "3DI" as ident.)
    if data[5] & 0x10 != 0 {
        size += ID3V2_HEADER_SIZE; // footer size
    }
    // optional padding (always zeroes)
    while (size < length) && (data[size] == 0) {
        size += 1;
    }
    size as i32
}

// Decode a string in the frame according to an encoding marker
fn id3v2_decode_string(string: &[u8], size: usize) -> Option<Vec<u8>> {
    if size == 0 {
        dbglog!("id3v2_decode_string: Bad string size: a string should have at least 1 byte");
        return None;
    } else if size < 2 {
        return None;
    }

    let mut str_buffer = None;

    if string[0] == b'\x01' {
        // UTF-16 string with a BOM
        if size <= 5 {
            if size < 5 {
                dbglog!(
                    "id3v2_decode_string: Bad BOM-UTF16 string size: {} < 5",
                    size as u32
                );
            }
            return None;
        }

        let copy_size = size - 3 + 2; // exclude 3 bytes of encoding hint, append 2 bytes for a NULL termination
        let mut src_buffer = string[3..3 + copy_size - 2].to_vec();
        src_buffer.extend_from_slice(&[0, 0]);

        if &string[..3] == b"\x01\xFE\xFF" {
            // UTF-16BE*/
            str_buffer = iconv_string("UTF-8", "UCS-2BE", &src_buffer).ok();
        } else if &string[..3] == b"\x01\xFF\xFE" {
            // UTF-16LE*/
            str_buffer = iconv_string("UTF-8", "UCS-2LE", &src_buffer).ok();
        }
    } else if string[0] == b'\x02' {
        // UTF-16BEstring without a BOM
        if size <= 3 {
            if size < 3 {
                dbglog!(
                    "id3v2_decode_string: Bad UTF16BE string size: {} < 3",
                    size as u32
                );
            }
            return None; // Blank string*/
        }

        let copy_size = size - 1 + 2; // exclude 1 byte of encoding hint, append 2 bytes for a NULL termination
        let mut src_buffer = string[1..1 + copy_size - 2].to_vec();
        src_buffer.extend_from_slice(&[0, 0]);

        str_buffer = iconv_string("UTF-8", "UCS-2BE", &src_buffer).ok();
    } else if string[0] == b'\x03' {
        // UTF-8 string
        if size <= 2 {
            return None; // Blank string*/
        }
        // SDL_strlcpy(str_buffer, string + 1, size)
        let src = cstr(&string[1..size]);
        str_buffer = Some(src.to_vec());
    } else if string[0] == b'\x00' {
        // Latin-1 string
        if size <= 2 {
            return None; // Blank string*/
        }
        str_buffer = parse_id3v1_ansi_string(&string[1..], size - 1);
    }

    str_buffer
}

// Write a tag string into internal meta-tags storage
fn set_id3v2_string_prop(props: &Properties, key: &str, string: &[u8], size: usize) {
    if let Some(str_buffer) = id3v2_decode_string(string, size) {
        let _ = props.set(key, cstring(&str_buffer));
    }
}

/// `snprintf("SDL_mixer.metadata.id3v2.%c%c%c%c", ...)`: the key ends at a
/// NUL character, like the C string does.
fn id3v2_generic_key(key: &[u8]) -> String {
    let mut k = b"SDL_mixer.metadata.id3v2.".to_vec();
    k.extend_from_slice(key);
    cstring(&k)
}

// Identify a meta-key and decode the string (Note: input buffer should have at least 4 characters!)
fn handle_id3v2_string(props: &Properties, key: &[u8; 4], string: &[u8], size: usize) {
    // put most text things in props in a generic "this is what the id3v2 key was" so apps can handle things we didn't pick out
    //  specifically, or new tags in the hypothetical future.
    if &key[1..4] != b"XXX" {
        // !!! FIXME: we (currently) skip ?XXX because they aren't simple key/value pairs.
        let generic_key = id3v2_generic_key(key);
        if key[0] == b'T' {
            // all text keys start with 'T'
            set_id3v2_string_prop(props, &generic_key, string, size);
        } else if key[0] == b'W' {
            // all URLs start with W.
            if let Some(decoded) = parse_id3v1_ansi_string(string, size) {
                let _ = props.set(&generic_key, cstring(&decoded));
            }
        }
    }

    // TODO: Extract "Copyright message" from TXXX value: a KEY=VALUE string divided by a zero byte:*/
    //  else if (SDL_memcmp(key, "TXXX", 4) == 0) {
    //      set_id3v2_string_prop(props, MIX_META_COPYRIGHT, string, size);
    //  }
}

// Identify a meta-key and decode the string (Note: input buffer should have at least 4 characters!)
fn handle_id3v2x2_string(props: &Properties, key: &[u8; 3], string: &[u8], size: usize) {
    // put most text things in props in a generic "this is what the id3v2.2 key was" so apps can handle things we didn't pick out
    //  specifically, or new tags in the hypothetical future.
    if &key[1..3] != b"XX" {
        // !!! FIXME: we (currently) skip ?XX because they aren't simple key/value pairs.
        let generic_key = id3v2_generic_key(key);
        if key[0] == b'T' {
            // all text keys start with 'T'
            set_id3v2_string_prop(props, &generic_key, string, size);
        } else if key[0] == b'W' {
            // all URLs start with W.
            if let Some(decoded) = parse_id3v1_ansi_string(string, size) {
                let _ = props.set(&generic_key, cstring(&decoded));
            }
        }
    }
}

// Parse a frame in ID3v2.2 format
fn id3v22_parse_frame(props: &Properties, io: &mut IoClamp<'_>, buffer: &mut [u8]) -> usize {
    let frame_begin = io.tell_io();
    let mut read_size = io.read_io(&mut buffer[..ID3V2_2_FRAME_HEADER_SIZE]);

    if read_size < ID3V2_2_FRAME_HEADER_SIZE {
        dbglog!("id3v22_parse_frame (1): Unexpected end of the file while frame header reading (had to read {} bytes, {} bytes wanted)", read_size, ID3V2_2_FRAME_HEADER_SIZE);
        io.seek_io(frame_begin, IoWhence::Set);
        return 0; // Buffer size that left is too small
    }

    if &buffer[..3] == b"\0\0\0" {
        io.seek_io(frame_begin, IoWhence::Set);
        return 0;
    }

    let key = [buffer[0], buffer[1], buffer[2]]; // Tag title (key)

    let size = read_sint24be(&buffer[ID3V2_FIELD_FRAME_SIZEV2..]) as usize;
    let expected_size = size.min(ID3V2_BUFFER_SIZE);
    read_size = io.read_io(&mut buffer[..expected_size]);
    if read_size < expected_size {
        dbglog!("id3v22_parse_frame (2): Unexpected end of the file while frame data reading (had to read {} bytes, {} bytes wanted)", read_size, expected_size);
        io.seek_io(frame_begin, IoWhence::Set);
        return 0; // Can't read frame data, possibly, a file size was reached
    }
    io.seek_io(
        frame_begin + size as i64 + ID3V2_2_FRAME_HEADER_SIZE as i64,
        IoWhence::Set,
    );

    handle_id3v2x2_string(props, &key, buffer, read_size);

    size + ID3V2_2_FRAME_HEADER_SIZE // data size + size of the header
}

// Parse a frame in ID3v2.3 and ID3v2.4 formats
fn id3v2x_parse_frame(
    props: &Properties,
    io: &mut IoClamp<'_>,
    buffer: &mut [u8],
    version: u8,
) -> usize {
    let frame_begin = io.tell_io();

    let mut read_size = io.read_io(&mut buffer[..ID3V2_3_FRAME_HEADER_SIZE]);

    if read_size < ID3V2_3_FRAME_HEADER_SIZE {
        dbglog!("id3v2x_parse_frame (1): Unexpected end of the file while frame header reading (had to read {} bytes, {} bytes wanted)", read_size, ID3V2_3_FRAME_HEADER_SIZE);
        io.seek_io(frame_begin, IoWhence::Set);
        return 0; // Can't read frame header, possibly, a file size was reached
    }

    if &buffer[..4] == b"\0\0\0\0" {
        io.seek_io(frame_begin, IoWhence::Set);
        return 0;
    }

    let key = [buffer[0], buffer[1], buffer[2], buffer[3]]; // Tag title (key)

    // (size_t is 64 bits here, as on the platforms upstream is tested on.)
    let size: u64 = if version == 4 {
        id3v2_synchsafe_decode(&buffer[ID3V2_FIELD_FRAME_SIZE..]) as i64 as u64
    } else {
        read_sint32be(&buffer[ID3V2_FIELD_FRAME_SIZE..]) as i64 as u64
    };

    let expected_size = size.min(ID3V2_BUFFER_SIZE as u64) as usize;
    read_size = io.read_io(&mut buffer[..expected_size]);
    if read_size < expected_size {
        dbglog!("id3v2x_parse_frame (2): Unexpected end of the file while frame data reading (had to read {} bytes, {} bytes wanted)", read_size, expected_size);
        io.seek_io(frame_begin, IoWhence::Set);
        return 0; // Can't read frame data, possibly, a file size was reached
    }
    io.seek_io(
        frame_begin
            .wrapping_add(size as i64)
            .wrapping_add(ID3V2_3_FRAME_HEADER_SIZE as i64),
        IoWhence::Set,
    );

    buffer[read_size] = b'\0'; // make sure it's definitely null-terminated.
    handle_id3v2_string(props, &key, buffer, read_size);

    size.wrapping_add(ID3V2_3_FRAME_HEADER_SIZE as u64) as usize // data size + size of the header
}

// Parse content of ID3v2. This expects the stream to be seeked to the start of the potential header.
fn parse_id3v2(props: &Properties, io: &mut IoClamp<'_>) -> bool {
    let mut buffer = [0u8; ID3V2_BUFFER_SIZE + 1];

    let mut total_length: i64 = 0;

    let read_size = io.read_io(&mut buffer[..ID3V2_HEADER_SIZE]); // Retrieve the header
    if read_size < ID3V2_HEADER_SIZE {
        dbglog!("parse_id3v2: fail to read a header ({} < 10)", read_size);
        return false; // Unsupported version of the tag
    }

    total_length += ID3V2_HEADER_SIZE as i64;

    let version_major = buffer[ID3V2_FIELD_VERSION_MAJOR]; // Major version
                                                           // version_minor = buffer[ID3v2_VERSION_MINOR]; // Minor version, UNUSED
    let flags = buffer[ID3V2_FIELD_HEAD_FLAGS]; // Flags
    let mut tag_len = id3v2_synchsafe_decode(&buffer[ID3V2_FIELD_TAG_LENGTH..]) as i64; // Length of a tag

    if version_major != 2 && version_major != 3 && version_major != 4 {
        dbglog!("parse_id3v2: Unsupported version {}", version_major);
        return false; // Unsupported version of the tag
    }

    let mut tag_extended_len: i64 = 0;
    if (version_major > 2) && ((flags & ID3V2_FLAG_HAS_EXTRA_HEAD) == ID3V2_FLAG_HAS_EXTRA_HEAD) {
        if io.read_io(
            &mut buffer[ID3V2_FIELD_EXTRA_HEADER_LENGTH..ID3V2_FIELD_EXTRA_HEADER_LENGTH + 4],
        ) != 4
        {
            return false;
        } else if io.seek_io(-4, IoWhence::Cur) < 0 {
            return false;
        }
        tag_extended_len =
            id3v2_synchsafe_decode(&buffer[ID3V2_FIELD_EXTRA_HEADER_LENGTH..]) as i64;
        // Length of an extended header
    }

    if tag_extended_len != 0 {
        tag_len -= tag_extended_len; // Subtract the size of extended header
        if io.seek_io(tag_extended_len, IoWhence::Cur) < 0 {
            // Skip extended header and it's size value
            return false;
        }
    }

    total_length += tag_len;

    if flags & ID3V2_FLAG_HAS_FOOTER != 0 {
        total_length += ID3V2_HEADER_SIZE as i64; // footer size
    }

    let mut pos = io.tell_io();

    if (pos + tag_len) > io.clamp_size() {
        dbglog!("parse_id3v2: Tag size bigger than actual file size");
        return false; // Tag size is bigger than actual buffer data
    }

    while (pos >= 0) && (pos < total_length) {
        let frame_length = if version_major == 2 {
            id3v22_parse_frame(props, io, &mut buffer)
        } else {
            id3v2x_parse_frame(props, io, &mut buffer, version_major)
        };
        if frame_length == 0 {
            break;
        }
        pos = io.tell_io();
    }

    true
}

/********************************************************
 *                  APE v1 and v2                       *
 ********************************************************/

const APE_BUFFER_SIZE: usize = 256;

const APE_V1: u32 = 1000;
const APE_V2: u32 = 2000;
const APE_HEADER_SIZE: usize = 32;

const APE_HEAD_FIELD_VERSION: usize = 8;
const APE_HEAD_FIELD_TAGSIZE: usize = 12;
const APE_HEAD_FIELD_ITEMS_COUNT: usize = 16;
const APE_HEAD_FIELD_FLAGS: usize = 20;
const APE_HEAD_FIELD_RESERVED: usize = 24;

const APE_FRAME_TAG_KEY: usize = 4;

fn is_apetag(data: &[u8], length: usize) -> bool {
    // https://wiki.hydrogenaud.io/index.php?title=APEv2_specification
    // Header/footer is 32 bytes: bytes 0-7 ident, bytes 8-11 version,
    // bytes 12-17 size. bytes 24-31 are reserved: must be all zeroes.

    if (length < 32) || (&data[..8] != b"APETAGEX") {
        return false;
    }

    let v = read_sint32le(&data[8..]) as u32; // version
    if v != APE_V2 && v != APE_V1 {
        return false;
    }
    // reserved bits :
    if data[24..28] != [0; 4] || data[28..32] != [0; 4] {
        return false;
    }
    true
}

fn get_ape_len(data: &[u8], version: &mut u32) -> i64 {
    let mut size = read_sint32le(&data[APE_HEAD_FIELD_TAGSIZE..]) as i64;
    *version = read_sint32le(&data[APE_HEAD_FIELD_VERSION..]) as u32;
    let flags = read_sint32le(&data[APE_HEAD_FIELD_FLAGS..]) as u32;
    if (*version == APE_V2) && (flags & (1u32 << 31)) != 0 {
        size += APE_HEADER_SIZE as i64; // header present.
    }
    size
}

/// Translation of `ape_find_value()`: the offset (in `data`) of the value
/// that follows the NUL-terminated key at `key`, if the key ends in the
/// buffer.
fn ape_find_value(data: &[u8], mut key: usize) -> Option<usize> {
    let end = key + APE_BUFFER_SIZE - 4;
    while data[key] != 0 && (key != end) {
        key += 1;
    }

    if key >= end {
        None
    } else {
        Some(key + 1)
    }
}

/// Translation of `ape_handle_tag()`. `data` is the read buffer, with room
/// past its `APE_BUFFER_SIZE + 1` bytes for the terminator upstream writes
/// up to 3 bytes beyond them (Note (upstream): a value that just fits
/// overruns the C buffer).
fn ape_handle_tag(props: &Properties, data: &mut [u8], valsize: usize) -> u32 {
    /* https://wiki.hydrogenaud.io/index.php?title=APE_Tag_Item
     * Tag entry has unclear size because of no size value for a key field
     * However, we only know next sizes:
     * - 4 bytes is a [length] of value field
     * - 4 bytes of value-specific flags
     * - unknown length of a key field. To detect its size
     *   it's need to find a zero byte looking at begin of the key field
     * - 1 byte of a null-terminator
     * - [length] bytes a value content
     */
    let key = APE_FRAME_TAG_KEY;
    let Some(value) = ape_find_value(data, key) else {
        return 0;
    };

    let key_len = (value - key) as u32;

    if valsize > (APE_BUFFER_SIZE - key_len as usize) {
        // maybe it's a list? convert embedded null chars to newlines. Note this will mess up binary data, but the APE spec doesn't currently list any binary keys.
        for b in &mut data[..APE_BUFFER_SIZE] {
            if *b == b'\0' {
                *b = b'\n';
            }
        }
        data[APE_BUFFER_SIZE] = b'\0'; // null-terminate the data.
    } else {
        // maybe it's a list? convert embedded null chars to newlines. Note this will mess up binary data, but the APE spec doesn't currently list any binary keys.
        for b in &mut data[value..value + valsize] {
            if *b == b'\0' {
                *b = b'\n';
            }
        }
        data[value + valsize] = b'\0'; // null-terminate the data.
    }

    if (key_len as usize) < 256 {
        let apekey: Vec<u8> = data[key..key + key_len as usize]
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let mut generic_key = b"SDL_mixer.metadata.ape.".to_vec();
        generic_key.extend_from_slice(cstr(&apekey));
        // (snprintf into 256 bytes: the key fits unless it would be truncated)
        if generic_key.len() < 256 {
            let generic_key = cstring(&generic_key);
            if !props.contains(&generic_key) {
                // in case there are multiple ID3v1 tags appended to a file, we'll take the last one, since we parse backwards from the end of file.
                let _ = props.set(&generic_key, cstring(&data[value..]));
            }
        }
    }

    4u32.wrapping_add(valsize as u32).wrapping_add(key_len)
}

// Parse content of APE tag
fn parse_ape(props: &Properties, io: &mut IoClamp<'_>, ape_head_pos: i64, version: u32) -> bool {
    let mut buffer = [0u8; APE_BUFFER_SIZE + 1 + 3];

    if io.seek_io(ape_head_pos, IoWhence::Set) != ape_head_pos {
        return false;
    }

    let read_size = io.read_io(&mut buffer[..APE_HEADER_SIZE]); // Retrieve the header
    if read_size < APE_HEADER_SIZE {
        io.seek_io(ape_head_pos, IoWhence::Set);
        return false;
    }

    let v = read_sint32le(&buffer[APE_HEAD_FIELD_VERSION..]) as u32; // version
    if v != APE_V2 && v != APE_V1 {
        return false;
    }

    let tag_size = read_sint32le(&buffer[APE_HEAD_FIELD_TAGSIZE..]) as u32; // tag size

    if version == APE_V1 {
        // If version 1, we are at footer
        let back = tag_size.wrapping_sub(APE_HEADER_SIZE as u32) as i64;
        if ape_head_pos - back < 0 {
            io.seek_io(ape_head_pos, IoWhence::Set);
            return false;
        }
        io.seek_io(ape_head_pos - back, IoWhence::Set);
    }

    let tag_items_count = read_sint32le(&buffer[APE_HEAD_FIELD_ITEMS_COUNT..]) as u32; // count tag items

    //flags = (Uint32)read_sint32be(buffer + APE_HEAD_FIELD_FLAGS); // global flags, unused
    let _ = APE_HEAD_FIELD_FLAGS;

    // reserved bits :
    if buffer[APE_HEAD_FIELD_RESERVED..APE_HEAD_FIELD_RESERVED + 8] != [0; 8] {
        return false;
    }

    for _ in 0..tag_items_count {
        let cur_tag = io.tell_io();
        if cur_tag < 0 {
            break;
        }
        let read_size = io.read_io(&mut buffer[..4]); // Retrieve the size
        if read_size < 4 {
            io.seek_io(ape_head_pos, IoWhence::Set);
            return false;
        }

        let v = read_sint32le(&buffer) as u32; // size of the tag's value field
                                               // (we still need to find key size by a null termination)

        // Retrieve the tag's data with an aproximal size as we can
        let want = (v.wrapping_add(40) as usize).min(APE_BUFFER_SIZE);
        let read_size = io.read_io(&mut buffer[..want]);
        buffer[read_size] = b'\0';

        let tag_item_size = ape_handle_tag(props, &mut buffer, v as usize);
        if tag_item_size == 0 {
            break;
        }
        if io.seek_io(cur_tag + tag_item_size as i64 + 4, IoWhence::Set) < 0 {
            return false;
        }
    }

    io.seek_io(ape_head_pos, IoWhence::Set) == ape_head_pos
}

/********************************************************
 *                   Lyrics3 skip                       *
 ********************************************************/

/* Header    : "LYRICSBEGIN"   -- 11 bytes
 * Size field: (decimal) (v2 only) 6 bytes
 * End marker: "LYRICS200" (v2) -  9 bytes
 * End marker: "LYRICSEND" (v1) -  9 bytes
 *
 * The maximum length of Lyrics3v1 is 5100 bytes.
 */

const LYRICS3V1_SEARCH_BUFFER: usize = 5120; // 5100 + 20 of tag begin and end keywords

const LYRICS3V1_HEAD_SIZE: usize = 11;
const LYRICS3V1_TAIL_SIZE: usize = 9;
const LYRICS3V2_TAG_SIZE_VALUE: usize = 6;
const LYRICS3_FOOTER_SIZE: usize = 15;

fn is_lyrics3tag(data: &[u8], length: usize) -> i32 {
    // https://id3.org/Lyrics3
    // https://id3.org/Lyrics3v2
    if length < LYRICS3_FOOTER_SIZE {
        return 0;
    }
    if &data[LYRICS3V2_TAG_SIZE_VALUE..LYRICS3V2_TAG_SIZE_VALUE + 9] == b"LYRICS200" {
        return 2; // v2
    }
    if &data[LYRICS3V2_TAG_SIZE_VALUE..LYRICS3V2_TAG_SIZE_VALUE + 9] == b"LYRICSEND" {
        return 1; // v1
    }
    0
}

fn get_lyrics3v1_len(io: &mut IoClamp<'_>) -> i64 {
    // needs manual search:  https://id3.org/Lyrics3
    let flen = io.clamp_size();
    if flen < 20 {
        return -1;
    }
    let mut len = flen.min(LYRICS3V1_SEARCH_BUFFER as i64);
    if io.seek_io(-len, IoWhence::End) < 0 {
        return -1;
    }

    let mut buf = [0u8; LYRICS3V1_SEARCH_BUFFER + 1];
    len -= LYRICS3V1_TAIL_SIZE as i64; // exclude footer
    let readamount = len as usize;
    if io.read_io(&mut buf[..readamount]) != readamount {
        return -1;
    }

    // strstr() won't work here.
    let mut i = len - LYRICS3V1_HEAD_SIZE as i64;
    let mut p = 0usize;
    while i >= 0 {
        if &buf[p..p + LYRICS3V1_HEAD_SIZE] == b"LYRICSBEGIN" {
            break;
        }
        i -= 1;
        p += 1;
    }
    if i < 0 {
        return -1;
    }
    len - p as i64 + LYRICS3V1_TAIL_SIZE as i64 // footer
}

fn get_lyrics3v2_len(data: &[u8], length: usize) -> i64 {
    // 6 bytes before the end marker is size in decimal format -
    // does not include the 9 bytes end marker and size field.
    if length != LYRICS3V2_TAG_SIZE_VALUE {
        0
    } else {
        sdl3::stdlib::string::strtol(cstr(data), 10).0 + LYRICS3_FOOTER_SIZE as i64
    }
}

fn verify_lyrics3v2(data: &[u8], length: usize) -> bool {
    (length >= LYRICS3V1_HEAD_SIZE) && (&data[..LYRICS3V1_HEAD_SIZE] == b"LYRICSBEGIN")
}

/********************************************************
 *                    MusicMatch                        *
 ********************************************************/

const MUSICMATCH_HEADER_SIZE: usize = 256;
const MUSICMATCH_VERSION_INFO_SIZE: usize = 256;
const MUSICMATCH_FOOTER_SIZE: usize = 48;
const MUSICMATCH_OFFSETS_SIZE: usize = 20;

// (MMTAG_PARANOID is defined upstream; its checks are always on here.)

fn is_musicmatch(data: &[u8], length: i64) -> bool {
    /* From docs/musicmatch.txt in id3lib: https://sourceforge.net/projects/id3lib/
       Overall tag structure:

       +-----------------------------+
       |           Header            |
       |    (256 bytes, OPTIONAL)    |
       +-----------------------------+
       |  Image extension (4 bytes)  |
       +-----------------------------+
       |        Image binary         |
       |  (var. length >= 4 bytes)   |
       +-----------------------------+
       |      Unused (4 bytes)       |
       +-----------------------------+
       |  Version info (256 bytes)   |
       +-----------------------------+
       |       Audio meta-data       |
       | (var. length >= 7868 bytes) |
       +-----------------------------+
       |   Data offsets (20 bytes)   |
       +-----------------------------+
       |      Footer (48 bytes)      |
       +-----------------------------+
    */
    if length < MUSICMATCH_FOOTER_SIZE as i64 {
        return false;
    }
    // sig: 19 bytes company name + 13 bytes space
    else if &data[..32] != b"Brava Software Inc.             " {
        return false;
    }
    // 4 bytes version: x.xx
    else if !data[32].is_ascii_digit()
        || data[33] != b'.'
        || !data[34].is_ascii_digit()
        || !data[35].is_ascii_digit()
    {
        return false;
    }
    // [36..47]: 12 bytes trailing space
    for &c in &data[36..MUSICMATCH_FOOTER_SIZE] {
        if c != b' ' {
            return false;
        }
    }
    true
}

fn get_musicmatch_len(io: &mut IoClamp<'_>) -> i64 {
    static METASIZES: [i32; 4] = [7868, 7936, 8004, 8132];
    static SYNCSTR: [u8; 10] = [b'1', b'8', b'2', b'7', b'3', b'6', b'4', b'5', 0, 0];
    let mut buf = [0u8; 256];
    let mut len: i64 = 0;

    if io.seek_io(-68, IoWhence::End) < 0 {
        return -1;
    } else if io.read_io(&mut buf[..20]) != 20 {
        return -1;
    }

    let imgext_ofs = i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let version_ofs = i32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    if version_ofs <= imgext_ofs {
        return -1;
    } else if (version_ofs <= 0) || (imgext_ofs <= 0) {
        return -1;
    }

    /* Try finding the version info section:
     * Because metadata section comes after it, and because metadata section
     * has different sizes across versions (format ver. <= 3.00: always 7868
     * bytes), we can _not_ directly calculate using deltas from the offsets
     * section. */
    let mut i = 0;
    while i < 4 {
        // 48: footer, 20: offsets, 256: version info
        len = METASIZES[i] as i64
            + MUSICMATCH_FOOTER_SIZE as i64
            + MUSICMATCH_OFFSETS_SIZE as i64
            + MUSICMATCH_VERSION_INFO_SIZE as i64;
        if io.seek_io(-len, IoWhence::End) < 0 {
            return -1;
        } else if io.read_io(&mut buf[..MUSICMATCH_VERSION_INFO_SIZE])
            != MUSICMATCH_VERSION_INFO_SIZE
        {
            return -1;
        }
        // [0..9]: sync string, [30..255]: 0x20
        if buf[30..MUSICMATCH_VERSION_INFO_SIZE]
            .iter()
            .any(|&c| c != b' ')
        {
            i += 1;
            continue;
        }
        if buf[..10] == SYNCSTR {
            break;
        }
        i += 1;
    }
    if i == 4 {
        return -1; // no luck.
    }

    // unused section: (4 bytes of 0x00)
    let mut j4 = [0u8; 4];
    if io.seek_io(-(len + 4), IoWhence::End) < 0 {
        return -1;
    } else if io.read_io(&mut j4) != 4 {
        return -1;
    } else if j4 != [0; 4] {
        return -1;
    }

    len += (version_ofs - imgext_ofs) as i64;
    if io.seek_io(-len, IoWhence::End) < 0 {
        return -1;
    } else if io.read_io(&mut buf[..8]) != 8 {
        return -1;
    }
    let j = i32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if j < 0 {
        return -1;
    }
    // verify image size:
    // without this, we may land at a wrong place.
    if j.wrapping_add(12) != version_ofs - imgext_ofs {
        return -1;
    }
    // try finding the optional header
    if io.seek_io(-(len + MUSICMATCH_HEADER_SIZE as i64), IoWhence::End) < 0 {
        return -1;
    } else if io.read_io(&mut buf[..MUSICMATCH_HEADER_SIZE]) != MUSICMATCH_HEADER_SIZE {
        return -1;
    }
    // [0..9]: sync string, [30..255]: 0x20
    if buf[..10] != SYNCSTR {
        return len;
    }
    if buf[30..MUSICMATCH_HEADER_SIZE].iter().any(|&c| c != b' ') {
        return len;
    }
    len + MUSICMATCH_HEADER_SIZE as i64 // header is present.
}

const TAG_FOUND: i32 = 1;
const TAG_INVALID: i32 = -1;
const TAG_NOT_FOUND: i32 = 0;

fn probe_id3v1(props: &Properties, io: &mut IoClamp<'_>, buf: &mut [u8], atend: bool) -> i32 {
    if io.seek_io(-(ID3V1_TAG_SIZE as i64), IoWhence::End) < 0 {
        return TAG_INVALID;
    } else if io.read_io(&mut buf[..ID3V1_TAG_SIZE]) != ID3V1_TAG_SIZE {
        return TAG_INVALID;
    } else if is_id3v1(buf, ID3V1_TAG_SIZE) {
        if !atend {
            // possible false positive?
            if is_musicmatch(&buf[128 - 48..], 48)
                || is_apetag(&buf[128 - 32..], 32)
                || is_lyrics3tag(&buf[128 - 15..], 15) != 0
            {
                return TAG_NOT_FOUND;
            }
        }
        parse_id3v1(props, buf);
        io.length -= ID3V1_TAG_SIZE as i64;
        return TAG_FOUND;
        // FIXME: handle possible double-ID3v1 tags??
    }

    TAG_NOT_FOUND
}

fn probe_mmtag(_props: &Properties, io: &mut IoClamp<'_>, buf: &mut [u8]) -> i32 {
    // !!! FIXME: Implement reading tag contents.
    if io.seek_io(-(MUSICMATCH_FOOTER_SIZE as i64), IoWhence::End) < 0 {
        return TAG_INVALID;
    } else if io.read_io(&mut buf[..MUSICMATCH_FOOTER_SIZE]) != MUSICMATCH_FOOTER_SIZE {
        return TAG_INVALID;
    } else if is_musicmatch(buf, MUSICMATCH_FOOTER_SIZE as i64) {
        let len = get_musicmatch_len(io);
        if len < 0 {
            return TAG_INVALID;
        }
        io.length -= len;
        return TAG_FOUND;
    }
    TAG_NOT_FOUND
}

fn probe_apetag(props: &Properties, io: &mut IoClamp<'_>, buf: &mut [u8]) -> i32 {
    // APE tag may be at the end: read the footer
    if io.seek_io(-(APE_HEADER_SIZE as i64), IoWhence::End) < 0 {
        return TAG_INVALID;
    } else if io.read_io(&mut buf[..APE_HEADER_SIZE]) != APE_HEADER_SIZE {
        return TAG_INVALID;
    }

    // APE tag may be at end or before ID3v1 tag
    let mut retval = TAG_NOT_FOUND;
    if is_apetag(buf, APE_HEADER_SIZE) {
        let mut v = 0u32;
        let len = get_ape_len(buf, &mut v);
        if v == APE_V2 {
            // verify header :
            if io.seek_io(-len, IoWhence::End) < 0 {
                return TAG_INVALID;
            } else if io.read_io(&mut buf[..APE_HEADER_SIZE]) != APE_HEADER_SIZE {
                return TAG_INVALID;
            } else if !is_apetag(buf, APE_HEADER_SIZE) {
                retval = TAG_NOT_FOUND;
            } else {
                let head = io.tell_io() - APE_HEADER_SIZE as i64;
                if parse_ape(props, io, head, APE_V2) {
                    retval = TAG_FOUND;
                }
            }
        } else if io.seek_io(-(APE_HEADER_SIZE as i64), IoWhence::End) < 0 {
            return TAG_INVALID;
        } else {
            let head = io.tell_io();
            if parse_ape(props, io, head, APE_V1) {
                retval = TAG_FOUND;
            }
        }
        io.length -= len;
    }

    retval
}

fn probe_lyrics3(io: &mut IoClamp<'_>, buf: &mut [u8]) -> i32 {
    if io.seek_io(-(LYRICS3_FOOTER_SIZE as i64), IoWhence::End) < 0 {
        return TAG_INVALID;
    } else if io.read_io(&mut buf[..LYRICS3_FOOTER_SIZE]) != LYRICS3_FOOTER_SIZE {
        return TAG_INVALID;
    }

    let ver = is_lyrics3tag(buf, LYRICS3_FOOTER_SIZE);
    if ver == 2 {
        let len = get_lyrics3v2_len(&buf[..LYRICS3_FOOTER_SIZE], LYRICS3V2_TAG_SIZE_VALUE);
        if len < LYRICS3_FOOTER_SIZE as i64 {
            return TAG_INVALID;
        } else if io.seek_io(-len, IoWhence::End) < 0 {
            return TAG_INVALID;
        } else if io.read_io(&mut buf[..LYRICS3V1_HEAD_SIZE]) != LYRICS3V1_HEAD_SIZE {
            return TAG_INVALID;
        } else if !verify_lyrics3v2(buf, LYRICS3V1_HEAD_SIZE) {
            return TAG_INVALID;
        }
        io.length -= len;
        return TAG_FOUND;
    } else if ver == 1 {
        let len = get_lyrics3v1_len(io);
        if len < 0 {
            return TAG_INVALID;
        }
        io.length -= len;
        return TAG_FOUND;
    }
    TAG_NOT_FOUND
}

/// `SDL_strtoll(str, &endp, 10)` where the entire string must be a valid
/// number: `(*str != '\0') && (*endp == '\0')`.
fn parse_whole_number(s: &[u8]) -> Option<i64> {
    let (value, consumed) = strtoll(s, 10);
    if !s.is_empty() && consumed == s.len() {
        Some(value)
    } else {
        None
    }
}

/// Translation of `ParseTrackNumString()`: `(track, total_tracks)`.
fn parse_track_num_string(s: Option<&[u8]>) -> (i64, i64) {
    let mut track = -1;
    let mut total_tracks = -1;

    let Some(s) = s else {
        return (track, total_tracks);
    };

    let mut trackstr = s;
    let mut totalstr: Option<&[u8]> = None;

    if let Some(ptr) = s.iter().position(|&c| c == b'/') {
        // see if it has both track _and_ total tracks.
        trackstr = &s[..ptr];
        totalstr = Some(&s[ptr + 1..]);
    }

    if let Some(ivalue) = parse_whole_number(trackstr) {
        // if true, entire string was a valid number.
        if ivalue >= 0 {
            // reject negative numbers, though.
            track = ivalue;
        }
    }

    if track > 0 {
        if let Some(totalstr) = totalstr {
            // FIXME (upstream): this parses `trackstr` again where it means
            // `totalstr`, so "3/10" reports 3 total tracks (when the total
            // is a valid number).
            let (ivalue, consumed) = strtoll(trackstr, 10);
            if !totalstr.is_empty() && consumed == trackstr.len() {
                // if true, entire string was a valid number.
                if ivalue >= 0 {
                    // reject negative numbers, though.
                    total_tracks = ivalue;
                }
            }
        }
    }

    (track, total_tracks)
}

/// Translation of `MIX_ReadMetadataTags()`: parse through `io` (a clamp,
/// whose `start` and `length` are moved past the tags found) for tags (ID3,
/// APE, MusicMatch, etc), and add metadata to props.
// !!! FIXME: several invalid tag things might cause a false return from here, but we should
// !!! FIXME:  just parse and clamp out what we can and only return false for legit i/o errors.
// !!! FIXME:  This will take a little work to clean out.
// !!! FIXME: as it stands, SDL_mixer ignores this return value and just accepts any new clamps
// !!! FIXME:  and properties. If there was a legit i/o error, it's going to find it shortly
// !!! FIXME:  as it tries to read the audio data anyhow.
pub(crate) fn read_metadata_tags(io: &mut IoClamp<'_>, props: &Properties) -> bool {
    let mut buf = [0u8; TAGS_INPUT_BUFFER_SIZE];

    /* MP3 standard has no metadata format, so everyone invented
     * their own thing, even with extensions, until ID3v2 became
     * dominant: Hence the impossible mess here.
     *
     * Note: I don't yet care about freaky broken mp3 files with
     * double tags. -- O.S.
     */

    // grab a block sufficient for initial tag discovery.
    let readsize = io.read_io(&mut buf);
    if readsize == 0 {
        return false;
    }

    // ID3v2 tag is at the start
    if is_id3v2(&buf, readsize) {
        let id3len = get_id3v2_len(&buf, readsize);
        if io.seek_io(0, IoWhence::Set) != 0 {
            return false;
        }
        parse_id3v2(props, io);
        io.start += id3len as i64;
        io.length -= id3len as i64;
    }
    // APE tag _might_ be at the start (discouraged
    // but not forbidden, either.)  read the header.
    else if is_apetag(&buf, readsize) {
        let mut v = 0u32;
        let apelen = get_ape_len(&buf, &mut v);
        if (v == APE_V1) || (v == APE_V2) {
            parse_ape(props, io, 0, v);
        }
        io.start += apelen;
        io.length -= apelen;
    }

    // we do not know the order of ape or lyrics3 or musicmatch tags (or if an extra tag was appended after a previous one instead of replacing it), hence the loop here..
    let mut found_any = false;
    loop {
        let mut found = false;
        // it's not impossible that _old_ MusicMatch tag
        // placing itself after ID3v1.
        if probe_mmtag(props, io, &mut buf) == TAG_FOUND {
            found = true;
        }

        if probe_id3v1(props, io, &mut buf, !found_any) == TAG_FOUND {
            found = true;
        }

        if probe_lyrics3(io, &mut buf) == TAG_FOUND {
            found = true;
        }

        if probe_mmtag(props, io, &mut buf) == TAG_FOUND {
            found = true;
        }

        if probe_apetag(props, io, &mut buf) == TAG_FOUND {
            found = true;
        }

        if found {
            found_any = true;
        } else {
            break;
        }
    }

    // Some tags we turn into standard SDL_mixer metadata keys...
    // favor them in this order: ID3v2.3+, ID3v2.2, APE, ID3v1 (maybe MusicMatch, eventually).
    static TITLE_KEYS: [&str; 4] = [
        "SDL_mixer.metadata.id3v2.TIT2",
        "SDL_mixer.metadata.id3v2.TT2",
        "SDL_mixer.metadata.ape.title",
        "SDL_mixer.metadata.id3v1.title",
    ];
    static ARTIST_KEYS: [&str; 4] = [
        "SDL_mixer.metadata.id3v2.TPE1",
        "SDL_mixer.metadata.id3v2.TP1",
        "SDL_mixer.metadata.ape.artist",
        "SDL_mixer.metadata.id3v1.artist",
    ];
    static ALBUM_KEYS: [&str; 4] = [
        "SDL_mixer.metadata.id3v2.TALB",
        "SDL_mixer.metadata.id3v2.TAL",
        "SDL_mixer.metadata.ape.album",
        "SDL_mixer.metadata.id3v1.album",
    ];
    static COPYRIGHT_KEYS: [&str; 4] = [
        "SDL_mixer.metadata.id3v2.TCOP",
        "SDL_mixer.metadata.id3v2.TCR",
        "SDL_mixer.metadata.ape.copyright",
        "SDL_mixer.metadata.id3v1.comment",
    ];
    static YEAR_KEYS: [&str; 4] = [
        "SDL_mixer.metadata.id3v2.TYER",
        "SDL_mixer.metadata.id3v2.TYE",
        "SDL_mixer.metadata.ape.year",
        "SDL_mixer.metadata.id3v1.year",
    ];

    // (true: a string property; false: a number)
    let tagmap: [(&str, bool, &[&str; 4]); 5] = [
        (PROP_METADATA_TITLE_STRING, true, &TITLE_KEYS),
        (PROP_METADATA_ARTIST_STRING, true, &ARTIST_KEYS),
        (PROP_METADATA_ALBUM_STRING, true, &ALBUM_KEYS),
        (PROP_METADATA_COPYRIGHT_STRING, true, &COPYRIGHT_KEYS),
        (PROP_METADATA_YEAR_NUMBER, false, &YEAR_KEYS),
    ];

    for (mixer, is_string, tags) in tagmap {
        if !props.contains(mixer) {
            for tag in tags.iter() {
                if let Some(s) = props.get_string(tag) {
                    if is_string {
                        let _ = props.set(mixer, s);
                    } else if let Some(ivalue) = parse_whole_number(s.as_bytes()) {
                        // if true, entire string was a valid number.
                        let _ = props.set(mixer, ivalue);
                    }
                    break;
                }
            }
        }
    }

    static TRACKNUM_KEYS: [&str; 4] = [
        "SDL_mixer.metadata.id3v2.TRCK",
        "SDL_mixer.metadata.id3v2.TRK",
        "SDL_mixer.metadata.ape.track",
        "SDL_mixer.metadata.id3v1.track",
    ];
    for key in TRACKNUM_KEYS {
        let s = props.get_string(key);
        let (track, total_tracks) = parse_track_num_string(s.as_deref().map(str::as_bytes));
        if track > 0 {
            let _ = props.set(PROP_METADATA_TRACK_NUMBER, track);
            if total_tracks > 0 {
                let _ = props.set(PROP_METADATA_TOTAL_TRACKS_NUMBER, total_tracks);
            }
            break;
        }
    }

    true
}

fn is_ogg_loop_tag(tag: &[u8]) -> bool {
    let buf = &tag[..tag.len().min(4)];
    strcasecmp(buf, "LOOP") == std::cmp::Ordering::Equal
}

/// `SDL_atoi()`.
fn atoi(s: &[u8]) -> i32 {
    sdl3::stdlib::string::strtol(s, 10).0 as i32
}

fn parse_ogg_time(time: &[u8], samplerate_hz: i64) -> i64 {
    /* Time is directly expressed as a sample position */
    if !time.contains(&b':') {
        return strtoll(time, 10).0;
    }

    let mut result: i64 = 0;
    let mut num_start = 0usize;

    for p in 0..time.len() {
        let c = time[p];
        if c == b'.' || c == b':' {
            let val = atoi(&time[num_start..p]);
            if val < 0 {
                return -1;
            }
            result = result.wrapping_mul(60).wrapping_add(val as i64);
            num_start = p + 1;
        }

        if c == b'.' {
            let val_f = strtod(&time[p..]).0;
            if val_f < 0.0 {
                return -1;
            }
            return result
                .wrapping_mul(samplerate_hz)
                .wrapping_add((val_f * samplerate_hz as f64) as i64);
        }
    }

    let val = atoi(&time[num_start..]);
    if val < 0 {
        return -1;
    }
    result
        .wrapping_mul(60)
        .wrapping_add(val as i64)
        .wrapping_mul(samplerate_hz)
}

/// `SDL_sscanf(value, "%d-%d-%d", ...) >= 1`: the first number, if any.
fn scan_first_int(s: &[u8]) -> Option<i32> {
    let (v, consumed) = sdl3::stdlib::string::strtol(s, 10);
    if consumed > 0 {
        Some(v as i32)
    } else {
        None
    }
}

/// Translation of `MIX_ParseOggComments()`. The vendor and comments are C
/// strings (they end at a NUL).
pub(crate) fn parse_ogg_comments(
    props: &Properties,
    freq: i32,
    vendor: Option<&[u8]>,
    user_comments: &[&[u8]],
    ogg_loop: &mut OggLoop,
) {
    if let Some(vendor) = vendor {
        if !cstr(vendor).is_empty() {
            let _ = props.set("SDL_mixer.metadata.ogg.vendor", cstring(vendor));
        }
    }

    ogg_loop.start = -1;
    ogg_loop.len = -1;
    ogg_loop.end = -1;
    ogg_loop.count = -1; // loops are infinite if a loop was specified but iteration count was not.

    let mut is_loop_length = false;
    for comment in user_comments {
        let param = cstr(comment);

        let (argument, value): (&[u8], &[u8]) = match param.iter().position(|&c| c == b'=') {
            None => (param, &[]),
            Some(eq) => (&param[..eq], &param[eq + 1..]),
        };
        let mut argument = argument.to_vec();

        // Want to match LOOP-START, LOOP_START, etc. Remove - or _ from string if it is present at position 4.
        if is_ogg_loop_tag(&argument)
            && argument.len() > 4
            && ((argument[4] == b'_') || (argument[4] == b'-'))
        {
            argument.remove(4);
        }

        let basekey = "SDL_mixer.metadata.ogg.";
        let mut generic_key = basekey.as_bytes().to_vec();
        generic_key.extend(argument.iter().map(|c| c.to_ascii_lowercase()));
        let _ = props.set(&cstring(&generic_key), cstring(value));

        let is = |name: &str| strcasecmp(&argument, name) == std::cmp::Ordering::Equal;

        if is("LOOPSTART") {
            ogg_loop.start = parse_ogg_time(value, freq as i64);
        } else if is("LOOPLENGTH") {
            ogg_loop.len = strtoll(value, 10).0;
            is_loop_length = true;
        } else if is("LOOPEND") {
            ogg_loop.end = parse_ogg_time(value, freq as i64);
            is_loop_length = false;
        } else if is("LOOPCOUNT") {
            ogg_loop.count = strtoll(value, 10).0;
            if ogg_loop.count <= 0 {
                ogg_loop.count = -1; // normalize infinite loop value.
            }
        } else if is("TITLE") {
            let _ = props.set(PROP_METADATA_TITLE_STRING, cstring(value));
        } else if is("ARTIST") {
            let _ = props.set(PROP_METADATA_ARTIST_STRING, cstring(value));
        } else if is("ALBUM") {
            let _ = props.set(PROP_METADATA_ALBUM_STRING, cstring(value));
        } else if is("COPYRIGHT") {
            let _ = props.set(PROP_METADATA_COPYRIGHT_STRING, cstring(value));
        } else if is("TRACKNUMBER") {
            let (track, total_tracks) = parse_track_num_string(Some(value));
            if track > 0 {
                let _ = props.set(PROP_METADATA_TRACK_NUMBER, track);
                if total_tracks > 0 {
                    let _ = props.set(PROP_METADATA_TOTAL_TRACKS_NUMBER, total_tracks);
                }
            }
        } else if is("DATE") {
            if let Some(year) = scan_first_int(value) {
                if year > 0 {
                    let _ = props.set(PROP_METADATA_YEAR_NUMBER, year as i64);
                }
            }
        }
    }

    if is_loop_length {
        ogg_loop.end = ogg_loop.start.wrapping_add(ogg_loop.len);
    } else {
        ogg_loop.len = ogg_loop.end.wrapping_sub(ogg_loop.start);
    }

    if ogg_loop.end == 0 || (ogg_loop.end < ogg_loop.start) {
        ogg_loop.len = -1; // invalidate the whole thing.
    }

    // Ignore invalid or missing loop tag
    ogg_loop.active = (ogg_loop.start >= 0) && (ogg_loop.len >= 0) && (ogg_loop.end >= 0);
    if !ogg_loop.active {
        ogg_loop.start = 0;
        ogg_loop.len = 0;
        ogg_loop.end = 0;
        ogg_loop.count = -1;
    }
}
