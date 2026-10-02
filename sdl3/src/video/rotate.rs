// Rust translation of src/video/SDL_rotate.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// SDL_rotate.c: rotates 32bit or 8bit surfaces
//
// Shamelessly stolen from SDL_gfx by Andreas Schiffler. Original copyright follows:
//
// Copyright (C) 2001-2011  Andreas Schiffler
//
// This software is provided 'as-is', without any express or implied
// warranty. In no event will the authors be held liable for any damages
// arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
//    1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
//
//    2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
//
//    3. This notice may not be removed or altered from any source
//    distribution.
//
// Andreas Schiffler -- aschiffler at ferzkopp dot net

//! Rotation of 32-bit and colorkeyed 8-bit surfaces (from SDL_gfx).

use crate::video::blendmode::BlendMode;
use crate::video::pixels::{PackedLayout, PixelFormat};
use crate::video::rect::{FPoint, Rect};
use crate::video::surface::Surface;

/// Number of guard rows added to destination surfaces.
/// This is a simple but effective workaround for observed issues.
/// These rows allocate extra memory and are then hidden from the surface.
/// Rows are added to the end of destination surfaces when they are allocated.
/// This catches any potential overflows which seem to happen with
/// just the right src image dimensions and scale/rotation and can lead
/// to a situation where the program can segfault.
const GUARD_ROWS: i32 = 2;

/// Returns colorkey info for a surface. Translation of `get_colorkey()`.
fn get_colorkey(src: &Surface<'_>) -> u32 {
    src.color_key().unwrap_or(0)
}

/// rotate (sx, sy) by (angle, center) into (dx, dy)
fn rotate(mut sx: f64, mut sy: f64, sinangle: f64, cosangle: f64, center: &FPoint) -> (f64, f64) {
    sx -= center.x as f64;
    sy -= center.y as f64;

    let mut dx = cosangle * sx - sinangle * sy;
    let mut dy = sinangle * sx + cosangle * sy;

    dx += center.x as f64;
    dy += center.y as f64;
    (dx, dy)
}

/// `SDL_min()` / `SDL_max()` on doubles (`(x < y) ? x : y`).
fn c_min(x: f64, y: f64) -> f64 {
    if x < y {
        x
    } else {
        y
    }
}
fn c_max(x: f64, y: f64) -> f64 {
    if x > y {
        x
    } else {
        y
    }
}

/// Internal target surface sizing function for rotations with trig result
/// return: `(rect_dest, cangle, sangle)`. Translation of `SDLgfx_rotozoomSurfaceSizeTrig()`.
pub(crate) fn rotozoom_surface_size_trig(
    width: i32,
    height: i32,
    angle: f64,
    center: &FPoint,
) -> (Rect, f64, f64) {
    let radangle = angle * (std::f64::consts::PI / 180.0);
    let sinangle = radangle.sin();
    let cosangle = radangle.cos();

    /*
     * Determine destination width and height by rotating a source box, at pixel center
     */
    let (x0, y0) = rotate(0.5, 0.5, sinangle, cosangle, center);
    let (x1, y1) = rotate(width as f64 - 0.5, 0.5, sinangle, cosangle, center);
    let (x2, y2) = rotate(0.5, height as f64 - 0.5, sinangle, cosangle, center);
    let (x3, y3) = rotate(
        width as f64 - 0.5,
        height as f64 - 0.5,
        sinangle,
        cosangle,
        center,
    );

    let minx = c_min(c_min(x0, x1), c_min(x2, x3)).floor() as i32;
    let maxx = c_max(c_max(x0, x1), c_max(x2, x3)).ceil() as i32;

    let miny = c_min(c_min(y0, y1), c_min(y2, y3)).floor() as i32;
    let maxy = c_max(c_max(y0, y1), c_max(y2, y3)).ceil() as i32;

    let mut rect_dest = Rect::new(minx, miny, maxx - minx, maxy - miny);

    // reverse the angle because our rotations are clockwise
    let mut sangle = -sinangle;
    let mut cangle = cosangle;

    {
        // The trig code below gets the wrong size (due to FP inaccuracy?) when angle is a multiple of 90 degrees
        let mut angle90 = (angle / 90.0) as i32;
        if angle90 as f64 == angle / 90.0 {
            // if the angle is a multiple of 90 degrees
            angle90 %= 4;
            if angle90 < 0 {
                angle90 += 4; // 0:0 deg, 1:90 deg, 2:180 deg, 3:270 deg
            }

            if angle90 & 1 != 0 {
                rect_dest.w = height;
                rect_dest.h = width;
                cangle = 0.0;
                sangle = if angle90 == 1 { -1.0 } else { 1.0 }; // reversed because our rotations are clockwise
            } else {
                rect_dest.w = width;
                rect_dest.h = height;
                cangle = if angle90 == 0 { 1.0 } else { -1.0 };
                sangle = 0.0;
            }
        }
    }
    (rect_dest, cangle, sangle)
}

/// Computes source pointer X/Y increments for a rotation that's a multiple
/// of 90 degrees: `(sincx, sincy, signx, signy)`. Translation of `computeSourceIncrements90()`.
fn compute_source_increments_90(
    src: &Surface<'_>,
    mut bpp: i32,
    angle: i32,
    flipx: bool,
    flipy: bool,
) -> (i32, i32, i32, i32) {
    let pitch = if flipy { -src.pitch } else { src.pitch };
    if flipx {
        bpp = -bpp;
    }
    let (sincx, sincy, mut signx, mut signy);
    match angle {
        // 0:0 deg, 1:90 deg, 2:180 deg, 3:270 deg
        0 => {
            sincx = bpp;
            sincy = pitch - src.w * sincx;
            signx = 1;
            signy = 1;
        }
        1 => {
            sincx = -pitch;
            sincy = bpp - sincx * src.h;
            signx = 1;
            signy = -1;
        }
        2 => {
            sincx = -bpp;
            sincy = -src.w * sincx - pitch;
            signx = -1;
            signy = -1;
        }
        _ => {
            sincx = pitch;
            sincy = -sincx * src.h - bpp;
            signx = -1;
            signy = 1;
        }
    }
    if flipx {
        signx = -signx;
    }
    if flipy {
        signy = -signy;
    }
    (sincx, sincy, signx, signy)
}

/// Performs a relatively fast rotation/flip when the angle is a multiple of
/// 90 degrees. Translation of `TRANSFORM_SURFACE_90()` (`transformSurfaceRGBA90()`,
/// `transformSurfaceY90()`).
fn transform_surface_90(
    src: &Surface<'_>,
    dst: &mut Surface<'_>,
    size: usize,
    angle: i32,
    flipx: bool,
    flipy: bool,
) {
    let dincy = dst.pitch as isize - dst.w as isize * size as isize;
    let (sincx, sincy, signx, signy) =
        compute_source_increments_90(src, size as i32, angle, flipx, flipy);
    let Some(sp_buf) = src.pixels.bytes() else {
        return;
    };
    let (dw, dh) = (dst.w as usize, dst.h);
    let Some(dp_buf) = dst.pixels.bytes_mut() else {
        return;
    };

    let mut sp: isize = 0;
    let mut dp: isize = 0;
    if signx < 0 {
        sp += (src.w as isize - 1) * size as isize;
    }
    if signy < 0 {
        sp += (src.h as isize - 1) * src.pitch as isize;
    }

    for _ in 0..dh {
        if sincx == size as i32 {
            // if advancing src and dest equally, use SDL_memcpy
            let n = dw * size;
            dp_buf[dp as usize..dp as usize + n]
                .copy_from_slice(&sp_buf[sp as usize..sp as usize + n]);
            sp += n as isize;
            dp += n as isize;
        } else {
            for _ in 0..dw {
                let (s, d) = (sp as usize, dp as usize);
                dp_buf[d..d + size].copy_from_slice(&sp_buf[s..s + size]);
                sp += sincx as isize;
                dp += size as isize;
            }
        }
        sp += sincy as isize;
        dp += dincy;
    }
}

/// Internal 32 bit rotozoomer with optional anti-aliasing.
///
/// Rotates and zooms 32 bit RGBA/ABGR 'src' surface to 'dst' surface based on the control
/// parameters by scanning the destination surface and applying optionally anti-aliasing
/// by bilinear interpolation.
/// Assumes src and dst surfaces are of 32 bit depth.
/// Assumes dst surface was allocated with the correct dimensions.
///
/// Translation of `transformSurfaceRGBA()`.
#[allow(clippy::too_many_arguments)]
fn transform_surface_rgba(
    src: &Surface<'_>,
    dst: &mut Surface<'_>,
    isin: i32,
    icos: i32,
    flipx: bool,
    flipy: bool,
    smooth: bool,
    rect_dest: &Rect,
    center: &FPoint,
) {
    const FP_HALF: i32 = 1 << 15;

    /*
     * Variable setup
     */
    let sw = src.w - 1;
    let sh = src.h - 1;
    let gap = dst.pitch as usize - dst.w as usize * 4;
    let cx = (center.x as f64 * 65536.0) as i32;
    let cy = (center.y as f64 * 65536.0) as i32;
    let (src_w, src_h, src_pitch) = (src.w, src.h, src.pitch as usize);
    let Some(spx) = src.pixels.bytes() else {
        return;
    };
    let (dw, dh) = (dst.w, dst.h);
    let Some(dpx) = dst.pixels.bytes_mut() else {
        return;
    };
    let px = |x: i32, y: i32| -> [u8; 4] {
        let i = src_pitch * y as usize + 4 * x as usize;
        [spx[i], spx[i + 1], spx[i + 2], spx[i + 3]]
    };

    let mut pc = 0usize;
    for y in 0..dh {
        let src_x = rect_dest.x as f64 + 0.0 + 0.5 - center.x as f64;
        let src_y = rect_dest.y as f64 + y as f64 + 0.5 - center.y as f64;
        let mut sdx =
            ((icos as f64 * src_x - isin as f64 * src_y) + cx as f64 - FP_HALF as f64) as i32;
        let mut sdy =
            ((isin as f64 * src_x + icos as f64 * src_y) + cy as f64 - FP_HALF as f64) as i32;
        for _ in 0..dw {
            let mut dx = sdx >> 16;
            let mut dy = sdy >> 16;
            if smooth {
                /*
                 * Switch between interpolating and non-interpolating code
                 */
                if flipx {
                    dx = sw - dx;
                }
                if flipy {
                    dy = sh - dy;
                }
                if (dx > -1) && (dy > -1) && (dx < (src_w - 1)) && (dy < (src_h - 1)) {
                    let mut c00 = px(dx, dy);
                    let mut c01 = px(dx + 1, dy);
                    let mut c11 = px(dx + 1, dy + 1);
                    let mut c10 = px(dx, dy + 1);
                    if flipx {
                        std::mem::swap(&mut c00, &mut c01);
                        std::mem::swap(&mut c10, &mut c11);
                    }
                    if flipy {
                        std::mem::swap(&mut c00, &mut c10);
                        std::mem::swap(&mut c01, &mut c11);
                    }
                    /*
                     * Interpolate colors
                     */
                    let ex = sdx & 0xffff;
                    let ey = sdy & 0xffff;
                    for k in 0..4 {
                        let t1 =
                            ((((c01[k] as i32 - c00[k] as i32) * ex) >> 16) + c00[k] as i32) & 0xff;
                        let t2 =
                            ((((c11[k] as i32 - c10[k] as i32) * ex) >> 16) + c10[k] as i32) & 0xff;
                        dpx[pc + k] = ((((t2 - t1) * ey) >> 16) + t1) as u8;
                    }
                }
            } else if (dx as u32) < src_w as u32 && (dy as u32) < src_h as u32 {
                if flipx {
                    dx = sw - dx;
                }
                if flipy {
                    dy = sh - dy;
                }
                dpx[pc..pc + 4].copy_from_slice(&px(dx, dy));
            }
            sdx = sdx.wrapping_add(icos);
            sdy = sdy.wrapping_add(isin);
            pc += 4;
        }
        pc += gap;
    }
}

/// Rotates and zooms 8 bit palette/Y 'src' surface to 'dst' surface without smoothing.
///
/// Rotates and zooms 8 bit RGBA/ABGR 'src' surface to 'dst' surface based on the control
/// parameters by scanning the destination surface.
/// Assumes src and dst surfaces are of 8 bit depth.
/// Assumes dst surface was allocated with the correct dimensions.
///
/// Translation of `transformSurfaceY()`.
#[allow(clippy::too_many_arguments)]
fn transform_surface_y(
    src: &Surface<'_>,
    dst: &mut Surface<'_>,
    isin: i32,
    icos: i32,
    flipx: bool,
    flipy: bool,
    rect_dest: &Rect,
    center: &FPoint,
) {
    const FP_HALF: i32 = 1 << 15;

    /*
     * Variable setup
     */
    let sw = src.w - 1;
    let sh = src.h - 1;
    let gap = dst.pitch as usize - dst.w as usize;
    let cx = (center.x as f64 * 65536.0) as i32;
    let cy = (center.y as f64 * 65536.0) as i32;
    let key = (get_colorkey(src) & 0xff) as u8;
    let (src_w, src_h, src_pitch) = (src.w, src.h, src.pitch as usize);
    let Some(spx) = src.pixels.bytes() else {
        return;
    };
    let (dw, dh, dpitch) = (dst.w, dst.h, dst.pitch as usize);
    let Some(dpx) = dst.pixels.bytes_mut() else {
        return;
    };

    /*
     * Clear surface to colorkey
     */
    dpx[..dpitch * dh as usize].fill(key);
    /*
     * Iterate through destination surface
     */
    let mut pc = 0usize;
    for y in 0..dh {
        let src_x = rect_dest.x as f64 + 0.0 + 0.5 - center.x as f64;
        let src_y = rect_dest.y as f64 + y as f64 + 0.5 - center.y as f64;
        let mut sdx =
            ((icos as f64 * src_x - isin as f64 * src_y) + cx as f64 - FP_HALF as f64) as i32;
        let mut sdy =
            ((isin as f64 * src_x + icos as f64 * src_y) + cy as f64 - FP_HALF as f64) as i32;
        for _ in 0..dw {
            let mut dx = sdx >> 16;
            let mut dy = sdy >> 16;
            if (dx as u32) < src_w as u32 && (dy as u32) < src_h as u32 {
                if flipx {
                    dx = sw - dx;
                }
                if flipy {
                    dy = sh - dy;
                }
                dpx[pc] = spx[src_pitch * dy as usize + dx as usize];
            }
            sdx = sdx.wrapping_add(icos);
            sdy = sdy.wrapping_add(isin);
            pc += 1;
        }
        pc += gap;
    }
}

/// Rotates and zooms a surface with different horizontal and vertival scaling factors and optional anti-aliasing.
///
/// Rotates a 32-bit or 8-bit 'src' surface to newly created 'dst' surface.
/// 'angle' is the rotation in degrees, 'center' the rotation center. If 'smooth' is set
/// then the destination 32-bit surface is anti-aliased. 8-bit surfaces must have a colorkey. 32-bit
/// surfaces must have a 8888 layout with red, green, blue and alpha masks (any ordering goes).
/// The blend mode of the 'src' surface has some effects on generation of the 'dst' surface: The NONE
/// mode will set the BLEND mode on the 'dst' surface. The MOD mode either generates a white 'dst'
/// surface and sets the colorkey or fills the it with the colorkey before copying the pixels.
/// When using the NONE and MOD modes, color and alpha modulation must be applied before using this function.
///
/// Translation of `SDLgfx_rotateSurface()`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rotate_surface(
    src: &mut Surface<'_>,
    angle: f64,
    smooth: bool,
    flipx: bool,
    flipy: bool,
    rect_dest: &Rect,
    cangle: f64,
    sangle: f64,
    center: &FPoint,
) -> Option<Surface<'static>> {
    let colorkey = src.color_key();
    let color_key_available = colorkey.is_some();
    let mut colorkey = colorkey.unwrap_or(0);
    // This function requires a 32-bit surface or 8-bit surface with a colorkey
    let is8bit = src.format == PixelFormat::INDEX8 && color_key_available;
    if !is8bit
        && !(src.format.bits_per_pixel() == 32 && src.format.pixel_layout() == PackedLayout::L8888)
    {
        return None;
    }

    // Calculate target factors from sine/cosine and zoom
    let sangleinv = sangle * 65536.0;
    let cangleinv = cangle * 65536.0;

    // Alloc space to completely contain the rotated surface
    let mut rz_dst = Surface::new(rect_dest.w, rect_dest.h + GUARD_ROWS, src.format).ok()?;
    if is8bit {
        // Target surface is 8 bit
        let _ = rz_dst.set_palette(src.palette.clone());
    }
    // (else: Target surface is 32 bit with source RGBA ordering)

    // Adjust for guard rows (the clip rectangle keeps them, as upstream's does)
    rz_dst.h = rect_dest.h;

    let mut blendmode = src.blend_mode();

    if color_key_available {
        // If available, the colorkey will be used to discard the pixels that are outside of the rotated area.
        let _ = rz_dst.set_color_key(Some(colorkey));
        let _ = rz_dst.fill_rect(None, colorkey);
    } else if blendmode == BlendMode::NONE {
        blendmode = BlendMode::BLEND;
    } else if blendmode == BlendMode::MOD || blendmode == BlendMode::MUL {
        /* Without a colorkey, the target texture has to be white for the MOD and MUL blend mode so
         * that the pixels outside the rotated area don't affect the destination surface.
         */
        colorkey = rz_dst.map_rgba(255, 255, 255, 0);
        let _ = rz_dst.fill_rect(None, colorkey);
        /* Setting a white colorkey for the destination surface makes the final blit discard
         * all pixels outside of the rotated area. This doesn't interfere with anything because
         * white pixels are already a no-op and the MOD blend mode does not interact with alpha.
         */
        let _ = rz_dst.set_color_key(Some(colorkey));
    }

    let _ = rz_dst.set_blend_mode(blendmode);

    // Lock source surface
    let must_lock = src.must_lock();
    if must_lock && src.lock_raw().is_err() {
        return None;
    }

    /* check if the rotation is a multiple of 90 degrees so we can take a fast path and also somewhat reduce
     * the off-by-one problem in transformSurfaceRGBA that expresses itself when the rotation is near
     * multiples of 90 degrees.
     */
    let mut angle90 = (angle / 90.0) as i32;
    if angle90 as f64 == angle / 90.0 {
        angle90 %= 4;
        if angle90 < 0 {
            angle90 += 4; // 0:0 deg, 1:90 deg, 2:180 deg, 3:270 deg
        }
    } else {
        angle90 = -1;
    }

    if is8bit {
        // Call the 8-bit transformation routine to do the rotation
        if angle90 >= 0 {
            transform_surface_90(src, &mut rz_dst, 1, angle90, flipx, flipy);
        } else {
            transform_surface_y(
                src,
                &mut rz_dst,
                sangleinv as i32,
                cangleinv as i32,
                flipx,
                flipy,
                rect_dest,
                center,
            );
        }
    } else {
        // Call the 32-bit transformation routine to do the rotation
        if angle90 >= 0 {
            transform_surface_90(src, &mut rz_dst, 4, angle90, flipx, flipy);
        } else {
            transform_surface_rgba(
                src,
                &mut rz_dst,
                sangleinv as i32,
                cangleinv as i32,
                flipx,
                flipy,
                smooth,
                rect_dest,
                center,
            );
        }
    }

    // Unlock source surface
    if must_lock {
        src.unlock_raw();
    }

    // Return rotated surface
    Some(rz_dst)
}
