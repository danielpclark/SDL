// Rust translation of the Softimage PIC loader of stb_image.h v2.30 (Sean
// Barrett and contributors, http://nothings.org/stb; "Softimage PIC loader
// by Tom Seddon"), from the copy PlutoVG bundles
// (source/plutovg-stb-image.h in SDL_ttf's external/plutovg), which SDL's
// own copy compiles out (`STBI_ONLY_PNG`, `STBI_ONLY_JPEG`).
// stb_image is available under the MIT License or in the public domain
// (see LICENSE.txt).
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Softimage PIC: 8-bit channel packets, uncompressed, pure RLE or mixed
//! RLE.
//!
//! See <http://softimage.wiki.softimage.com/index.php/INFO:_PIC_file_format>
//! and <http://ozviz.wasp.uwa.edu.au/~pbourke/dataformats/softimagepic/>.

use super::stb_image::{
    convert_format8, err, mad3sizes_valid, malloc_mad3, Context, Image, Pixels, STBI_MAX_DIMENSIONS,
};
use crate::error::Result;

/// Translation of `stbi__pic_is4()`.
fn pic_is4(s: &mut Context<'_>, str: &[u8; 4]) -> bool {
    for &c in str {
        if s.get8() != c {
            return false;
        }
    }
    true
}

/// Translation of `stbi__pic_test_core()`.
fn pic_test_core(s: &mut Context<'_>) -> bool {
    if !pic_is4(s, b"\x53\x80\xF6\x34") {
        return false;
    }

    for _ in 0..84 {
        s.get8();
    }

    if !pic_is4(s, b"PICT") {
        return false;
    }

    true
}

/// Translation of `stbi__pic_packet`.
#[derive(Clone, Copy, Default)]
struct PicPacket {
    size: u8,
    ty: u8,
    channel: u8,
}

/// Translation of `stbi__readval()`.
fn readval(s: &mut Context<'_>, channel: i32, dest: &mut [u8]) -> Result<()> {
    let mut mask = 0x80;
    for d in dest.iter_mut().take(4) {
        if channel & mask != 0 {
            if s.at_eof() {
                return err("PIC file too short");
            }
            *d = s.get8();
        }
        mask >>= 1;
    }
    Ok(())
}

/// Translation of `stbi__copyval()`.
fn copyval(channel: i32, dest: &mut [u8], src: &[u8; 4]) {
    let mut mask = 0x80;
    for i in 0..4 {
        if channel & mask != 0 {
            dest[i] = src[i];
        }
        mask >>= 1;
    }
}

/// Translation of `stbi__pic_load_core()`: returns the number of
/// components (`*comp`).
fn pic_load_core(s: &mut Context<'_>, width: i32, height: i32, result: &mut [u8]) -> Result<i32> {
    let mut act_comp = 0;
    let mut num_packets = 0;
    let mut packets = [PicPacket::default(); 10];

    // this will (should...) cater for even some bizarre stuff like having data
    // for the same channel in multiple packets.
    loop {
        if num_packets == packets.len() {
            return err("too many packets");
        }

        let packet = &mut packets[num_packets];
        num_packets += 1;

        let chained = s.get8();
        packet.size = s.get8();
        packet.ty = s.get8();
        packet.channel = s.get8();

        act_comp |= packet.channel as i32;

        if s.at_eof() {
            return err("file too short (reading packets)");
        }
        if packet.size != 8 {
            return err("packet isn't 8bpp");
        }
        if chained == 0 {
            break;
        }
    }

    let comp = if act_comp & 0x10 != 0 { 4 } else { 3 }; // has alpha channel?

    let row = width as usize * 4;
    for y in 0..height as usize {
        for packet in &packets[..num_packets] {
            let dest = &mut result[y * row..(y + 1) * row];
            let channel = packet.channel as i32;

            match packet.ty {
                0 => {
                    //uncompressed
                    for px in dest.chunks_exact_mut(4) {
                        readval(s, channel, px)?;
                    }
                }

                1 => {
                    //Pure RLE
                    let mut left = width;
                    let mut d = 0usize;

                    while left > 0 {
                        let mut value = [0u8; 4];

                        let mut count = s.get8();
                        if s.at_eof() {
                            return err("file too short (pure read count)");
                        }

                        if count as i32 > left {
                            count = left as u8;
                        }

                        readval(s, channel, &mut value)?;

                        for _ in 0..count {
                            copyval(channel, &mut dest[d..d + 4], &value);
                            d += 4;
                        }
                        left -= count as i32;
                    }
                }

                2 => {
                    //Mixed RLE
                    let mut left = width;
                    let mut d = 0usize;
                    while left > 0 {
                        let mut count = s.get8() as i32;
                        if s.at_eof() {
                            return err("file too short (mixed read count)");
                        }

                        if count >= 128 {
                            // Repeated
                            let mut value = [0u8; 4];

                            if count == 128 {
                                count = s.get16be();
                            } else {
                                count -= 127;
                            }
                            if count > left {
                                return err("scanline overrun");
                            }

                            readval(s, channel, &mut value)?;

                            for _ in 0..count {
                                copyval(channel, &mut dest[d..d + 4], &value);
                                d += 4;
                            }
                        } else {
                            // Raw
                            count += 1;
                            if count > left {
                                return err("scanline overrun");
                            }

                            for _ in 0..count {
                                readval(s, channel, &mut dest[d..d + 4])?;
                                d += 4;
                            }
                        }
                        left -= count;
                    }
                }

                _ => return err("packet has bad compression type"),
            }
        }
    }

    Ok(comp)
}

/// Translation of `stbi__pic_load()`.
pub(crate) fn pic_load(s: &mut Context<'_>, req_comp: i32) -> Result<Image<Pixels>> {
    for _ in 0..92 {
        s.get8();
    }

    let x = s.get16be();
    let y = s.get16be();

    if y > STBI_MAX_DIMENSIONS as i32 {
        return err("Very large image (corrupt?)");
    }
    if x > STBI_MAX_DIMENSIONS as i32 {
        return err("Very large image (corrupt?)");
    }

    if s.at_eof() {
        return err("file too short (pic header)");
    }
    if !mad3sizes_valid(x, y, 4, 0) {
        return err("PIC image too large to decode");
    }

    s.get32be(); //skip `ratio'
    s.get16be(); //skip `fields'
    s.get16be(); //skip `pad'

    // intermediate buffer is RGBA
    let Some(mut result) = malloc_mad3(x, y, 4, 0) else {
        return err("Out of memory");
    };
    result.fill(0xff);

    /* (on failure, C frees the result and then passes NULL to
    stbi__convert_format(), which returns it as it is for the 4
    components PlutoVG asks for, and dereferences it for the others) */
    let comp = pic_load_core(s, x, y, &mut result)?;
    let req_comp = if req_comp == 0 { comp } else { req_comp };
    let result = convert_format8(result, 4, req_comp, x as u32, y as u32)?;

    Ok(Image {
        x,
        y,
        comp,
        data: Pixels::Bits8(result),
    })
}

/// Translation of `stbi__pic_test()`.
pub(crate) fn pic_test(s: &mut Context<'_>) -> bool {
    let r = pic_test_core(s);
    s.rewind();
    r
}
