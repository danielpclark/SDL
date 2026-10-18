// Rust translation of the TGA loader of stb_image.h v2.30 (Sean Barrett and
// contributors, http://nothings.org/stb; "Targa Truevision - TGA by
// Jonathan Dummer"), from the copy PlutoVG bundles
// (source/plutovg-stb-image.h in SDL_ttf's external/plutovg), which SDL's
// own copy compiles out (`STBI_ONLY_PNG`, `STBI_ONLY_JPEG`).
// stb_image is available under the MIT License or in the public domain
// (see LICENSE.txt).
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Targa Truevision - TGA: grey, RGB, RGBA and 15/16-bit RGB, paletted or
//! not, RLE or not.

use super::stb_image::{
    convert_format8, err, mad3sizes_valid, malloc_mad2, malloc_mad3, Context, Image, Pixels,
    STBI_MAX_DIMENSIONS,
};
use crate::error::Result;

/// returns STBI_rgb or whatever, 0 on error. Translation of
/// `stbi__tga_get_comp()`.
fn tga_get_comp(bits_per_pixel: i32, is_grey: bool, is_rgb16: Option<&mut bool>) -> i32 {
    // only RGB or RGBA (incl. 16bit) or grey allowed
    let mut rgb16 = false;
    let comp = match bits_per_pixel {
        8 => 1,             // STBI_grey
        16 if is_grey => 2, // STBI_grey_alpha
        15 | 16 => {
            rgb16 = true;
            3 // STBI_rgb
        }
        24 | 32 => bits_per_pixel / 8,
        _ => 0,
    };
    if let Some(is_rgb16) = is_rgb16 {
        *is_rgb16 = rgb16;
    }
    comp
}

/// Translation of `stbi__tga_test()`.
pub(crate) fn tga_test(s: &mut Context<'_>) -> bool {
    let res = (|| {
        s.get8(); //   discard Offset
        let tga_color_type = s.get8(); //   color type
        if tga_color_type > 1 {
            return false; //   only RGB or indexed allowed
        }
        let sz = s.get8(); //   image type
        if tga_color_type == 1 {
            // colormapped (paletted) image
            if sz != 1 && sz != 9 {
                return false; // colortype 1 demands image type 1 or 9
            }
            s.skip(4); // skip index of first colormap entry and number of entries
            let sz = s.get8(); //   check bits per palette color entry
            if sz != 8 && sz != 15 && sz != 16 && sz != 24 && sz != 32 {
                return false;
            }
            s.skip(4); // skip image x and y origin
        } else {
            // "normal" image w/o colormap
            if sz != 2 && sz != 3 && sz != 10 && sz != 11 {
                return false; // only RGB or grey allowed, +/- RLE
            }
            s.skip(9); // skip colormap specification and image x/y origin
        }
        if s.get16le() < 1 {
            return false; //   test width
        }
        if s.get16le() < 1 {
            return false; //   test height
        }
        let sz = s.get8(); //   bits per pixel
        if tga_color_type == 1 && sz != 8 && sz != 16 {
            return false; // for colormapped images, bpp is size of an index
        }
        if sz != 8 && sz != 15 && sz != 16 && sz != 24 && sz != 32 {
            return false;
        }
        true // if we got this far, everything's good and we can return 1 instead of 0
    })();
    s.rewind();
    res
}

/// read 16bit value and convert to 24bit RGB. Translation of
/// `stbi__tga_read_rgb16()`.
fn tga_read_rgb16(s: &mut Context<'_>, out: &mut [u8]) {
    let px = s.get16le() as u16;
    let five_bit_mask: u16 = 31;
    // we have 3 channels with 5bits each
    let r = ((px >> 10) & five_bit_mask) as i32;
    let g = ((px >> 5) & five_bit_mask) as i32;
    let b = (px & five_bit_mask) as i32;
    // Note that this saves the data in RGB(A) order, so it doesn't need to be swapped later
    out[0] = ((r * 255) / 31) as u8;
    out[1] = ((g * 255) / 31) as u8;
    out[2] = ((b * 255) / 31) as u8;

    // some people claim that the most significant bit might be used for alpha
    // (possibly if an alpha-bit is set in the "image descriptor byte")
    // but that only made 16bit test images completely translucent..
    // so let's treat all 15 and 16bit TGAs as RGB with no alpha.
}

/// Translation of `stbi__tga_load()`.
pub(crate) fn tga_load(s: &mut Context<'_>, req_comp: i32) -> Result<Image<Pixels>> {
    //   read in the TGA header stuff
    let tga_offset = s.get8() as i32;
    let tga_indexed = s.get8() as i32;
    let mut tga_image_type = s.get8() as i32;
    let mut tga_is_rle = false;
    let tga_palette_start = s.get16le();
    let tga_palette_len = s.get16le();
    let tga_palette_bits = s.get8() as i32;
    let _tga_x_origin = s.get16le(); // @TODO
    let _tga_y_origin = s.get16le(); // @TODO
    let tga_width = s.get16le();
    let tga_height = s.get16le();
    let tga_bits_per_pixel = s.get8() as i32;
    let mut tga_rgb16 = false;
    let mut tga_inverted = s.get8() as i32;
    // int tga_alpha_bits = tga_inverted & 15; // the 4 lowest bits - unused (useless?)
    let mut raw_data = [0u8; 4];
    let mut rle_count = 0;
    let mut rle_repeating = 0;
    let mut read_next_pixel;

    if tga_height > STBI_MAX_DIMENSIONS as i32 {
        return err("Very large image (corrupt?)");
    }
    if tga_width > STBI_MAX_DIMENSIONS as i32 {
        return err("Very large image (corrupt?)");
    }

    //   do a tiny bit of precessing
    if tga_image_type >= 8 {
        tga_image_type -= 8;
        tga_is_rle = true;
    }
    tga_inverted = 1 - ((tga_inverted >> 5) & 1);

    //   If I'm paletted, then I'll use the number of bits from the palette
    let tga_comp = if tga_indexed != 0 {
        tga_get_comp(tga_palette_bits, false, Some(&mut tga_rgb16))
    } else {
        tga_get_comp(
            tga_bits_per_pixel,
            tga_image_type == 3,
            Some(&mut tga_rgb16),
        )
    };

    if tga_comp == 0 {
        // shouldn't really happen, stbi__tga_test() should have ensured basic consistency
        return err("Can't find out TGA pixelformat");
    }

    if !mad3sizes_valid(tga_width, tga_height, tga_comp, 0) {
        return err("Corrupt TGA");
    }

    /* FIXME (upstream): C leaves the pixels uninitialized, and the rows
    of an uncompressed image that stbi__getn() can't read whole keep
    whatever the allocation held; they are zero here. */
    let Some(mut tga_data) = malloc_mad3(tga_width, tga_height, tga_comp, 0) else {
        return err("Out of memory");
    };

    // skip to the data's starting position (offset usually = 0)
    s.skip(tga_offset);

    let comp = tga_comp as usize;
    if tga_indexed == 0 && !tga_is_rle && !tga_rgb16 {
        let row_len = (tga_width * tga_comp) as usize;
        for i in 0..tga_height {
            let row = if tga_inverted != 0 {
                tga_height - i - 1
            } else {
                i
            } as usize;
            s.getn(&mut tga_data[row * row_len..(row + 1) * row_len]);
        }
    } else {
        let mut tga_palette = Vec::new();
        //   do I need to load a palette?
        if tga_indexed != 0 {
            if tga_palette_len == 0 {
                /* you have to have at least one entry! */
                return err("Corrupt TGA");
            }

            //   any data to skip? (offset usually = 0)
            s.skip(tga_palette_start);
            //   load the palette
            let Some(p) = malloc_mad2(tga_palette_len, tga_comp, 0) else {
                return err("Out of memory");
            };
            tga_palette = p;
            if tga_rgb16 {
                // (STBI_ASSERT(tga_comp == STBI_rgb))
                for pal_entry in tga_palette.chunks_exact_mut(comp) {
                    tga_read_rgb16(s, pal_entry);
                }
            } else if !s.getn(&mut tga_palette) {
                return err("Corrupt TGA");
            }
        }
        //   load the data
        for px in tga_data.chunks_exact_mut(comp) {
            //   if I'm in RLE mode, do I need to get a RLE stbi__pngchunk?
            if tga_is_rle {
                if rle_count == 0 {
                    //   yep, get the next byte as a RLE command
                    let rle_cmd = s.get8() as i32;
                    rle_count = 1 + (rle_cmd & 127);
                    rle_repeating = rle_cmd >> 7;
                    read_next_pixel = true;
                } else {
                    read_next_pixel = rle_repeating == 0;
                }
            } else {
                read_next_pixel = true;
            }
            /* (the C keeps read_next_pixel across pixels; it is cleared
            after each read, so a repeated pixel doesn't set it) */
            //   OK, if I need to read a pixel, do it now
            if read_next_pixel {
                //   load however much data we did have
                if tga_indexed != 0 {
                    // read in index, then perform the lookup
                    let mut pal_idx = if tga_bits_per_pixel == 8 {
                        s.get8() as i32
                    } else {
                        s.get16le()
                    };
                    if pal_idx >= tga_palette_len {
                        // invalid index
                        pal_idx = 0;
                    }
                    let pal_idx = (pal_idx * tga_comp) as usize;
                    raw_data[..comp].copy_from_slice(&tga_palette[pal_idx..pal_idx + comp]);
                } else if tga_rgb16 {
                    // (STBI_ASSERT(tga_comp == STBI_rgb))
                    tga_read_rgb16(s, &mut raw_data);
                } else {
                    //   read in the data raw
                    for r in raw_data.iter_mut().take(comp) {
                        *r = s.get8();
                    }
                }
                //   clear the reading flag for the next pixel
            } // end of reading a pixel

            // copy data
            px.copy_from_slice(&raw_data[..comp]);

            //   in case we're in RLE mode, keep counting down
            rle_count -= 1;
        }
        //   do I need to invert the image?
        if tga_inverted != 0 {
            let row = (tga_width * tga_comp) as usize;
            let h = tga_height as usize;
            let mut j = 0;
            while j * 2 < h {
                // (the middle row of an odd height swaps with itself)
                if h - 1 - j != j {
                    let (top, bottom) = tga_data.split_at_mut((h - 1 - j) * row);
                    top[j * row..(j + 1) * row].swap_with_slice(&mut bottom[..row]);
                }
                j += 1;
            }
        }
        //   clear my palette, if I had one
    }

    // swap RGB - if the source data was RGB16, it already is in the right order
    if tga_comp >= 3 && !tga_rgb16 {
        for px in tga_data.chunks_exact_mut(comp) {
            px.swap(0, 2);
        }
    }

    // convert to target component count
    if req_comp != 0 && req_comp != tga_comp {
        tga_data = convert_format8(
            tga_data,
            tga_comp,
            req_comp,
            tga_width as u32,
            tga_height as u32,
        )?;
    }

    //   OK, done
    Ok(Image {
        x: tga_width,
        y: tga_height,
        comp: tga_comp,
        data: Pixels::Bits8(tga_data),
    })
}
