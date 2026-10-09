// Rust translation of src/sdf/ftbsdf.c from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it).
// Copyright (C) 2020-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// Written by Anuj Verma.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Signed Distance Field support for bitmap fonts (body only).
//!
//! Translation note: the distance map is indexed by position instead of
//! pointer, and the source bitmap's pixels are read with bounds checks
//! (a pixel outside of the buffer reads as 0).

use super::super::base::ftcalc::{ft_div_fix, ft_mul_fix, ft_vector_norm_len};
use super::super::ftimage::{FtRasterFuncs, FtRasterParams, FtRasterSource, SdfRasterExt};
use super::super::fttypes::*;
use super::ftsdfcommon::*;

/**************************************************************************
 *
 * A brief technical overview of how the BSDF rasterizer works
 * -----------------------------------------------------------
 *
 * [Notes]:
 *   * SDF stands for Signed Distance Field everywhere.
 *
 *   * BSDF stands for Bitmap to Signed Distance Field rasterizer.
 *
 *   * This renderer converts rasterized bitmaps to SDF.  There is another
 *     renderer called 'sdf', which generates SDF directly from outlines;
 *     see file `ftsdf.c` for more.
 *
 *   * The idea of generating SDF from bitmaps is taken from two research
 *     papers, where one is dependent on the other:
 *
 *     - Per-Erik Danielsson: Euclidean Distance Mapping
 *       http://webstaff.itn.liu.se/~stegu/JFA/Danielsson.pdf
 *
 *       From this paper we use the eight-point sequential Euclidean
 *       distance mapping (8SED).  This is the heart of the process used
 *       in this rasterizer.
 *
 *     - Stefan Gustavson, Robin Strand: Anti-aliased Euclidean distance transform.
 *       http://weber.itn.liu.se/~stegu/aadist/edtaa_preprint.pdf
 *
 *       The original 8SED algorithm discards the pixels' alpha values,
 *       which can contain information about the actual outline of the
 *       glyph.  This paper takes advantage of those alpha values and
 *       approximates outline pretty accurately.
 *
 *   * This rasterizer also works for monochrome bitmaps.  However, the
 *     result is not as accurate since we don't have any way to
 *     approximate outlines from binary bitmaps.
 *
 * ========================================================================
 *
 * Generating SDF from bitmap is done in several steps.
 *
 * (1) The only information we have is the bitmap itself.  It can
 *     be monochrome or anti-aliased.  If it is anti-aliased, pixel values
 *     are nothing but coverage values.  These coverage values can be used
 *     to extract information about the outline of the image.  For
 *     example, if the pixel's alpha value is 0.5, then we can safely
 *     assume that the outline passes through the center of the pixel.
 *
 * (2) Find edge pixels in the bitmap (see `bsdf_is_edge` for more).  For
 *     all edge pixels we use the Anti-aliased Euclidean distance
 *     transform algorithm and compute approximate edge distances (see
 *     `compute_edge_distance` and/or the second paper for more).
 *
 * (3) Now that we have computed approximate distances for edge pixels we
 *     use the 8SED algorithm to basically sweep the entire bitmap and
 *     compute distances for the rest of the pixels.  (Since the algorithm
 *     is pretty convoluted it is only explained briefly in a comment to
 *     function `edt8`.  To see the actual algorithm refer to the first
 *     paper.)
 *
 * (4) Finally, compute the sign for each pixel.  This is done in function
 *     `finalize_sdf`.  The basic idea is that if a pixel's original
 *     alpha/coverage value is greater than 0.5 then it is 'inside' (and
 *     'outside' otherwise).
 *
 * Pseudo Code:
 *
 * ```
 * b  = source bitmap;
 * t  = target bitmap;
 * dm = list of distances; // dimension equal to b
 *
 * foreach grid_point (x, y) in b:
 * {
 *   if (is_edge(x, y)):
 *     dm = approximate_edge_distance(b, x, y);
 *
 *   // do the 8SED on the distances
 *   edt8(dm);
 *
 *   // determine the signs
 *   determine_signs(dm):
 *
 *   // copy SDF data to the target bitmap
 *   copy(dm to t);
 * }
 *
 */

/**************************************************************************
 *
 * useful macros
 *
 */

/// 1 in 16.16
const ONE: Ft16D16 = 65536;

/**************************************************************************
 *
 * structs
 *
 */

/* BSDF_TRaster: the raster holds no state (see `FtRasterFuncs`) */

/// `ED`: Euclidean distance.  It gets used for Euclidean distance
/// transforms; it can also be interpreted as an edge distance.
#[derive(Debug, Clone, Copy, Default)]
struct Ed {
    /// Vector length of the `prox` parameter.  Can be squared or absolute
    /// depending on the `USE_SQUARED_DISTANCES` macro defined in file
    /// `ftsdfcommon.h`.
    dist: Ft16D16,
    /// Vector to the nearest edge.  Can also be interpreted as shortest
    /// distance of a point.
    prox: Ft16D16Vec,
    /// Alpha value of the original bitmap from which we generate SDF.
    /// Needed for computing the gradient and determining the proper sign
    /// of a pixel.
    alpha: FtByte,
}

/// `BSDF_Worker`: A convenience struct that is passed to functions while
/// generating SDF; most of those functions require the same parameters.
#[derive(Debug)]
struct BsdfWorker {
    /// A one-dimensional array that gets interpreted as two-dimensional
    /// one.  It contains the Euclidean distances of all points of the
    /// bitmap.
    distance_map: Vec<Ed>,

    /// Width of the above `distance_map`.
    width: FtInt,
    /// Number of rows in the above `distance_map`.
    rows: FtInt,

    /// Internal parameters and properties required by the rasterizer.  See
    /// file `ftsdf.h` for more.
    params: SdfRasterExt,
}

/**************************************************************************
 *
 * initializer
 *
 */

/* zero_ed: `Ed::default()' */

/**************************************************************************
 *
 * rasterizer functions
 *
 */

/// `bsdf_is_edge`: Check whether a pixel is an edge pixel, i.e., whether
/// it is surrounded by a completely black pixel (zero alpha), and the
/// current pixel is not a completely black pixel.
///
/// `dm` is the distance map and `index` the position of the current
/// pixel, at (`x`, `y`) in a `w` x `r` map.
fn bsdf_is_edge(
    dm: &[Ed],
    index: usize, /* distance map              */
    x: FtInt,     /* x index of point to check */
    y: FtInt,     /* y index of point to check */
    w: FtInt,     /* width                     */
    r: FtInt,     /* rows                      */
) -> bool {
    let mut is_edge = false;
    let mut num_neighbors: FtInt = 0;

    'done: {
        if dm[index].alpha == 0 {
            break 'done;
        }

        if dm[index].alpha > 0 && dm[index].alpha < 255 {
            is_edge = true;
            break 'done;
        }

        /* CHECK_NEIGHBOR */
        let mut check_neighbor = |x_offset: FtInt, y_offset: FtInt| -> bool {
            if x + x_offset >= 0 && x + x_offset < w && y + y_offset >= 0 && y + y_offset < r {
                num_neighbors += 1;

                let to_check = (index as isize + (y_offset * w + x_offset) as isize) as usize;
                if dm[to_check].alpha == 0 {
                    return true;
                }
            }
            false
        };

        /* up, down, left, right, up left, up right, down left, down right */
        for (x_offset, y_offset) in [
            (0, -1),
            (0, 1),
            (-1, 0),
            (1, 0),
            (-1, -1),
            (1, -1),
            (-1, 1),
            (1, 1),
        ] {
            if check_neighbor(x_offset, y_offset) {
                is_edge = true;
                break 'done;
            }
        }

        if num_neighbors != 8 {
            is_edge = true;
        }
    }

    /* Done: */
    is_edge
}

/// `compute_edge_distance`: Approximate the outline and compute the
/// distance from `current` to the approximated outline.
///
/// `dm` is the array of Euclidean distances, `current` the position for
/// which the distance is to be caculated, at (`x`, `y`) in a `w` x `r`
/// array.  The result is a vector pointing to the approximate edge
/// distance.
///
/// This is a computationally expensive function.  Try to reduce the
/// number of calls to this function.  Moreover, this must only be used
/// for edge pixel positions.
fn compute_edge_distance(
    dm: &[Ed],
    current: usize,
    x: FtInt,
    y: FtInt,
    w: FtInt,
    r: FtInt,
) -> Ft16D16Vec {
    /*
     * This function, based on the paper presented by Stefan Gustavson and
     * Robin Strand, gets used to approximate edge distances from
     * anti-aliased bitmaps.
     *
     * The algorithm is as follows.
     *
     * (1) In anti-aliased images, the pixel's alpha value is the coverage
     *     of the pixel by the outline.  For example, if the alpha value is
     *     0.5f we can assume that the outline passes through the center of
     *     the pixel.
     *
     * (2) For this reason we can use that alpha value to approximate the real
     *     distance of the pixel to edge pretty accurately.  A simple
     *     approximation is `(0.5f - alpha)`, assuming that the outline is
     *     parallel to the x or y~axis.  However, in this algorithm we use a
     *     different approximation which is quite accurate even for
     *     non-axis-aligned edges.
     *
     * (3) The only remaining piece of information that we cannot
     *     approximate directly from the alpha is the direction of the edge.
     *     This is where we use Sobel's operator to compute the gradient of
     *     the pixel.  The gradient give us a pretty good approximation of
     *     the edge direction.  We use a 3x3 kernel filter to compute the
     *     gradient.
     *
     * (4) After the above two steps we have both the direction and the
     *     distance to the edge which is used to generate the Signed
     *     Distance Field.
     *
     * References:
     *
     * - Anti-Aliased Euclidean Distance Transform:
     *     http://weber.itn.liu.se/~stegu/aadist/edtaa_preprint.pdf
     * - Sobel Operator:
     *     https://en.wikipedia.org/wiki/Sobel_operator
     */

    let mut g: Ft16D16Vec = FtVector { x: 0, y: 0 };
    let dist: Ft16D16;
    let current_alpha: Ft16D16;
    let a1: Ft16D16;
    let temp: Ft16D16;
    let mut gx: Ft16D16;
    let mut gy: Ft16D16;
    let mut alphas = [0 as Ft16D16; 9];

    /* Since our spread cannot be 0, this condition */
    /* can never be true.                           */
    if x <= 0 || x >= w - 1 || y <= 0 || y >= r - 1 {
        return g;
    }

    let at = |offset: FtInt| -> Ft16D16 {
        256 * dm[(current as isize + offset as isize) as usize].alpha as Ft16D16
    };

    /* initialize the alphas */
    alphas[0] = at(-w - 1);
    alphas[1] = at(-w);
    alphas[2] = at(-w + 1);
    alphas[3] = at(-1);
    alphas[4] = at(0);
    alphas[5] = at(1);
    alphas[6] = at(w - 1);
    alphas[7] = at(w);
    alphas[8] = at(w + 1);

    current_alpha = alphas[4];

    /* Compute the gradient using the Sobel operator. */
    /* In this case we use the following 3x3 filters: */
    /*                                                */
    /* For x: |   -1     0   -1    |                  */
    /*        | -root(2) 0 root(2) |                  */
    /*        |    -1    0    1    |                  */
    /*                                                */
    /* For y: |   -1 -root(2) -1   |                  */
    /*        |    0    0      0   |                  */
    /*        |    1  root(2)  1   |                  */
    /*                                                */
    /* [Note]: 92681 is root(2) in 16.16 format.      */
    g.x = -(alphas[0] as FtPos) - ft_mul_fix(alphas[3] as FtLong, 92681) - alphas[6] as FtPos
        + alphas[2] as FtPos
        + ft_mul_fix(alphas[5] as FtLong, 92681)
        + alphas[8] as FtPos;

    g.y = -(alphas[0] as FtPos) - ft_mul_fix(alphas[1] as FtLong, 92681) - alphas[2] as FtPos
        + alphas[6] as FtPos
        + ft_mul_fix(alphas[7] as FtLong, 92681)
        + alphas[8] as FtPos;

    ft_vector_norm_len(&mut g);

    /* The gradient gives us the direction of the    */
    /* edge for the current pixel.  Once we have the */
    /* approximate direction of the edge, we can     */
    /* approximate the edge distance much better.    */

    if g.x == 0 || g.y == 0 {
        dist = ONE / 2 - alphas[4];
    } else {
        gx = g.x as Ft16D16;
        gy = g.y as Ft16D16;

        gx = gx.wrapping_abs();
        gy = gy.wrapping_abs();

        if gx < gy {
            temp = gx;
            gx = gy;
            gy = temp;
        }

        a1 = (ft_div_fix(gy as FtLong, gx as FtLong) / 2) as Ft16D16;

        if current_alpha < a1 {
            dist = ((gx.wrapping_add(gy)) / 2).wrapping_sub(square_root(
                (2 * ft_mul_fix(
                    gx as FtLong,
                    ft_mul_fix(gy as FtLong, current_alpha as FtLong),
                )) as Ft16D16,
            ));
        } else if current_alpha < (ONE - a1) {
            dist = ft_mul_fix((ONE / 2 - current_alpha) as FtLong, gx as FtLong) as Ft16D16;
        } else {
            dist = (gx.wrapping_add(gy).wrapping_neg() / 2).wrapping_add(square_root(
                (2 * ft_mul_fix(
                    gx as FtLong,
                    ft_mul_fix(gy as FtLong, (ONE - current_alpha) as FtLong),
                )) as Ft16D16,
            ));
        }
    }

    g.x = ft_mul_fix(g.x, dist as FtLong);
    g.y = ft_mul_fix(g.y, dist as FtLong);

    g
}

/// `bsdf_approximate_edge`: Loops over all the pixels and call
/// `compute_edge_distance` only for edge pixels.  This maked the process
/// a lot faster since `compute_edge_distance` uses functions such as
/// `FT_Vector_NormLen', which are quite slow.
///
/// The function directly manipulates `worker->distance_map`.
fn bsdf_approximate_edge(worker: &mut BsdfWorker) -> FtResult<()> {
    let ed = &mut worker.distance_map;

    for j in 0..worker.rows {
        for i in 0..worker.width {
            let index = (j * worker.width + i) as usize;

            if bsdf_is_edge(ed, index, i, j, worker.width, worker.rows) {
                /* approximate the edge distance for edge pixels */
                ed[index].prox = compute_edge_distance(ed, index, i, j, worker.width, worker.rows);
                ed[index].dist = vector_length_16d16(&ed[index].prox) as Ft16D16;
            } else {
                /* for non-edge pixels assign far away distances */
                ed[index].dist = 400 * ONE;
                ed[index].prox.x = (200 * ONE) as FtPos;
                ed[index].prox.y = (200 * ONE) as FtPos;
            }
        }
    }

    Ok(())
}

/// `bsdf_init_distance_map`: Initialize the distance map according to the
/// '8-point sequential Euclidean distance mapping' (8SED) algorithm.
/// Basically it copies the `source` bitmap alpha values to the
/// `distance_map->alpha` parameter of `worker`.
fn bsdf_init_distance_map(source: &FtBitmap, worker: &mut BsdfWorker) -> FtResult<()> {
    let mut x_diff: FtInt;
    let mut y_diff: FtInt;

    /* Because of the way we convert a bitmap to SDF, */
    /* i.e., aligning the source to the center of the */
    /* target, the target's width and rows must be    */
    /* checked before copying.                        */
    if worker.width < source.width as FtInt || worker.rows < source.rows as FtInt {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* check pixel mode */
    if source.pixel_mode == FT_PIXEL_MODE_NONE {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* Calculate the width and row differences */
    /* between target and source.              */
    x_diff = worker.width - source.width as FtInt;
    y_diff = worker.rows - source.rows as FtInt;

    x_diff /= 2;
    y_diff /= 2;

    let t = &mut worker.distance_map;
    let s = &source.buffer;
    let pixel_at = |i: FtInt| -> FtByte {
        if i < 0 {
            0
        } else {
            s.get(i as usize).copied().unwrap_or(0)
        }
    };

    /* For now we only support pixel mode `FT_PIXEL_MODE_MONO`  */
    /* and `FT_PIXEL_MODE_GRAY`.  More will be added later.     */
    /*                                                          */
    /* [NOTE]: We can also use @FT_Bitmap_Convert to convert    */
    /*         bitmap to 8bpp.  To avoid extra allocation and   */
    /*         since the target bitmap can be 16bpp we manually */
    /*         convert the source bitmap to the desired bpp.    */

    match source.pixel_mode {
        FT_PIXEL_MODE_MONO => {
            let t_width = worker.width;
            let t_rows = worker.rows;
            let s_width = source.width as FtInt;
            let s_rows = source.rows as FtInt;

            for t_j in 0..t_rows {
                for t_i in 0..t_width {
                    let t_index = (t_j * t_width + t_i) as usize;
                    let s_index: FtInt;

                    t[t_index] = Ed::default();

                    let s_i = t_i - x_diff;
                    let s_j = t_j - y_diff;

                    /* Assign 0 to padding similar to */
                    /* the source bitmap.             */
                    if s_i < 0 || s_i >= s_width || s_j < 0 || s_j >= s_rows {
                        continue;
                    }

                    if worker.params.flip_y {
                        s_index = (s_rows - s_j - 1).wrapping_mul(source.pitch);
                    } else {
                        s_index = s_j.wrapping_mul(source.pitch);
                    }

                    let div = s_index.wrapping_add(s_i / 8);
                    let modulo = 7 - s_i % 8;

                    let pixel = pixel_at(div);
                    let byte = (1 << modulo) as FtByte;

                    t[t_index].alpha = if pixel & byte != 0 { 255 } else { 0 };
                }
            }
        }

        FT_PIXEL_MODE_GRAY => {
            let t_width = worker.width;
            let t_rows = worker.rows;
            let s_width = source.width as FtInt;
            let s_rows = source.rows as FtInt;

            /* loop over all pixels and assign pixel values from source */
            for t_j in 0..t_rows {
                for t_i in 0..t_width {
                    let t_index = (t_j * t_width + t_i) as usize;
                    let s_index: FtInt;

                    t[t_index] = Ed::default();

                    let s_i = t_i - x_diff;
                    let s_j = t_j - y_diff;

                    /* Assign 0 to padding similar to */
                    /* the source bitmap.             */
                    if s_i < 0 || s_i >= s_width || s_j < 0 || s_j >= s_rows {
                        continue;
                    }

                    if worker.params.flip_y {
                        s_index = (s_rows - s_j - 1) * s_width + s_i;
                    } else {
                        s_index = s_j * s_width + s_i;
                    }

                    /* simply copy the alpha values */
                    t[t_index].alpha = pixel_at(s_index);
                }
            }
        }

        _ => {
            return Err(FT_ERR_UNIMPLEMENTED_FEATURE);
        }
    }

    Ok(())
}

/// `compare_neighbor`: Compare neighbor pixel (which is defined by the
/// offset) and update `current` distance if the new distance is shorter
/// than the original.
///
/// `dm` is the array of distances, `current` the position whose neighbor
/// is to be checked, `width` the width of the array.
fn compare_neighbor(dm: &mut [Ed], current: usize, x_offset: FtInt, y_offset: FtInt, width: FtInt) {
    let mut dist: Ft16D16;
    let mut dist_vec: Ft16D16Vec;

    let to_check = &dm[(current as isize + (y_offset * width + x_offset) as isize) as usize];

    /*
     * While checking for the nearest point we first approximate the
     * distance of `current` by adding the deviation (which is sqrt(2) at
     * most).  Only if the new value is less than the current value we
     * calculate the actual distances using `FT_Vector_Length`.  This last
     * step can be omitted by using squared distances.
     */

    /*
     * Approximate the distance.  We subtract 1 to avoid precision errors,
     * which could happen because the two directions can be opposite.
     */
    dist = to_check.dist.wrapping_sub(ONE);

    if dist < dm[current].dist {
        dist_vec = to_check.prox;

        dist_vec.x = dist_vec.x.wrapping_add((x_offset * ONE) as FtPos);
        dist_vec.y = dist_vec.y.wrapping_add((y_offset * ONE) as FtPos);
        dist = vector_length_16d16(&dist_vec) as Ft16D16;

        if dist < dm[current].dist {
            dm[current].dist = dist;
            dm[current].prox = dist_vec;
        }
    }
}

/// `first_pass`: First pass of the 8SED algorithm.  Loop over the bitmap
/// from top to bottom and scan each row left to right, updating the
/// distances in `worker->distance_map`.
fn first_pass(worker: &mut BsdfWorker) {
    let dm = &mut worker.distance_map; /* distance map */
    let w = worker.width; /* width, rows  */
    let r = worker.rows;

    /* Start scanning from top to bottom and sweep each    */
    /* row back and forth comparing the distances of the   */
    /* neighborhood.  Leave the first row as it has no top */
    /* neighbor; it will be covered in the second scan of  */
    /* the image (from bottom to top).                     */
    for j in 1..r {
        /* Forward pass of rows (left -> right).  Leave the first  */
        /* column, which gets covered in the backward pass.        */
        for i in 1..w - 1 {
            let current = (j * w + i) as usize;

            /* left-up */
            compare_neighbor(dm, current, -1, -1, w);
            /* up */
            compare_neighbor(dm, current, 0, -1, w);
            /* up-right */
            compare_neighbor(dm, current, 1, -1, w);
            /* left */
            compare_neighbor(dm, current, -1, 0, w);
        }

        /* Backward pass of rows (right -> left).  Leave the last */
        /* column, which was already covered in the forward pass. */
        let mut i = w - 2;
        while i >= 0 {
            let current = (j * w + i) as usize;

            /* right */
            compare_neighbor(dm, current, 1, 0, w);
            i -= 1;
        }
    }
}

/// `second_pass`: Second pass of the 8SED algorithm.  Loop over the bitmap
/// from bottom to top and scan each row left to right, updating the
/// distances in `worker->distance_map`.
fn second_pass(worker: &mut BsdfWorker) {
    let dm = &mut worker.distance_map; /* distance map */
    let w = worker.width; /* width, rows  */
    let r = worker.rows;

    /* Start scanning from bottom to top and sweep each    */
    /* row back and forth comparing the distances of the   */
    /* neighborhood.  Leave the last row as it has no down */
    /* neighbor; it is already covered in the first scan   */
    /* of the image (from top to bottom).                  */
    let mut j = r - 2;
    while j >= 0 {
        /* Forward pass of rows (left -> right).  Leave the first */
        /* column, which gets covered in the backward pass.       */
        for i in 1..w - 1 {
            let current = (j * w + i) as usize;

            /* left-up */
            compare_neighbor(dm, current, -1, 1, w);
            /* up */
            compare_neighbor(dm, current, 0, 1, w);
            /* up-right */
            compare_neighbor(dm, current, 1, 1, w);
            /* left */
            compare_neighbor(dm, current, -1, 0, w);
        }

        /* Backward pass of rows (right -> left).  Leave the last */
        /* column, which was already covered in the forward pass. */
        let mut i = w - 2;
        while i >= 0 {
            let current = (j * w + i) as usize;

            /* right */
            compare_neighbor(dm, current, 1, 0, w);
            i -= 1;
        }
        j -= 1;
    }
}

/// `edt8`: Compute the distance map of the a bitmap.  Execute both first
/// and second pass of the 8SED algorithm.
fn edt8(worker: &mut BsdfWorker) -> FtResult<()> {
    /* first scan of the image */
    first_pass(worker);

    /* second scan of the image */
    second_pass(worker);

    Ok(())
}

/// `finalize_sdf`: Copy the SDF data from `worker->distance_map` to the
/// `target` bitmap.  Also transform the data to output format, (which is
/// 6.10 fixed-point format at the moment).
fn finalize_sdf(worker: &BsdfWorker, target: &mut FtBitmap) -> FtResult<()> {
    let w = target.width as FtInt;
    let r = target.rows as FtInt;
    let t_buffer = &mut target.buffer;

    if w != worker.width || r != worker.rows {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let spread = ft_int_16d16(worker.params.spread as FtPos) as Ft16D16;

    /* (USE_SQUARED_DISTANCES is 0) */
    let sp_sq = ft_int_16d16(worker.params.spread as FtPos) as Ft16D16;

    for j in 0..r {
        for i in 0..w {
            let index = (j * w + i) as usize;
            let mut dist: Ft16D16;
            let final_dist: FtSdfFormat;
            let mut sign: i8;

            dist = worker.distance_map[index].dist;

            if dist < 0 || dist > sp_sq {
                dist = sp_sq;
            }

            /* We assume that if the pixel is inside a contour */
            /* its coverage value must be > 127.               */
            sign = if worker.distance_map[index].alpha < 127 {
                -1
            } else {
                1
            };

            /* flip the sign according to the property */
            if worker.params.flip_sign {
                sign = -sign;
            }

            /* concatenate from 16.16 to appropriate format */
            final_dist = map_fixed_to_sdf(dist.wrapping_mul(sign as Ft16D16), spread);

            if let Some(t) = t_buffer.get_mut(index) {
                *t = final_dist;
            }
        }
    }

    Ok(())
}

/**************************************************************************
 *
 * interface functions
 *
 */

/* bsdf_raster_new, bsdf_raster_reset, bsdf_raster_done: the raster */
/* holds no state                                                    */

/// `bsdf_raster_set_mode` (unused)
pub fn bsdf_raster_set_mode(_mode: FtULong) -> FtResult<()> {
    Ok(())
}

/// `bsdf_raster_render`: called while rendering through `FT_Render_Glyph`
fn bsdf_raster_render(source: FtRasterSource, params: &mut FtRasterParams) -> FtResult<()> {
    let sdf_params = params.sdf;

    /* check whether the flag is set */
    if params.flags != FT_RASTER_FLAG_SDF {
        return Err(FT_ERR_RASTER_CORRUPTED);
    }

    /* check source and target bitmap */
    let source = match source {
        FtRasterSource::Bitmap(b) => b,
        FtRasterSource::Outline(_) => return Err(FT_ERR_INVALID_ARGUMENT),
    };
    let Some(target) = params.target.as_deref_mut() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    /* check whether spread is set properly */
    if sdf_params.spread > MAX_SPREAD || sdf_params.spread < MIN_SPREAD {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* set up the worker */

    /* allocate the distance map */
    let n = (target.rows as usize).checked_mul(target.width as usize);
    let mut distance_map: Vec<Ed> = Vec::new();
    match n {
        Some(n) if distance_map.try_reserve_exact(n).is_ok() => {
            distance_map.resize(n, Ed::default());
        }
        _ => return Err(FT_ERR_OUT_OF_MEMORY),
    }

    let mut worker = BsdfWorker {
        distance_map,
        width: target.width as FtInt,
        rows: target.rows as FtInt,
        params: sdf_params,
    };

    bsdf_init_distance_map(source, &mut worker)?;
    bsdf_approximate_edge(&mut worker)?;
    edt8(&mut worker)?;
    finalize_sdf(&worker, target)?;

    /* (dropping the worker's distance map is FT_FREE) */
    Ok(())
}

/// `ft_bitmap_sdf_raster`
pub static FT_BITMAP_SDF_RASTER: FtRasterFuncs = FtRasterFuncs {
    glyph_format: FT_GLYPH_FORMAT_BITMAP,

    raster_render: bsdf_raster_render, /* raster_render   */
};
