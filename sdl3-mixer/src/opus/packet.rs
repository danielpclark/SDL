// Rust translation of the packet parsing of src/opus.c from libopus 1.4
// (https://opus-codec.org, at the revision SDL_mixer's external/opus pins).
// Copyright (c) 2011 Xiph.Org Foundation, Skype Limited
// Written by Jean-Marc Valin and Koen Vos
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Opus packet parsing. `opus_pcm_soft_clip()` and `encode_size()` (the
//! rest of opus.c) aren't used by the float decoder.

use super::{OPUS_BAD_ARG, OPUS_INVALID_PACKET};

fn parse_size(data: &[u8], len: i32, size: &mut i16) -> i32 {
    if len < 1 {
        *size = -1;
        -1
    } else if data[0] < 252 {
        *size = data[0] as i16;
        1
    } else if len < 2 {
        *size = -1;
        -1
    } else {
        *size = 4 * data[1] as i16 + data[0] as i16;
        2
    }
}

/// Translation of `opus_packet_get_samples_per_frame()`.
pub(crate) fn opus_packet_get_samples_per_frame(data: &[u8], fs: i32) -> i32 {
    let mut audiosize;
    if data[0] & 0x80 != 0 {
        audiosize = ((data[0] >> 3) & 0x3) as i32;
        audiosize = (fs << audiosize) / 400;
    } else if (data[0] & 0x60) == 0x60 {
        audiosize = if data[0] & 0x08 != 0 {
            fs / 50
        } else {
            fs / 100
        };
    } else {
        audiosize = ((data[0] >> 3) & 0x3) as i32;
        if audiosize == 3 {
            audiosize = fs * 60 / 1000;
        } else {
            audiosize = (fs << audiosize) / 100;
        }
    }
    audiosize
}

/// Translation of `opus_packet_parse_impl()`. `data` holds (at least) the
/// `len` bytes of the packet. The `frames` output isn't translated: the
/// decoder never asks for it (it passes NULL).
#[allow(clippy::too_many_arguments)]
pub(crate) fn opus_packet_parse_impl(
    data: &[u8],
    mut len: i32,
    self_delimited: bool,
    out_toc: Option<&mut u8>,
    size: &mut [i16; 48],
    payload_offset: Option<&mut i32>,
    packet_offset: Option<&mut i32>,
) -> i32 {
    let count: i32;
    let mut cbr;
    let mut last_size: i32;
    let mut pad: i32 = 0;
    let mut p = 0usize;

    if len < 0 {
        return OPUS_BAD_ARG;
    }
    if len == 0 {
        return OPUS_INVALID_PACKET;
    }

    let framesize = opus_packet_get_samples_per_frame(data, 48000);

    cbr = false;
    let toc = data[p];
    p += 1;
    len -= 1;
    last_size = len;
    match toc & 0x3 {
        /* One frame */
        0 => {
            count = 1;
        }
        /* Two CBR frames */
        1 => {
            count = 2;
            cbr = true;
            if !self_delimited {
                if len & 0x1 != 0 {
                    return OPUS_INVALID_PACKET;
                }
                last_size = len / 2;
                /* If last_size doesn't fit in size[0], we'll catch it later */
                size[0] = last_size as i16;
            }
        }
        /* Two VBR frames */
        2 => {
            count = 2;
            let bytes = parse_size(&data[p..], len, &mut size[0]);
            len -= bytes;
            if size[0] < 0 || size[0] as i32 > len {
                return OPUS_INVALID_PACKET;
            }
            p += bytes as usize;
            last_size = len - size[0] as i32;
        }
        /* Multiple CBR/VBR frames (from 0 to 120 ms) */
        _ => {
            /*case 3:*/
            if len < 1 {
                return OPUS_INVALID_PACKET;
            }
            /* Number of frames encoded in bits 0 to 5 */
            let ch = data[p];
            p += 1;
            count = (ch & 0x3F) as i32;
            if count <= 0 || framesize * count > 5760 {
                return OPUS_INVALID_PACKET;
            }
            len -= 1;
            /* Padding flag is bit 6 */
            if ch & 0x40 != 0 {
                loop {
                    if len <= 0 {
                        return OPUS_INVALID_PACKET;
                    }
                    let pb = data[p] as i32;
                    p += 1;
                    len -= 1;
                    let tmp = if pb == 255 { 254 } else { pb };
                    len -= tmp;
                    pad += tmp;
                    if pb != 255 {
                        break;
                    }
                }
            }
            if len < 0 {
                return OPUS_INVALID_PACKET;
            }
            /* VBR flag is bit 7 */
            cbr = ch & 0x80 == 0;
            if !cbr {
                /* VBR case */
                last_size = len;
                for i in 0..(count - 1) as usize {
                    let bytes = parse_size(&data[p..], len, &mut size[i]);
                    len -= bytes;
                    if size[i] < 0 || size[i] as i32 > len {
                        return OPUS_INVALID_PACKET;
                    }
                    p += bytes as usize;
                    last_size -= bytes + size[i] as i32;
                }
                if last_size < 0 {
                    return OPUS_INVALID_PACKET;
                }
            } else if !self_delimited {
                /* CBR case */
                last_size = len / count;
                if last_size * count != len {
                    return OPUS_INVALID_PACKET;
                }
                for s in size[..(count - 1) as usize].iter_mut() {
                    *s = last_size as i16;
                }
            }
        }
    }
    let cu = count as usize;
    /* Self-delimited framing has an extra size for the last frame. */
    if self_delimited {
        let bytes = parse_size(&data[p..], len, &mut size[cu - 1]);
        len -= bytes;
        if size[cu - 1] < 0 || size[cu - 1] as i32 > len {
            return OPUS_INVALID_PACKET;
        }
        p += bytes as usize;
        /* For CBR packets, apply the size to all the frames. */
        if cbr {
            if size[cu - 1] as i32 * count > len {
                return OPUS_INVALID_PACKET;
            }
            let last = size[cu - 1];
            for s in size[..cu - 1].iter_mut() {
                *s = last;
            }
        } else if bytes + size[cu - 1] as i32 > last_size {
            return OPUS_INVALID_PACKET;
        }
    } else {
        /* Because it's not encoded explicitly, it's possible the size of the
        last packet (or all the packets, for the CBR case) is larger than
        1275. Reject them here.*/
        if last_size > 1275 {
            return OPUS_INVALID_PACKET;
        }
        size[cu - 1] = last_size as i16;
    }

    if let Some(po) = payload_offset {
        *po = p as i32;
    }

    for s in size[..cu].iter() {
        p += *s as usize;
    }

    if let Some(po) = packet_offset {
        *po = pad + p as i32;
    }

    if let Some(t) = out_toc {
        *t = toc;
    }

    count
}
