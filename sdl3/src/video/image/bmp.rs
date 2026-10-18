// Rust translation of the BMP loader of stb_image.h v2.30 (Sean Barrett and
// contributors, http://nothings.org/stb), from the copy PlutoVG bundles
// (source/plutovg-stb-image.h in SDL_ttf's external/plutovg), which SDL's
// own copy compiles out (`STBI_ONLY_PNG`, `STBI_ONLY_JPEG`).
// stb_image is available under the MIT License or in the public domain
// (see LICENSE.txt).
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Microsoft/Windows BMP image: 1/4/8-bit paletted, 16/24/32-bit with
//! default or explicit bit masks, v1 to v5 headers; no RLE.

use super::stb_image::{
    convert_format8, err, mad3sizes_valid, malloc_mad3, Context, Image, Pixels, STBI_MAX_DIMENSIONS,
};
use crate::error::Result;

/// Translation of `stbi__bmp_test_raw()`.
fn bmp_test_raw(s: &mut Context<'_>) -> bool {
    if s.get8() != b'B' {
        return false;
    }
    if s.get8() != b'M' {
        return false;
    }
    s.get32le(); // discard filesize
    s.get16le(); // discard reserved
    s.get16le(); // discard reserved
    s.get32le(); // discard data offset
    let sz = s.get32le();
    sz == 12 || sz == 40 || sz == 56 || sz == 108 || sz == 124
}

/// Translation of `stbi__bmp_test()`.
pub(crate) fn bmp_test(s: &mut Context<'_>) -> bool {
    let r = bmp_test_raw(s);
    s.rewind();
    r
}

/// returns 0..31 for the highest set bit. Translation of `stbi__high_bit()`.
fn high_bit(mut z: u32) -> i32 {
    let mut n = 0;
    if z == 0 {
        return -1;
    }
    if z >= 0x10000 {
        n += 16;
        z >>= 16;
    }
    if z >= 0x00100 {
        n += 8;
        z >>= 8;
    }
    if z >= 0x00010 {
        n += 4;
        z >>= 4;
    }
    if z >= 0x00004 {
        n += 2;
        z >>= 2;
    }
    if z >= 0x00002 {
        n += 1; /* >>=  1;*/
    }
    n
}

/// Translation of `stbi__bitcount()`.
fn bitcount(mut a: u32) -> i32 {
    a = (a & 0x55555555) + ((a >> 1) & 0x55555555); // max 2
    a = (a & 0x33333333) + ((a >> 2) & 0x33333333); // max 4
    a = (a + (a >> 4)) & 0x0f0f0f0f; // max 8 per 4, now 8 bits
    a = a.wrapping_add(a >> 8); // max 16 per 8 bits
    a = a.wrapping_add(a >> 16); // max 32 per 8 bits
    (a & 0xff) as i32
}

/// extract an arbitrarily-aligned N-bit value (N=bits)
/// from v, and then make it 8-bits long and fractionally
/// extend it to full full range. Translation of `stbi__shiftsigned()`.
fn shiftsigned(mut v: u32, shift: i32, bits: i32) -> i32 {
    static MUL_TABLE: [u32; 9] = [
        0, 0xff, /*0b11111111*/ 0x55, /*0b01010101*/ 0x49, /*0b01001001*/
        0x11, /*0b00010001*/ 0x21, /*0b00100001*/ 0x41, /*0b01000001*/
        0x81, /*0b10000001*/ 0x01, /*0b00000001*/
    ];
    static SHIFT_TABLE: [u32; 9] = [0, 0, 0, 1, 0, 2, 4, 6, 0];
    if shift < 0 {
        v <<= -shift;
    } else {
        v >>= shift;
    }
    // (STBI_ASSERT(v < 256): compiled out, NDEBUG; the masks' high bits
    // make it hold)
    v >>= 8 - bits;
    // (STBI_ASSERT(bits >= 0 && bits <= 8): the callers check the counts)
    (v.wrapping_mul(MUL_TABLE[bits as usize]) >> SHIFT_TABLE[bits as usize]) as i32
}

/// Translation of `stbi__bmp_data`.
#[derive(Default)]
struct BmpData {
    bpp: i32,
    offset: i32,
    hsz: i32,
    mr: u32,
    mg: u32,
    mb: u32,
    ma: u32,
    all_a: u32,
    extra_read: i32,
}

/// Translation of `stbi__bmp_set_mask_defaults()`.
fn bmp_set_mask_defaults(info: &mut BmpData, compress: i32) -> bool {
    // BI_BITFIELDS specifies masks explicitly, don't override
    if compress == 3 {
        return true;
    }

    if compress == 0 {
        if info.bpp == 16 {
            info.mr = 31 << 10;
            info.mg = 31 << 5;
            info.mb = 31;
        } else if info.bpp == 32 {
            info.mr = 0xff << 16;
            info.mg = 0xff << 8;
            info.mb = 0xff;
            info.ma = 0xff << 24;
            info.all_a = 0; // if all_a is 0 at end, then we loaded alpha channel but it was all 0
        } else {
            // otherwise, use defaults, which is all-0
            info.mr = 0;
            info.mg = 0;
            info.mb = 0;
            info.ma = 0;
        }
        return true;
    }
    false // error
}

/// Translation of `stbi__bmp_parse_header()`.
fn bmp_parse_header(s: &mut Context<'_>, info: &mut BmpData) -> Result<()> {
    if s.get8() != b'B' || s.get8() != b'M' {
        return err("Corrupt BMP");
    }
    s.get32le(); // discard filesize
    s.get16le(); // discard reserved
    s.get16le(); // discard reserved
    info.offset = s.get32le() as i32;
    let hsz = s.get32le() as i32;
    info.hsz = hsz;
    info.mr = 0;
    info.mg = 0;
    info.mb = 0;
    info.ma = 0;
    info.extra_read = 14;

    if info.offset < 0 {
        return err("bad BMP");
    }

    if hsz != 12 && hsz != 40 && hsz != 56 && hsz != 108 && hsz != 124 {
        return err("BMP type not supported: unknown");
    }
    if hsz == 12 {
        s.img_x = s.get16le() as u32;
        s.img_y = s.get16le() as u32;
    } else {
        s.img_x = s.get32le();
        s.img_y = s.get32le();
    }
    if s.get16le() != 1 {
        return err("bad BMP");
    }
    info.bpp = s.get16le();
    if hsz != 12 {
        let compress = s.get32le() as i32;
        if compress == 1 || compress == 2 {
            return err("BMP type not supported: RLE");
        }
        if compress >= 4 {
            return err("BMP type not supported: unsupported compression"); // this includes PNG/JPEG modes
        }
        if compress == 3 && info.bpp != 16 && info.bpp != 32 {
            return err("bad BMP"); // bitfields requires 16 or 32 bits/pixel
        }
        s.get32le(); // discard sizeof
        s.get32le(); // discard hres
        s.get32le(); // discard vres
        s.get32le(); // discard colorsused
        s.get32le(); // discard max important
        if hsz == 40 || hsz == 56 {
            if hsz == 56 {
                s.get32le();
                s.get32le();
                s.get32le();
                s.get32le();
            }
            if info.bpp == 16 || info.bpp == 32 {
                if compress == 0 {
                    bmp_set_mask_defaults(info, compress);
                } else if compress == 3 {
                    info.mr = s.get32le();
                    info.mg = s.get32le();
                    info.mb = s.get32le();
                    info.extra_read += 12;
                    // not documented, but generated by photoshop and handled by mspaint
                    if info.mr == info.mg && info.mg == info.mb {
                        // ?!?!?
                        return err("bad BMP");
                    }
                } else {
                    return err("bad BMP");
                }
            }
        } else {
            // V4/V5 header
            if hsz != 108 && hsz != 124 {
                return err("bad BMP");
            }
            info.mr = s.get32le();
            info.mg = s.get32le();
            info.mb = s.get32le();
            info.ma = s.get32le();
            if compress != 3 {
                // override mr/mg/mb unless in BI_BITFIELDS mode, as per docs
                bmp_set_mask_defaults(info, compress);
            }
            s.get32le(); // discard color space
            for _ in 0..12 {
                s.get32le(); // discard color space parameters
            }
            if hsz == 124 {
                s.get32le(); // discard rendering intent
                s.get32le(); // discard offset of profile data
                s.get32le(); // discard size of profile data
                s.get32le(); // discard reserved
            }
        }
    }
    Ok(())
}

/// Translation of `stbi__bmp_load()`.
pub(crate) fn bmp_load(s: &mut Context<'_>, req_comp: i32) -> Result<Image<Pixels>> {
    /* FIXME (upstream): the palette is uninitialized in C, and the
    pixels' indices aren't checked against the number of entries read
    (stb reads whatever its stack holds for the others); it is zeroed
    here. */
    let mut pal = [[0u8; 4]; 256];
    let mut psize: i32 = 0;
    let width: i32;
    let pad: i32;

    let mut info = BmpData {
        all_a: 255,
        ..BmpData::default()
    };
    bmp_parse_header(s, &mut info)?; // error code already set

    let flip_vertically = (s.img_y as i32) > 0;
    s.img_y = (s.img_y as i32).wrapping_abs() as u32;

    if s.img_y > STBI_MAX_DIMENSIONS {
        return err("Very large image (corrupt?)");
    }
    if s.img_x > STBI_MAX_DIMENSIONS {
        return err("Very large image (corrupt?)");
    }

    let (mr, mg, mb, ma) = (info.mr, info.mg, info.mb, info.ma);
    let mut all_a = info.all_a;

    if info.hsz == 12 {
        if info.bpp < 24 {
            psize = (info.offset - info.extra_read - 24) / 3;
        }
    } else if info.bpp < 16 {
        psize = (info.offset - info.extra_read - info.hsz) >> 2;
    }
    if psize == 0 {
        // accept some number of extra bytes after the header, but if the offset points either to before
        // the header ends or implies a large amount of extra data, reject the file as malformed
        let bytes_read_so_far = s.bytes_read();
        let header_limit = 1024; // max we actually read is below 256 bytes currently.
        let extra_data_limit = 256 * 4; // what ordinarily goes here is a palette; 256 entries*4 bytes is its max size.
        if bytes_read_so_far <= 0 || bytes_read_so_far > header_limit {
            return err("Corrupt BMP");
        }
        // we established that bytes_read_so_far is positive and sensible.
        // the first half of this test rejects offsets that are either too small positives, or
        // negative, and guarantees that info.offset >= bytes_read_so_far > 0. this in turn
        // ensures the number computed in the second half of the test can't overflow.
        if info.offset < bytes_read_so_far || info.offset - bytes_read_so_far > extra_data_limit {
            return err("Corrupt BMP");
        } else {
            s.skip(info.offset - bytes_read_so_far);
        }
    }

    s.img_n = if info.bpp == 24 && ma == 0xff000000 {
        3
    } else if ma != 0 {
        4
    } else {
        3
    };
    let target = if req_comp != 0 && req_comp >= 3 {
        // we can directly decode 3 or 4
        req_comp
    } else {
        s.img_n // if they want monochrome, we'll post-convert
    };

    // sanity-check size
    if !mad3sizes_valid(target, s.img_x as i32, s.img_y as i32, 0) {
        return err("Corrupt BMP");
    }

    let Some(mut out) = malloc_mad3(target, s.img_x as i32, s.img_y as i32, 0) else {
        return err("Out of memory");
    };
    let (img_x, img_y) = (s.img_x as i32, s.img_y as i32);
    let mut z = 0usize;
    if info.bpp < 16 {
        if psize == 0 || psize > 256 {
            return err("Corrupt BMP");
        }
        // (a negative psize, from an offset before the header's end, reads
        // no entries)
        for p in pal.iter_mut().take(psize.max(0) as usize) {
            p[2] = s.get8();
            p[1] = s.get8();
            p[0] = s.get8();
            if info.hsz != 12 {
                s.get8();
            }
            p[3] = 255;
        }
        s.skip(
            info.offset - info.extra_read - info.hsz - psize * if info.hsz == 12 { 3 } else { 4 },
        );
        width = match info.bpp {
            1 => (s.img_x.wrapping_add(7) >> 3) as i32,
            4 => (s.img_x.wrapping_add(1) >> 1) as i32,
            8 => s.img_x as i32,
            _ => return err("Corrupt BMP"),
        };
        pad = width.wrapping_neg() & 3;
        let put = |out: &mut [u8], z: &mut usize, color: usize| {
            out[*z] = pal[color][0];
            out[*z + 1] = pal[color][1];
            out[*z + 2] = pal[color][2];
            *z += 3;
            if target == 4 {
                out[*z] = 255;
                *z += 1;
            }
        };
        if info.bpp == 1 {
            for _ in 0..img_y {
                let mut bit_offset = 7;
                let mut v = s.get8() as i32;
                for i in 0..img_x {
                    let color = (v >> bit_offset) & 0x1;
                    put(&mut out, &mut z, color as usize);
                    if i + 1 == img_x {
                        break;
                    }
                    bit_offset -= 1;
                    if bit_offset < 0 {
                        bit_offset = 7;
                        v = s.get8() as i32;
                    }
                }
                s.skip(pad);
            }
        } else {
            for _ in 0..img_y {
                let mut i = 0;
                while i < img_x {
                    let mut v = s.get8() as i32;
                    let mut v2 = 0;
                    if info.bpp == 4 {
                        v2 = v & 15;
                        v >>= 4;
                    }
                    put(&mut out, &mut z, v as usize);
                    if i + 1 == img_x {
                        break;
                    }
                    v = if info.bpp == 8 { s.get8() as i32 } else { v2 };
                    put(&mut out, &mut z, v as usize);
                    i += 2;
                }
                s.skip(pad);
            }
        }
    } else {
        let (mut rshift, mut gshift, mut bshift, mut ashift) = (0, 0, 0, 0);
        let (mut rcount, mut gcount, mut bcount, mut acount) = (0, 0, 0, 0);
        let mut easy = 0;
        s.skip(info.offset - info.extra_read - info.hsz);
        width = if info.bpp == 24 {
            3i32.wrapping_mul(s.img_x as i32)
        } else if info.bpp == 16 {
            2i32.wrapping_mul(s.img_x as i32)
        } else {
            /* bpp = 32 and pad = 0 */
            0
        };
        pad = width.wrapping_neg() & 3;
        if info.bpp == 24 {
            easy = 1;
        } else if info.bpp == 32
            && mb == 0xff
            && mg == 0xff00
            && mr == 0x00ff0000
            && ma == 0xff000000
        {
            easy = 2;
        }
        if easy == 0 {
            if mr == 0 || mg == 0 || mb == 0 {
                return err("Corrupt BMP");
            }
            // right shift amt to put high bit in position #7
            rshift = high_bit(mr) - 7;
            rcount = bitcount(mr);
            gshift = high_bit(mg) - 7;
            gcount = bitcount(mg);
            bshift = high_bit(mb) - 7;
            bcount = bitcount(mb);
            ashift = high_bit(ma) - 7;
            acount = bitcount(ma);
            if rcount > 8 || gcount > 8 || bcount > 8 || acount > 8 {
                return err("Corrupt BMP");
            }
        }
        for _ in 0..img_y {
            if easy != 0 {
                for _ in 0..img_x {
                    out[z + 2] = s.get8();
                    out[z + 1] = s.get8();
                    out[z] = s.get8();
                    z += 3;
                    let a = if easy == 2 { s.get8() } else { 255 };
                    all_a |= a as u32;
                    if target == 4 {
                        out[z] = a;
                        z += 1;
                    }
                }
            } else {
                let bpp = info.bpp;
                for _ in 0..img_x {
                    let v = if bpp == 16 {
                        s.get16le() as u32
                    } else {
                        s.get32le()
                    };
                    out[z] = shiftsigned(v & mr, rshift, rcount) as u8;
                    out[z + 1] = shiftsigned(v & mg, gshift, gcount) as u8;
                    out[z + 2] = shiftsigned(v & mb, bshift, bcount) as u8;
                    z += 3;
                    let a = if ma != 0 {
                        shiftsigned(v & ma, ashift, acount) as u32
                    } else {
                        255
                    };
                    all_a |= a;
                    if target == 4 {
                        out[z] = a as u8;
                        z += 1;
                    }
                }
            }
            s.skip(pad);
        }
    }

    // if alpha channel is all 0s, replace with all 255s
    if target == 4 && all_a == 0 {
        for px in out.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }

    if flip_vertically {
        let row = img_x as usize * target as usize;
        for j in 0..(img_y >> 1) as usize {
            let (top, bottom) = out.split_at_mut((img_y as usize - 1 - j) * row);
            top[j * row..(j + 1) * row].swap_with_slice(&mut bottom[..row]);
        }
    }

    if req_comp != 0 && req_comp != target {
        out = convert_format8(out, target, req_comp, s.img_x, s.img_y)?;
    }

    Ok(Image {
        x: s.img_x as i32,
        y: s.img_y as i32,
        comp: s.img_n,
        data: Pixels::Bits8(out),
    })
}
