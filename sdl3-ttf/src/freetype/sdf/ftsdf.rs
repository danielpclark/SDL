// Rust translation of src/sdf/ftsdf.c (and ftsdf.h) from FreeType (2.13.2,
// as SDL_ttf's external/freetype pins it).
// Copyright (C) 2020-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// Written by Anuj Verma.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! Signed Distance Field support for outline fonts (body).
//!
//! Translation notes: the singly linked lists of contours and edges are
//! `VecDeque`s in list order (C prepends to them, which is `push_front`),
//! and C's `long` and `int` arithmetic is kept with the types of its
//! variables (`FT_Pos` vectors, 32-bit 16.16 and 26.6 scalars), wrapping
//! where C would overflow.

use std::collections::VecDeque;

use super::super::base::ftcalc::{ft_div_fix, ft_mul_fix, ft_vector_norm_len};
use super::super::base::ftoutln::{
    ft_outline_decompose, ft_outline_get_orientation, FtOrientation, FtOutlineFuncs,
    FT_ORIENTATION_FILL_LEFT, FT_ORIENTATION_FILL_RIGHT,
};
use super::super::ftimage::{FtRasterFuncs, FtRasterParams, FtRasterSource};
use super::super::fttypes::*;
use super::ftsdfcommon::*;

/**************************************************************************
 *
 * A brief technical overview of how the SDF rasterizer works
 * ----------------------------------------------------------
 *
 * [Notes]:
 *   * SDF stands for Signed Distance Field everywhere.
 *
 *   * This renderer generates SDF directly from outlines.  There is
 *     another renderer called 'bsdf', which converts bitmaps to SDF; see
 *     file `ftbsdf.c` for more.
 *
 *   * The basic idea of generating the SDF is taken from Viktor Chlumsky's
 *     research paper.  The paper explains both single and multi-channel
 *     SDF, however, this implementation only generates single-channel SDF.
 *
 *       Chlumsky, Viktor: Shape Decomposition for Multi-channel Distance
 *       Fields.  Master's thesis.  Czech Technical University in Prague,
 *       Faculty of InformationTechnology, 2015.
 *
 *     For more information: https://github.com/Chlumsky/msdfgen
 *
 * ========================================================================
 *
 * Generating SDF from outlines is pretty straightforward.
 *
 * (1) We have a set of contours that make the outline of a shape/glyph.
 *     Each contour comprises of several edges, with three types of edges.
 *
 *     * line segments
 *     * conic Bezier curves
 *     * cubic Bezier curves
 *
 * (2) Apart from the outlines we also have a two-dimensional grid, namely
 *     the bitmap that is used to represent the final SDF data.
 *
 * (3) In order to generate SDF, our task is to find shortest signed
 *     distance from each grid point to the outline.  The 'signed
 *     distance' means that if the grid point is filled by any contour
 *     then its sign is positive, otherwise it is negative.  The pseudo
 *     code is as follows.
 *
 *     ```
 *     foreach grid_point (x, y):
 *     {
 *       int min_dist = INT_MAX;
 *
 *       foreach contour in outline:
 *       {
 *         foreach edge in contour:
 *         {
 *           // get shortest distance from point (x, y) to the edge
 *           d = get_min_dist(x, y, edge);
 *
 *           if (d < min_dist)
 *             min_dist = d;
 *         }
 *
 *         bitmap[x, y] = min_dist;
 *       }
 *     }
 *     ```
 *
 * (4) After running this algorithm the bitmap contains information about
 *     the shortest distance from each point to the outline of the shape.
 *     Of course, while this is the most straightforward way of generating
 *     SDF, we use various optimizations in our implementation.  See the
 *     `sdf_generate_*' functions in this file for all details.
 *
 *     The optimization currently used by default is subdivision; see
 *     function `sdf_generate_subdivision` for more.
 *
 *     Also, to see how we compute the shortest distance from a point to
 *     each type of edge, check out the `get_min_distance_*' functions.
 *
 */

/**************************************************************************
 *
 * definitions
 *
 */

/*
 * If set to 1, the rasterizer uses Newton-Raphson's method for finding
 * the shortest distance from a point to a conic curve.
 *
 * If set to 0, an analytical method gets used instead, which computes the
 * roots of a cubic polynomial to find the shortest distance.  However,
 * the analytical method can currently underflow; we thus use Newton's
 * method by default.
 */
/* USE_NEWTON_FOR_CONIC: 1 */

/*
 * The number of intervals a Bezier curve gets sampled and checked to find
 * the shortest distance.
 */
const MAX_NEWTON_DIVISIONS: FtInt32 = 4;

/*
 * The number of steps of Newton's iterations in each interval of the
 * Bezier curve.  Basically, we run Newton's approximation
 *
 *   x -= Q(t) / Q'(t)
 *
 * for each division to get the shortest distance.
 */
const MAX_NEWTON_STEPS: u16 = 4;

/*
 * The epsilon distance (in 16.16 fractional units) used for corner
 * resolving.  If the difference of two distances is less than this value
 * they will be checked for a corner if they are ambiguous.
 */
const CORNER_CHECK_EPSILON: Ft16D16 = 32;

/* (CG_DIMEN: #if 0) */

/**************************************************************************
 *
 * macros
 *
 */

/// `MUL_26D6`
#[inline]
fn mul_26d6(a: FtPos, b: FtPos) -> FtPos {
    a.wrapping_mul(b) / 64
}

/// `VEC_26D6_DOT`
#[inline]
fn vec_26d6_dot(p: &FtVector, q: &FtVector) -> FtPos {
    mul_26d6(p.x, q.x).wrapping_add(mul_26d6(p.y, q.y))
}

/**************************************************************************
 *
 * structures and enums
 *
 */

/* SDF_TRaster: the raster holds no state (see `FtRasterFuncs`) */

/// `SDF_Edge_Type`: Enumeration of all curve types present in fonts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SdfEdgeType {
    /// Undefined edge, simply used to initialize and detect errors.
    #[default]
    Undefined = 0,
    /// Line segment with start and end point.
    Line = 1,
    /// A conic/quadratic Bezier curve with start, end, and one control
    /// point.
    Conic = 2,
    /// A cubic Bezier curve with start, end, and two control points.
    Cubic = 3,
}

/// `SDF_Contour_Orientation`: Enumeration of all orientation values of a
/// contour.  We determine the orientation by calculating the area covered
/// by a contour.  Contrary to values returned by
/// `FT_Outline_Get_Orientation`, `SDF_Contour_Orientation` is independent
/// of the fill rule, which can be different for different font formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SdfContourOrientation {
    /// Undefined orientation, used for initialization and error detection.
    None = 0,
    /// Clockwise orientation (positive area covered).
    Cw = 1,
    /// Counter-clockwise orientation (negative area covered).
    Ccw = 2,
}

/// `SDF_Edge`: Represent an edge of a contour (`next` is the contour's
/// list).
#[derive(Debug, Clone, Copy, Default)]
struct SdfEdge {
    /// Start position of an edge.  Valid for all types of edges.
    start_pos: Ft26D6Vec,
    /// Etart position of an edge.  Valid for all types of edges.
    end_pos: Ft26D6Vec,
    /// A control point of the edge.  Valid only for `SDF_EDGE_CONIC` and
    /// `SDF_EDGE_CUBIC`.
    control_a: Ft26D6Vec,
    /// Another control point of the edge.  Valid only for
    /// `SDF_EDGE_CONIC`.
    control_b: Ft26D6Vec,
    /// Type of the edge, see `SDF_Edge_Type` for all possible edge types.
    edge_type: SdfEdgeType,
}

/// `SDF_Contour`: Represent a complete contour, which contains a list of
/// edges (`next` is the shape's list).
#[derive(Debug, Clone, Default)]
struct SdfContour {
    /// Contains the value of `end_pos' of the last edge in the list of
    /// edges.  Useful while decomposing the outline with
    /// `FT_Outline_Decompose`.
    last_pos: Ft26D6Vec,
    /// Linked list of all the edges that make the contour.
    edges: VecDeque<SdfEdge>,
}

/// `SDF_Shape`: Represent a complete shape, which is the decomposition of
/// `FT_Outline`.
#[derive(Debug, Default)]
struct SdfShape {
    /// Linked list of all the contours that make the shape.
    contours: VecDeque<SdfContour>,
}

/// `SDF_Signed_Distance`: Represent signed distance of a point, i.e., the
/// distance of the edge nearest to the point.
#[derive(Debug, Clone, Copy, Default)]
struct SdfSignedDistance {
    /// Distance of the point from the nearest edge.  Can be squared or
    /// absolute depending on the `USE_SQUARED_DISTANCES` macro defined in
    /// file `ftsdfcommon.h`.
    distance: Ft16D16,
    /// Cross product of the shortest distance vector (i.e., the vector
    /// from the point to the nearest edge) and the direction of the edge
    /// at the nearest point.  This is used to resolve ambiguities of
    /// `sign`.
    cross: Ft16D16,
    /// A value used to indicate whether the distance vector is outside or
    /// inside the contour corresponding to the edge.
    sign: i8,
}

/// `SDF_Params`: Yet another internal parameters required by the
/// rasterizer.
#[derive(Debug, Clone, Copy)]
struct SdfParams {
    /// This is not the `SDF_Contour_Orientation` value but
    /// `FT_Orientation`, which determines whether clockwise-oriented
    /// outlines are to be filled or counter-clockwise-oriented ones.
    orientation: FtOrientation,
    /// If set to true, flip the sign.  By default the points filled by the
    /// outline are positive.
    flip_sign: bool,
    /// If set to true the output bitmap is upside-down.  Can be useful
    /// because OpenGL and DirectX use different coordinate systems for
    /// textures.
    flip_y: bool,
    /// In the subdivision and bounding box optimization, the default
    /// outside sign is taken as -1.  This parameter can be used to modify
    /// that behaviour.  For example, while generating SDF for a single
    /// counter-clockwise contour, the outside sign should be 1.
    overload_sign: FtInt,
}

/**************************************************************************
 *
 * constants, initializer, and destructor
 *
 */

const ZERO_VECTOR: FtVector = FtVector { x: 0, y: 0 };

/* null_edge, null_contour, null_shape: the `Default`s */

const MAX_SDF: SdfSignedDistance = SdfSignedDistance {
    distance: i32::MAX,
    cross: 0,
    sign: 0,
};

/// `FT_QNEW` of a list element, put at the front of `list`
fn sdf_push_front<T>(list: &mut VecDeque<T>, item: T) -> FtResult<()> {
    if list.try_reserve(1).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    list.push_front(item);
    Ok(())
}

/* sdf_edge_new, sdf_edge_done, sdf_contour_new, sdf_contour_done,      */
/* sdf_shape_new, sdf_shape_done: `sdf_push_front' and dropping things  */

/**************************************************************************
 *
 * shape decomposition functions
 *
 */

impl FtOutlineFuncs for SdfShape {
    /// `sdf_move_to`: This function is called when starting a new contour
    /// at `to`, which gets added to the shape's list.
    fn move_to(&mut self, to: &FtVector) -> FtError {
        let contour = SdfContour {
            last_pos: *to,
            edges: VecDeque::new(),
        };

        match sdf_push_front(&mut self.contours, contour) {
            Ok(()) => 0,
            Err(e) => e,
        }
    }

    /// `sdf_line_to`: This function is called when there is a line in the
    /// contour.  The line starts at the previous edge point and stops at
    /// `to`.
    fn line_to(&mut self, to: &FtVector) -> FtError {
        let Some(contour) = self.contours.front_mut() else {
            return FT_ERR_INVALID_ARGUMENT;
        };

        if contour.last_pos.x == to.x && contour.last_pos.y == to.y {
            return 0;
        }

        let edge = SdfEdge {
            edge_type: SdfEdgeType::Line,
            start_pos: contour.last_pos,
            end_pos: *to,
            ..SdfEdge::default()
        };

        if let Err(e) = sdf_push_front(&mut contour.edges, edge) {
            return e;
        }
        contour.last_pos = *to;

        0
    }

    /// `sdf_conic_to`: This function is called when there is a conic
    /// Bezier curve in the contour.  The curve starts at the previous edge
    /// point and stops at `to`, with control point `control_1`.
    fn conic_to(&mut self, control_1: &FtVector, to: &FtVector) -> FtError {
        let Some(contour) = self.contours.front_mut() else {
            return FT_ERR_INVALID_ARGUMENT;
        };

        /* If the control point coincides with any of the end points */
        /* then it is a line and should be treated as one to avoid   */
        /* unnecessary complexity later in the algorithm.            */
        if (contour.last_pos.x == control_1.x && contour.last_pos.y == control_1.y)
            || (control_1.x == to.x && control_1.y == to.y)
        {
            let _ = self.line_to(to);
            return 0;
        }

        let edge = SdfEdge {
            edge_type: SdfEdgeType::Conic,
            start_pos: contour.last_pos,
            control_a: *control_1,
            end_pos: *to,
            ..SdfEdge::default()
        };

        if let Err(e) = sdf_push_front(&mut contour.edges, edge) {
            return e;
        }
        contour.last_pos = *to;

        0
    }

    /// `sdf_cubic_to`: This function is called when there is a cubic
    /// Bezier curve in the contour.  The curve starts at the previous edge
    /// point and stops at `to`, with two control points `control_1` and
    /// `control_2`.
    fn cubic_to(&mut self, control_1: &FtVector, control_2: &FtVector, to: &FtVector) -> FtError {
        let Some(contour) = self.contours.front_mut() else {
            return FT_ERR_INVALID_ARGUMENT;
        };

        let edge = SdfEdge {
            edge_type: SdfEdgeType::Cubic,
            start_pos: contour.last_pos,
            control_a: *control_1,
            control_b: *control_2,
            end_pos: *to,
        };

        if let Err(e) = sdf_push_front(&mut contour.edges, edge) {
            return e;
        }
        contour.last_pos = *to;

        0
    }
}

/* sdf_decompose_funcs: the `FtOutlineFuncs' implementation above */

/// `sdf_outline_decompose`: Decompose `outline` and put it into the
/// `shape` structure.
fn sdf_outline_decompose(outline: &FtOutline, shape: &mut SdfShape) -> FtResult<()> {
    ft_outline_decompose(outline, shape, 0, 0)
}

/**************************************************************************
 *
 * utility functions
 *
 */

/// `get_control_box`: Return the control box of an edge.  The control box
/// is a rectangle in which all the control points can fit tightly.
fn get_control_box(edge: SdfEdge) -> FtCBox {
    let mut cbox = FtCBox::default();
    let mut is_set = false;

    match edge.edge_type {
        SdfEdgeType::Undefined => return cbox,
        _ => {
            if edge.edge_type == SdfEdgeType::Cubic {
                cbox.xMin = edge.control_b.x;
                cbox.xMax = edge.control_b.x;
                cbox.yMin = edge.control_b.y;
                cbox.yMax = edge.control_b.y;

                is_set = true;
                /* FALL_THROUGH */
            }

            if edge.edge_type == SdfEdgeType::Cubic || edge.edge_type == SdfEdgeType::Conic {
                if is_set {
                    cbox.xMin = if edge.control_a.x < cbox.xMin {
                        edge.control_a.x
                    } else {
                        cbox.xMin
                    };
                    cbox.xMax = if edge.control_a.x > cbox.xMax {
                        edge.control_a.x
                    } else {
                        cbox.xMax
                    };

                    cbox.yMin = if edge.control_a.y < cbox.yMin {
                        edge.control_a.y
                    } else {
                        cbox.yMin
                    };
                    cbox.yMax = if edge.control_a.y > cbox.yMax {
                        edge.control_a.y
                    } else {
                        cbox.yMax
                    };
                } else {
                    cbox.xMin = edge.control_a.x;
                    cbox.xMax = edge.control_a.x;
                    cbox.yMin = edge.control_a.y;
                    cbox.yMax = edge.control_a.y;

                    is_set = true;
                }
                /* FALL_THROUGH */
            }

            /* SDF_EDGE_LINE */
            if is_set {
                cbox.xMin = if edge.start_pos.x < cbox.xMin {
                    edge.start_pos.x
                } else {
                    cbox.xMin
                };
                cbox.xMax = if edge.start_pos.x > cbox.xMax {
                    edge.start_pos.x
                } else {
                    cbox.xMax
                };

                cbox.yMin = if edge.start_pos.y < cbox.yMin {
                    edge.start_pos.y
                } else {
                    cbox.yMin
                };
                cbox.yMax = if edge.start_pos.y > cbox.yMax {
                    edge.start_pos.y
                } else {
                    cbox.yMax
                };
            } else {
                cbox.xMin = edge.start_pos.x;
                cbox.xMax = edge.start_pos.x;
                cbox.yMin = edge.start_pos.y;
                cbox.yMax = edge.start_pos.y;
            }

            cbox.xMin = if edge.end_pos.x < cbox.xMin {
                edge.end_pos.x
            } else {
                cbox.xMin
            };
            cbox.xMax = if edge.end_pos.x > cbox.xMax {
                edge.end_pos.x
            } else {
                cbox.xMax
            };

            cbox.yMin = if edge.end_pos.y < cbox.yMin {
                edge.end_pos.y
            } else {
                cbox.yMin
            };
            cbox.yMax = if edge.end_pos.y > cbox.yMax {
                edge.end_pos.y
            } else {
                cbox.yMax
            };
        }
    }

    cbox
}

/// `get_contour_orientation`: Return orientation of a single contour.
/// Note that the orientation is independent of the fill rule!  So, for TTF
/// a clockwise-oriented contour has to be filled and the opposite for OTF
/// fonts.
fn get_contour_orientation(contour: &SdfContour) -> SdfContourOrientation {
    let mut area: Ft26D6 = 0;

    /* return none if invalid parameters */
    if contour.edges.is_empty() {
        return SdfContourOrientation::None;
    }

    let add = |area: Ft26D6, v: FtPos| -> Ft26D6 { (area as FtPos).wrapping_add(v) as Ft26D6 };

    /* Calculate the area of the control box for all edges. */
    for head in &contour.edges {
        match head.edge_type {
            SdfEdgeType::Line => {
                area = add(
                    area,
                    mul_26d6(
                        head.end_pos.x.wrapping_sub(head.start_pos.x),
                        head.end_pos.y.wrapping_add(head.start_pos.y),
                    ),
                );
            }

            SdfEdgeType::Conic => {
                area = add(
                    area,
                    mul_26d6(
                        head.control_a.x.wrapping_sub(head.start_pos.x),
                        head.control_a.y.wrapping_add(head.start_pos.y),
                    ),
                );
                area = add(
                    area,
                    mul_26d6(
                        head.end_pos.x.wrapping_sub(head.control_a.x),
                        head.end_pos.y.wrapping_add(head.control_a.y),
                    ),
                );
            }

            SdfEdgeType::Cubic => {
                area = add(
                    area,
                    mul_26d6(
                        head.control_a.x.wrapping_sub(head.start_pos.x),
                        head.control_a.y.wrapping_add(head.start_pos.y),
                    ),
                );
                area = add(
                    area,
                    mul_26d6(
                        head.control_b.x.wrapping_sub(head.control_a.x),
                        head.control_b.y.wrapping_add(head.control_a.y),
                    ),
                );
                area = add(
                    area,
                    mul_26d6(
                        head.end_pos.x.wrapping_sub(head.control_b.x),
                        head.end_pos.y.wrapping_add(head.control_b.y),
                    ),
                );
            }

            SdfEdgeType::Undefined => return SdfContourOrientation::None,
        }
    }

    /* Clockwise contours cover a positive area, and counter-clockwise */
    /* contours cover a negative area.                                 */
    if area > 0 {
        SdfContourOrientation::Cw
    } else {
        SdfContourOrientation::Ccw
    }
}

/// `split_conic`: This function is exactly the same as the one in the
/// smooth renderer.  It splits a conic into two conics exactly half way at
/// t = 0.5.
fn split_conic(base: &mut [Ft26D6Vec]) {
    let mut a: Ft26D6;
    let mut b: Ft26D6;

    base[4].x = base[2].x;
    a = base[0].x.wrapping_add(base[1].x) as Ft26D6;
    b = base[1].x.wrapping_add(base[2].x) as Ft26D6;
    base[3].x = (b / 2) as FtPos;
    base[2].x = (a.wrapping_add(b) / 4) as FtPos;
    base[1].x = (a / 2) as FtPos;

    base[4].y = base[2].y;
    a = base[0].y.wrapping_add(base[1].y) as Ft26D6;
    b = base[1].y.wrapping_add(base[2].y) as Ft26D6;
    base[3].y = (b / 2) as FtPos;
    base[2].y = (a.wrapping_add(b) / 4) as FtPos;
    base[1].y = (a / 2) as FtPos;
}

/// `split_cubic`: This function is exactly the same as the one in the
/// smooth renderer.  It splits a cubic into two cubics exactly half way at
/// t = 0.5.
fn split_cubic(base: &mut [Ft26D6Vec]) {
    let mut a: Ft26D6;
    let mut b: Ft26D6;
    let mut c: Ft26D6;

    base[6].x = base[3].x;
    a = base[0].x.wrapping_add(base[1].x) as Ft26D6;
    b = base[1].x.wrapping_add(base[2].x) as Ft26D6;
    c = base[2].x.wrapping_add(base[3].x) as Ft26D6;
    base[5].x = (c / 2) as FtPos;
    c = c.wrapping_add(b);
    base[4].x = (c / 4) as FtPos;
    base[1].x = (a / 2) as FtPos;
    a = a.wrapping_add(b);
    base[2].x = (a / 4) as FtPos;
    base[3].x = (a.wrapping_add(c) / 8) as FtPos;

    base[6].y = base[3].y;
    a = base[0].y.wrapping_add(base[1].y) as Ft26D6;
    b = base[1].y.wrapping_add(base[2].y) as Ft26D6;
    c = base[2].y.wrapping_add(base[3].y) as Ft26D6;
    base[5].y = (c / 2) as FtPos;
    c = c.wrapping_add(b);
    base[4].y = (c / 4) as FtPos;
    base[1].y = (a / 2) as FtPos;
    a = a.wrapping_add(b);
    base[2].y = (a / 4) as FtPos;
    base[3].y = (a.wrapping_add(c) / 8) as FtPos;
}

/// Add the two lines of a split curve to the front of `out` (the
/// `Append` part of the curve splitters).
fn append_split_lines(
    out: &mut VecDeque<SdfEdge>,
    p0: Ft26D6Vec,
    p1: Ft26D6Vec,
    p2: Ft26D6Vec,
) -> FtResult<()> {
    /* Do allocation and add the lines to the list. */

    let left = SdfEdge {
        start_pos: p0,
        end_pos: p1,
        edge_type: SdfEdgeType::Line,
        ..SdfEdge::default()
    };

    let right = SdfEdge {
        start_pos: p1,
        end_pos: p2,
        edge_type: SdfEdgeType::Line,
        ..SdfEdge::default()
    };

    if out.try_reserve(2).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }
    out.push_front(right);
    out.push_front(left);

    Ok(())
}

/// `split_sdf_conic`: Split a conic Bezier curve into a number of lines
/// and add them to `out'.
///
/// This function uses recursion; we thus need parameter `max_splits' for
/// stopping.
fn split_sdf_conic(
    control_points: &[Ft26D6Vec],
    max_splits: FtUInt,
    out: &mut VecDeque<SdfEdge>,
) -> FtResult<()> {
    let mut cpos = [ZERO_VECTOR; 5];

    /* split conic outline */
    cpos[0] = control_points[0];
    cpos[1] = control_points[1];
    cpos[2] = control_points[2];

    split_conic(&mut cpos);

    /* If max number of splits is done */
    /* then stop and add the lines to  */
    /* the list.                       */
    if max_splits <= 2 {
        return append_split_lines(out, cpos[0], cpos[2], cpos[4]);
    }

    /* Otherwise keep splitting. */
    split_sdf_conic(&cpos[0..], max_splits / 2, out)?;
    split_sdf_conic(&cpos[2..], max_splits / 2, out)?;

    /* [NOTE]: This is not an efficient way of   */
    /* splitting the curve.  Check the deviation */
    /* instead and stop if the deviation is less */
    /* than a pixel.                             */

    Ok(())
}

/// `split_sdf_cubic`: Split a cubic Bezier curve into a number of lines
/// and add them to `out`.
///
/// This function uses recursion; we thus need parameter `max_splits' for
/// stopping.
fn split_sdf_cubic(
    control_points: &[Ft26D6Vec],
    max_splits: FtUInt,
    out: &mut VecDeque<SdfEdge>,
) -> FtResult<()> {
    let mut cpos = [ZERO_VECTOR; 7];
    let threshold: Ft26D6 = (ONE_PIXEL / 4) as Ft26D6;

    /* split the cubic */
    cpos[0] = control_points[0];
    cpos[1] = control_points[1];
    cpos[2] = control_points[2];
    cpos[3] = control_points[3];

    let dev = |a: FtPos, b: FtPos, c: FtPos| -> FtPos {
        a.wrapping_sub(b).wrapping_add(c).wrapping_abs()
    };

    /* If the segment is flat enough we won't get any benefit by */
    /* splitting it further, so we can just stop splitting.      */
    /*                                                           */
    /* Check the deviation of the Bezier curve and stop if it is */
    /* smaller than the pre-defined `threshold` value.           */
    if dev(
        cpos[0].x.wrapping_mul(2),
        cpos[1].x.wrapping_mul(3),
        cpos[3].x,
    ) < threshold as FtPos
        && dev(
            cpos[0].y.wrapping_mul(2),
            cpos[1].y.wrapping_mul(3),
            cpos[3].y,
        ) < threshold as FtPos
        && dev(
            cpos[0].x,
            cpos[2].x.wrapping_mul(3),
            cpos[3].x.wrapping_mul(2),
        ) < threshold as FtPos
        && dev(
            cpos[0].y,
            cpos[2].y.wrapping_mul(3),
            cpos[3].y.wrapping_mul(2),
        ) < threshold as FtPos
    {
        split_cubic(&mut cpos);
        return append_split_lines(out, cpos[0], cpos[3], cpos[6]);
    }

    split_cubic(&mut cpos);

    /* If max number of splits is done */
    /* then stop and add the lines to  */
    /* the list.                       */
    if max_splits <= 2 {
        return append_split_lines(out, cpos[0], cpos[3], cpos[6]);
    }

    /* Otherwise keep splitting. */
    split_sdf_cubic(&cpos[0..], max_splits / 2, out)?;
    split_sdf_cubic(&cpos[3..], max_splits / 2, out)?;

    /* [NOTE]: This is not an efficient way of   */
    /* splitting the curve.  Check the deviation */
    /* instead and stop if the deviation is less */
    /* than a pixel.                             */

    Ok(())
}

/// `split_sdf_shape`: Subdivide an entire shape into line segments such
/// that it doesn't look visually different from the original curve.
fn split_sdf_shape(shape: &mut SdfShape) -> FtResult<()> {
    let mut new_contours: VecDeque<SdfContour> = VecDeque::new();

    /* for each contour */
    for contour in &shape.contours {
        let mut new_edges: VecDeque<SdfEdge> = VecDeque::new();

        /* for each edge */
        for edge in &contour.edges {
            match edge.edge_type {
                SdfEdgeType::Line => {
                    /* Just create a duplicate edge in case     */
                    /* it is a line.  We can use the same edge. */
                    sdf_push_front(&mut new_edges, *edge)?;
                }

                SdfEdgeType::Conic => {
                    /* Subdivide the curve and add it to the list. */
                    let ctrls = [edge.start_pos, edge.control_a, edge.end_pos];
                    let mut dx: Ft26D6;
                    let dy: Ft26D6;
                    let mut num_splits: FtUInt;

                    dx = ctrls[2]
                        .x
                        .wrapping_add(ctrls[0].x)
                        .wrapping_sub(ctrls[1].x.wrapping_mul(2))
                        .wrapping_abs() as Ft26D6;
                    dy = ctrls[2]
                        .y
                        .wrapping_add(ctrls[0].y)
                        .wrapping_sub(ctrls[1].y.wrapping_mul(2))
                        .wrapping_abs() as Ft26D6;
                    if dx < dy {
                        dx = dy;
                    }

                    /* Calculate the number of necessary bisections.  Each      */
                    /* bisection causes a four-fold reduction of the deviation, */
                    /* hence we bisect the Bezier curve until the deviation     */
                    /* becomes less than 1/8 of a pixel.  For more details      */
                    /* check file `ftgrays.c`.                                  */
                    num_splits = 1;
                    while dx > (ONE_PIXEL / 8) as Ft26D6 {
                        dx >>= 2;
                        num_splits = num_splits.wrapping_shl(1);
                    }

                    split_sdf_conic(&ctrls, num_splits, &mut new_edges)?;
                }

                SdfEdgeType::Cubic => {
                    /* Subdivide the curve and add it to the list. */
                    let ctrls = [edge.start_pos, edge.control_a, edge.control_b, edge.end_pos];

                    split_sdf_cubic(&ctrls, 32, &mut new_edges)?;
                }

                SdfEdgeType::Undefined => return Err(FT_ERR_INVALID_ARGUMENT),
            }
        }

        /* add to the contours list */
        sdf_push_front(
            &mut new_contours,
            SdfContour {
                last_pos: ZERO_VECTOR,
                edges: new_edges,
            },
        )?;

        /* deallocate the contour: (it goes with the old list) */
    }

    shape.contours = new_contours;

    Ok(())
}

/* sdf_shape_dump: FT_DEBUG_LEVEL_TRACE only */

/**************************************************************************
 *
 * math functions
 *
 */

/* cube_root, arc_cos, solve_quadratic_equation, solve_cubic_equation: */
/* !USE_NEWTON_FOR_CONIC only                                          */

/**************************************************************************
 *
 * RASTERIZER
 *
 */

/// `resolve_corner`: At some places on the grid two edges can give
/// opposite directions; this happens when the closest point is on one of
/// the endpoint.  In that case we need to check the proper sign.
///
/// This can be visualized by an example:
///
/// ```text
///              x
///
///                 o
///                ^ \
///               /   \
///              /     \
///         (a) /       \  (b)
///            /         \
///           /           \
///          /             v
/// ```
///
/// Suppose `x` is the point whose shortest distance from an arbitrary
/// contour we want to find out.  It is clear that `o` is the nearest
/// point on the contour.  Now to determine the sign we do a cross
/// product of the shortest distance vector and the edge direction, i.e.,
///
/// ```text
/// => sign = cross(x - o, direction(a))
/// ```
///
/// Using the right hand thumb rule we can see that the sign will be
/// positive.
///
/// If we use `b', however, we have
///
/// ```text
/// => sign = cross(x - o, direction(b))
/// ```
///
/// In this case the sign will be negative.  To determine the correct
/// sign we thus divide the plane in two halves and check which plane the
/// point lies in.
///
/// ```text
///                 |
///              x  |
///                 |
///                 o
///                ^|\
///               / | \
///              /  |  \
///         (a) /   |   \  (b)
///            /    |    \
///           /           \
///          /             v
/// ```
///
/// We can see that `x` lies in the plane of `a`, so we take the sign
/// determined by `a`.  This test can be easily done by calculating the
/// orthogonality and taking the greater one.
///
/// The orthogonality is simply the sinus of the two vectors (i.e.,
/// x - o) and the corresponding direction.  We efficiently pre-compute
/// the orthogonality with the corresponding `get_min_distance_*`
/// functions.
///
/// The function does not care about the actual distance, it simply
/// returns the signed distance which has a larger cross product.  As a
/// consequence, this function should not be used if the two distances
/// are fairly apart.  In that case simply use the signed distance with
/// a shorter absolute distance.
fn resolve_corner(sdf1: SdfSignedDistance, sdf2: SdfSignedDistance) -> SdfSignedDistance {
    if sdf1.cross.wrapping_abs() > sdf2.cross.wrapping_abs() {
        sdf1
    } else {
        sdf2
    }
}

/// `get_min_distance_line`: Find the shortest distance from the `line`
/// segment to a given `point` and assign it to `out`.  Use it for line
/// segments only.
fn get_min_distance_line(
    line: &SdfEdge,
    point: Ft26D6Vec,
    out: &mut SdfSignedDistance,
) -> FtResult<()> {
    /*
     * In order to calculate the shortest distance from a point to
     * a line segment, we do the following.  Let's assume that
     *
     * ```
     * a = start point of the line segment
     * b = end point of the line segment
     * p = point from which shortest distance is to be calculated
     * ```
     *
     * (1) Write the parametric equation of the line.
     *
     *     ```
     *     point_on_line = a + (b - a) * t   (t is the factor)
     *     ```
     *
     * (2) Find the projection of point `p` on the line.  The projection
     *     will be perpendicular to the line, which allows us to get the
     *     solution by making the dot product zero.
     *
     *     ```
     *     (point_on_line - a) . (p - point_on_line) = 0
     *
     *                (point_on_line)
     *      (a) x-------o----------------x (b)
     *                |_|
     *                  |
     *                  |
     *                 (p)
     *     ```
     *
     * (3) Simplification of the above equation yields the factor of
     *     `point_on_line`:
     *
     *     ```
     *     t = ((p - a) . (b - a)) / |b - a|^2
     *     ```
     *
     * (4) We clamp factor `t` between [0.0f, 1.0f] because `point_on_line`
     *     can be outside of the line segment:
     *
     *     ```
     *                                          (point_on_line)
     *     (a) x------------------------x (b) -----o---
     *                                           |_|
     *                                             |
     *                                             |
     *                                            (p)
     *     ```
     *
     * (5) Finally, the distance we are interested in is
     *
     *     ```
     *     |point_on_line - p|
     *     ```
     */

    let mut line_segment: Ft26D6Vec = ZERO_VECTOR; /* `b` - `a` */
    let mut p_sub_a: Ft26D6Vec = ZERO_VECTOR; /* `p` - `a` */

    let sq_line_length: Ft26D6; /* squared length of `line_segment` */
    let mut factor: Ft16D16; /* factor of the nearest point      */
    let cross: Ft26D6; /* used to determine sign           */

    let mut nearest_point: Ft16D16Vec = ZERO_VECTOR; /* `point_on_line`       */
    let mut nearest_vector: Ft16D16Vec = ZERO_VECTOR; /* `p` - `nearest_point` */

    if line.edge_type != SdfEdgeType::Line {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let a = line.start_pos; /* start position */
    let b = line.end_pos; /* end position   */
    let p = point; /* current point  */

    line_segment.x = b.x.wrapping_sub(a.x);
    line_segment.y = b.y.wrapping_sub(a.y);

    p_sub_a.x = p.x.wrapping_sub(a.x);
    p_sub_a.y = p.y.wrapping_sub(a.y);

    sq_line_length = (line_segment.x.wrapping_mul(line_segment.x) / 64)
        .wrapping_add(line_segment.y.wrapping_mul(line_segment.y) / 64)
        as Ft26D6;

    /* currently factor is 26.6 */
    factor = (p_sub_a.x.wrapping_mul(line_segment.x) / 64)
        .wrapping_add(p_sub_a.y.wrapping_mul(line_segment.y) / 64) as Ft16D16;

    /* now factor is 16.16 */
    factor = ft_div_fix(factor as FtLong, sq_line_length as FtLong) as Ft16D16;

    /* clamp the factor between 0.0 and 1.0 in fixed-point */
    if factor as FtPos > ft_int_16d16(1) {
        factor = ft_int_16d16(1) as Ft16D16;
    }
    if factor < 0 {
        factor = 0;
    }

    nearest_point.x = ft_mul_fix(ft_26d6_16d16(line_segment.x), factor as FtLong);
    nearest_point.y = ft_mul_fix(ft_26d6_16d16(line_segment.y), factor as FtLong);

    nearest_point.x = ft_26d6_16d16(a.x).wrapping_add(nearest_point.x);
    nearest_point.y = ft_26d6_16d16(a.y).wrapping_add(nearest_point.y);

    nearest_vector.x = nearest_point.x.wrapping_sub(ft_26d6_16d16(p.x));
    nearest_vector.y = nearest_point.y.wrapping_sub(ft_26d6_16d16(p.y));

    cross = ft_mul_fix(nearest_vector.x, line_segment.y)
        .wrapping_sub(ft_mul_fix(nearest_vector.y, line_segment.x)) as Ft26D6;

    /* assign the output */
    out.sign = if cross < 0 { 1 } else { -1 };
    out.distance = vector_length_16d16(&nearest_vector) as Ft16D16;

    /* Instead of finding `cross` for checking corner we */
    /* directly set it here.  This is more efficient     */
    /* because if the distance is perpendicular we can   */
    /* directly set it to 1.                             */
    if factor != 0 && factor as FtPos != ft_int_16d16(1) {
        out.cross = ft_int_16d16(1) as Ft16D16;
    } else {
        /* [OPTIMIZATION]: Pre-compute this direction. */
        /* If not perpendicular then compute `cross`.  */
        ft_vector_norm_len(&mut line_segment);
        ft_vector_norm_len(&mut nearest_vector);

        out.cross = ft_mul_fix(line_segment.x, nearest_vector.y)
            .wrapping_sub(ft_mul_fix(line_segment.y, nearest_vector.x))
            as Ft16D16;
    }

    Ok(())
}

/* get_min_distance_conic (the analytical one): !USE_NEWTON_FOR_CONIC */

/// `get_min_distance_conic` (`USE_NEWTON_FOR_CONIC`): Find the shortest
/// distance from the `conic` Bezier curve to a given `point` and assign it
/// to `out`.  Use it for conic/quadratic curves only.
///
/// The function uses Newton's approximation to find the shortest distance,
/// which is a bit slower than the analytical method but doesn't cause
/// underflow.
fn get_min_distance_conic(
    conic: &SdfEdge,
    point: Ft26D6Vec,
    out: &mut SdfSignedDistance,
) -> FtResult<()> {
    /*
     * This method uses Newton-Raphson's approximation to find the shortest
     * distance from a point to a conic curve.  It does not involve solving
     * any cubic equation, that is why there is no risk of underflow.
     *
     * Let's assume that
     *
     * ```
     * p0 = first endpoint
     * p1 = control point
     * p3 = second endpoint
     * p  = point from which shortest distance is to be calculated
     * ```
     *
     * (1) The equation of a quadratic Bezier curve can be written as
     *
     *     ```
     *     B(t) = (1 - t)^2 * p0 + 2(1 - t)t * p1 + t^2 * p2
     *     ```
     *
     *     with `t` the factor in the range [0.0f, 1.0f].  The above
     *     equation can be rewritten as
     *
     *     ```
     *     B(t) = t^2 * (p0 - 2p1 + p2) + 2t * (p1 - p0) + p0
     *     ```
     *
     *     With
     *
     *     ```
     *     A = p0 - 2p1 + p2
     *     B = 2 * (p1 - p0)
     *     ```
     *
     *     we have
     *
     *     ```
     *     B(t) = t^2 * A + t * B + p0
     *     ```
     *
     * (2) The derivative of the above equation is
     *
     *     ```
     *     B'(t) = 2t * A + B
     *     ```
     *
     * (3) The second derivative of the above equation is
     *
     *     ```
     *     B''(t) = 2A
     *     ```
     *
     * (4) The equation `P(t)` of the distance from point `p` to the curve
     *     can be written as
     *
     *     ```
     *     P(t) = t^2 * A + t^2 * B + p0 - p
     *     ```
     *
     *     With
     *
     *     ```
     *     C = p0 - p
     *     ```
     *
     *     we have
     *
     *     ```
     *     P(t) = t^2 * A + t * B + C
     *     ```
     *
     * (5) Finally, the equation of the angle between `B(t)` and `P(t)` can
     *     be written as
     *
     *     ```
     *     Q(t) = P(t) . B'(t)
     *     ```
     *
     * (6) Our task is to find a value of `t` such that the above equation
     *     `Q(t)` becomes zero, that is, the point-to-curve vector makes
     *     90~degrees with the curve.  We solve this with the Newton-Raphson
     *     method.
     *
     * (7) We first assume an arbitrary value of factor `t`, which we then
     *     improve.
     *
     *     ```
     *     t := Q(t) / Q'(t)
     *     ```
     *
     *     Putting the value of `Q(t)` from the above equation gives
     *
     *     ```
     *     t := P(t) . B'(t) / derivative(P(t) . B'(t))
     *     t := P(t) . B'(t) /
     *            (P'(t) . B'(t) + P(t) . B''(t))
     *     ```
     *
     *     Note that `P'(t)` is the same as `B'(t)` because the constant is
     *     gone due to the derivative.
     *
     * (8) Finally we get the equation to improve the factor as
     *
     *     ```
     *     t := P(t) . B'(t) /
     *            (B'(t) . B'(t) + P(t) . B''(t))
     *     ```
     *
     * [note]: `B` and `B(t)` are different in the above equations.
     */

    let mut aA: Ft26D6Vec = ZERO_VECTOR; /* A, B, C in the above comment          */
    let mut bB: Ft26D6Vec = ZERO_VECTOR;
    let mut cC: Ft26D6Vec = ZERO_VECTOR;
    let mut nearest_point: Ft26D6Vec = ZERO_VECTOR;
    /* point on curve nearest to `point`     */
    let mut direction: Ft26D6Vec = ZERO_VECTOR; /* direction of curve at `nearest_point` */

    let mut min_factor: Ft16D16 = 0; /* factor at `nearest_point'     */
    let cross: Ft16D16; /* to determine the sign         */
    let mut min: Ft16D16 = i32::MAX; /* shortest squared distance     */

    if conic.edge_type != SdfEdgeType::Conic {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let p0 = conic.start_pos; /* control points of a conic curve       */
    let p1 = conic.control_a;
    let p2 = conic.end_pos;
    let p = point; /* `point` to which shortest distance    */

    /* compute substitution coefficients */
    aA.x = p0.x.wrapping_sub(p1.x.wrapping_mul(2)).wrapping_add(p2.x);
    aA.y = p0.y.wrapping_sub(p1.y.wrapping_mul(2)).wrapping_add(p2.y);

    bB.x = p1.x.wrapping_sub(p0.x).wrapping_mul(2);
    bB.y = p1.y.wrapping_sub(p0.y).wrapping_mul(2);

    cC.x = p0.x;
    cC.y = p0.y;

    /* do Newton's iterations */
    for iterations in 0..=MAX_NEWTON_DIVISIONS {
        let mut factor: Ft16D16 =
            (ft_int_16d16(iterations as FtPos) / MAX_NEWTON_DIVISIONS as FtPos) as Ft16D16;
        let mut factor2: Ft16D16;
        let mut length: Ft16D16;

        let mut curve_point: Ft16D16Vec = ZERO_VECTOR; /* point on the curve  */
        let mut dist_vector: Ft16D16Vec = ZERO_VECTOR; /* `curve_point` - `p` */

        let mut d1: Ft26D6Vec = ZERO_VECTOR; /* first  derivative   */
        let mut d2: Ft26D6Vec = ZERO_VECTOR; /* second derivative   */

        let mut temp1: Ft16D16;
        let mut temp2: Ft16D16;

        for _steps in 0..MAX_NEWTON_STEPS {
            factor2 = ft_mul_fix(factor as FtLong, factor as FtLong) as Ft16D16;

            /* B(t) = t^2 * A + t * B + p0 */
            curve_point.x = ft_mul_fix(aA.x, factor2 as FtLong)
                .wrapping_add(ft_mul_fix(bB.x, factor as FtLong))
                .wrapping_add(cC.x);
            curve_point.y = ft_mul_fix(aA.y, factor2 as FtLong)
                .wrapping_add(ft_mul_fix(bB.y, factor as FtLong))
                .wrapping_add(cC.y);

            /* convert to 16.16 */
            curve_point.x = ft_26d6_16d16(curve_point.x);
            curve_point.y = ft_26d6_16d16(curve_point.y);

            /* P(t) in the comment */
            dist_vector.x = curve_point.x.wrapping_sub(ft_26d6_16d16(p.x));
            dist_vector.y = curve_point.y.wrapping_sub(ft_26d6_16d16(p.y));

            length = vector_length_16d16(&dist_vector) as Ft16D16;

            if length < min {
                min = length;
                min_factor = factor;
                nearest_point = curve_point;
            }

            /* This is Newton's approximation.          */
            /*                                          */
            /*   t := P(t) . B'(t) /                    */
            /*          (B'(t) . B'(t) + P(t) . B''(t)) */

            /* B'(t) = 2tA + B */
            d1.x = ft_mul_fix(aA.x, factor.wrapping_mul(2) as FtLong).wrapping_add(bB.x);
            d1.y = ft_mul_fix(aA.y, factor.wrapping_mul(2) as FtLong).wrapping_add(bB.y);

            /* B''(t) = 2A */
            d2.x = aA.x.wrapping_mul(2);
            d2.y = aA.y.wrapping_mul(2);

            dist_vector.x /= 1024;
            dist_vector.y /= 1024;

            /* temp1 = P(t) . B'(t) */
            temp1 = vec_26d6_dot(&dist_vector, &d1) as Ft16D16;

            /* temp2 = B'(t) . B'(t) + P(t) . B''(t) */
            temp2 = vec_26d6_dot(&d1, &d1).wrapping_add(vec_26d6_dot(&dist_vector, &d2)) as Ft16D16;

            factor = (factor as FtLong).wrapping_sub(ft_div_fix(temp1 as FtLong, temp2 as FtLong))
                as Ft16D16;

            if factor < 0 || factor as FtPos > ft_int_16d16(1) {
                break;
            }
        }
    }

    /* B'(t) = 2t * A + B */
    direction.x = ft_mul_fix(aA.x, min_factor as FtLong)
        .wrapping_mul(2)
        .wrapping_add(bB.x);
    direction.y = ft_mul_fix(aA.y, min_factor as FtLong)
        .wrapping_mul(2)
        .wrapping_add(bB.y);

    /* determine the sign */
    cross = ft_mul_fix(
        nearest_point.x.wrapping_sub(ft_26d6_16d16(p.x)),
        direction.y,
    )
    .wrapping_sub(ft_mul_fix(
        nearest_point.y.wrapping_sub(ft_26d6_16d16(p.y)),
        direction.x,
    )) as Ft16D16;

    /* assign the values */
    out.distance = min;
    out.sign = if cross < 0 { 1 } else { -1 };

    if min_factor != 0 && min_factor as FtPos != ft_int_16d16(1) {
        out.cross = ft_int_16d16(1) as Ft16D16; /* the two are perpendicular */
    } else {
        /* convert to nearest vector */
        nearest_point.x = nearest_point.x.wrapping_sub(ft_26d6_16d16(p.x));
        nearest_point.y = nearest_point.y.wrapping_sub(ft_26d6_16d16(p.y));

        /* compute `cross` if not perpendicular */
        ft_vector_norm_len(&mut direction);
        ft_vector_norm_len(&mut nearest_point);

        out.cross = ft_mul_fix(direction.x, nearest_point.y)
            .wrapping_sub(ft_mul_fix(direction.y, nearest_point.x)) as Ft16D16;
    }

    Ok(())
}

/// `get_min_distance_cubic`: Find the shortest distance from the `cubic`
/// Bezier curve to a given `point` and assigns it to `out`.  Use it for
/// cubic curves only.
///
/// The function uses Newton's approximation to find the shortest
/// distance.  Another way would be to divide the cubic into conic or
/// subdivide the curve into lines, but that is not implemented.
fn get_min_distance_cubic(
    cubic: &SdfEdge,
    point: Ft26D6Vec,
    out: &mut SdfSignedDistance,
) -> FtResult<()> {
    /*
     * The procedure to find the shortest distance from a point to a cubic
     * Bezier curve is similar to quadratic curve algorithm.  The only
     * difference is that while calculating factor `t`, instead of a cubic
     * polynomial equation we have to find the roots of a 5th degree
     * polynomial equation.  Solving this would require a significant amount
     * of time, and still the results may not be accurate.  We are thus
     * going to directly approximate the value of `t` using the Newton-Raphson
     * method.
     *
     * Let's assume that
     *
     * ```
     * p0 = first endpoint
     * p1 = first control point
     * p2 = second control point
     * p3 = second endpoint
     * p  = point from which shortest distance is to be calculated
     * ```
     *
     * (1) The equation of a cubic Bezier curve can be written as
     *
     *     ```
     *     B(t) = (1 - t)^3 * p0 + 3(1 - t)^2 t * p1 +
     *              3(1 - t)t^2 * p2 + t^3 * p3
     *     ```
     *
     *     The equation can be expanded and written as
     *
     *     ```
     *     B(t) = t^3 * (-p0 + 3p1 - 3p2 + p3) +
     *              3t^2 * (p0 - 2p1 + p2) + 3t * (-p0 + p1) + p0
     *     ```
     *
     *     With
     *
     *     ```
     *     A = -p0 + 3p1 - 3p2 + p3
     *     B = 3(p0 - 2p1 + p2)
     *     C = 3(-p0 + p1)
     *     ```
     *
     *     we have
     *
     *     ```
     *     B(t) = t^3 * A + t^2 * B + t * C + p0
     *     ```
     *
     * (2) The derivative of the above equation is
     *
     *     ```
     *     B'(t) = 3t^2 * A + 2t * B + C
     *     ```
     *
     * (3) The second derivative of the above equation is
     *
     *     ```
     *     B''(t) = 6t * A + 2B
     *     ```
     *
     * (4) The equation `P(t)` of the distance from point `p` to the curve
     *     can be written as
     *
     *     ```
     *     P(t) = t^3 * A + t^2 * B + t * C + p0 - p
     *     ```
     *
     *     With
     *
     *     ```
     *     D = p0 - p
     *     ```
     *
     *     we have
     *
     *     ```
     *     P(t) = t^3 * A + t^2 * B + t * C + D
     *     ```
     *
     * (5) Finally the equation of the angle between `B(t)` and `P(t)` can
     *     be written as
     *
     *     ```
     *     Q(t) = P(t) . B'(t)
     *     ```
     *
     * (6) Our task is to find a value of `t` such that the above equation
     *     `Q(t)` becomes zero, that is, the point-to-curve vector makes
     *     90~degree with curve.  We solve this with the Newton-Raphson
     *     method.
     *
     * (7) We first assume an arbitrary value of factor `t`, which we then
     *     improve.
     *
     *     ```
     *     t := Q(t) / Q'(t)
     *     ```
     *
     *     Putting the value of `Q(t)` from the above equation gives
     *
     *     ```
     *     t := P(t) . B'(t) / derivative(P(t) . B'(t))
     *     t := P(t) . B'(t) /
     *            (P'(t) . B'(t) + P(t) . B''(t))
     *     ```
     *
     *     Note that `P'(t)` is the same as `B'(t)` because the constant is
     *     gone due to the derivative.
     *
     * (8) Finally we get the equation to improve the factor as
     *
     *     ```
     *     t := P(t) . B'(t) /
     *            (B'(t) . B'( t ) + P(t) . B''(t))
     *     ```
     *
     * [note]: `B` and `B(t)` are different in the above equations.
     */

    let mut aA: Ft26D6Vec = ZERO_VECTOR; /* A, B, C, D in the above comment       */
    let mut bB: Ft26D6Vec = ZERO_VECTOR;
    let mut cC: Ft26D6Vec = ZERO_VECTOR;
    let mut dD: Ft26D6Vec = ZERO_VECTOR;
    let mut nearest_point: Ft16D16Vec = ZERO_VECTOR;
    /* point on curve nearest to `point`     */
    let mut direction: Ft16D16Vec = ZERO_VECTOR; /* direction of curve at `nearest_point` */

    let mut min_factor: Ft16D16 = 0; /* factor at shortest distance */
    let mut min_factor_sq: Ft16D16 = 0; /* factor at shortest distance */
    let cross: Ft16D16; /* to determine the sign       */
    let mut min: Ft16D16 = i32::MAX; /* shortest distance           */

    if cubic.edge_type != SdfEdgeType::Cubic {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let p0 = cubic.start_pos; /* control points of a cubic curve       */
    let p1 = cubic.control_a;
    let p2 = cubic.control_b;
    let p3 = cubic.end_pos;
    let p = point; /* `point` to which shortest distance    */

    /* compute substitution coefficients */
    aA.x =
        p0.x.wrapping_neg()
            .wrapping_add(p1.x.wrapping_sub(p2.x).wrapping_mul(3))
            .wrapping_add(p3.x);
    aA.y =
        p0.y.wrapping_neg()
            .wrapping_add(p1.y.wrapping_sub(p2.y).wrapping_mul(3))
            .wrapping_add(p3.y);

    bB.x =
        p0.x.wrapping_sub(p1.x.wrapping_mul(2))
            .wrapping_add(p2.x)
            .wrapping_mul(3);
    bB.y =
        p0.y.wrapping_sub(p1.y.wrapping_mul(2))
            .wrapping_add(p2.y)
            .wrapping_mul(3);

    cC.x = p1.x.wrapping_sub(p0.x).wrapping_mul(3);
    cC.y = p1.y.wrapping_sub(p0.y).wrapping_mul(3);

    dD.x = p0.x;
    dD.y = p0.y;

    for iterations in 0..=MAX_NEWTON_DIVISIONS {
        let mut factor: Ft16D16 =
            (ft_int_16d16(iterations as FtPos) / MAX_NEWTON_DIVISIONS as FtPos) as Ft16D16;

        let mut factor2: Ft16D16; /* factor^2            */
        let mut factor3: Ft16D16; /* factor^3            */
        let mut length: Ft16D16;

        let mut curve_point: Ft16D16Vec = ZERO_VECTOR; /* point on the curve  */
        let mut dist_vector: Ft16D16Vec = ZERO_VECTOR; /* `curve_point' - `p' */

        let mut d1: Ft26D6Vec = ZERO_VECTOR; /* first  derivative   */
        let mut d2: Ft26D6Vec = ZERO_VECTOR; /* second derivative   */

        let mut temp1: Ft16D16;
        let mut temp2: Ft16D16;

        for _steps in 0..MAX_NEWTON_STEPS {
            factor2 = ft_mul_fix(factor as FtLong, factor as FtLong) as Ft16D16;
            factor3 = ft_mul_fix(factor2 as FtLong, factor as FtLong) as Ft16D16;

            /* B(t) = t^3 * A + t^2 * B + t * C + D */
            curve_point.x = ft_mul_fix(aA.x, factor3 as FtLong)
                .wrapping_add(ft_mul_fix(bB.x, factor2 as FtLong))
                .wrapping_add(ft_mul_fix(cC.x, factor as FtLong))
                .wrapping_add(dD.x);
            curve_point.y = ft_mul_fix(aA.y, factor3 as FtLong)
                .wrapping_add(ft_mul_fix(bB.y, factor2 as FtLong))
                .wrapping_add(ft_mul_fix(cC.y, factor as FtLong))
                .wrapping_add(dD.y);

            /* convert to 16.16 */
            curve_point.x = ft_26d6_16d16(curve_point.x);
            curve_point.y = ft_26d6_16d16(curve_point.y);

            /* P(t) in the comment */
            dist_vector.x = curve_point.x.wrapping_sub(ft_26d6_16d16(p.x));
            dist_vector.y = curve_point.y.wrapping_sub(ft_26d6_16d16(p.y));

            length = vector_length_16d16(&dist_vector) as Ft16D16;

            if length < min {
                min = length;
                min_factor = factor;
                min_factor_sq = factor2;
                nearest_point = curve_point;
            }

            /* This the Newton's approximation.         */
            /*                                          */
            /*   t := P(t) . B'(t) /                    */
            /*          (B'(t) . B'(t) + P(t) . B''(t)) */

            /* B'(t) = 3t^2 * A + 2t * B + C */
            d1.x = ft_mul_fix(aA.x, factor2.wrapping_mul(3) as FtLong)
                .wrapping_add(ft_mul_fix(bB.x, factor.wrapping_mul(2) as FtLong))
                .wrapping_add(cC.x);
            d1.y = ft_mul_fix(aA.y, factor2.wrapping_mul(3) as FtLong)
                .wrapping_add(ft_mul_fix(bB.y, factor.wrapping_mul(2) as FtLong))
                .wrapping_add(cC.y);

            /* B''(t) = 6t * A + 2B */
            d2.x = ft_mul_fix(aA.x, factor.wrapping_mul(6) as FtLong)
                .wrapping_add(bB.x.wrapping_mul(2));
            d2.y = ft_mul_fix(aA.y, factor.wrapping_mul(6) as FtLong)
                .wrapping_add(bB.y.wrapping_mul(2));

            dist_vector.x /= 1024;
            dist_vector.y /= 1024;

            /* temp1 = P(t) . B'(t) */
            temp1 = vec_26d6_dot(&dist_vector, &d1) as Ft16D16;

            /* temp2 = B'(t) . B'(t) + P(t) . B''(t) */
            temp2 = vec_26d6_dot(&d1, &d1).wrapping_add(vec_26d6_dot(&dist_vector, &d2)) as Ft16D16;

            factor = (factor as FtLong).wrapping_sub(ft_div_fix(temp1 as FtLong, temp2 as FtLong))
                as Ft16D16;

            if factor < 0 || factor as FtPos > ft_int_16d16(1) {
                break;
            }
        }
    }

    /* B'(t) = 3t^2 * A + 2t * B + C */
    direction.x = ft_mul_fix(aA.x, min_factor_sq.wrapping_mul(3) as FtLong)
        .wrapping_add(ft_mul_fix(bB.x, min_factor.wrapping_mul(2) as FtLong))
        .wrapping_add(cC.x);
    direction.y = ft_mul_fix(aA.y, min_factor_sq.wrapping_mul(3) as FtLong)
        .wrapping_add(ft_mul_fix(bB.y, min_factor.wrapping_mul(2) as FtLong))
        .wrapping_add(cC.y);

    /* determine the sign */
    cross = ft_mul_fix(
        nearest_point.x.wrapping_sub(ft_26d6_16d16(p.x)),
        direction.y,
    )
    .wrapping_sub(ft_mul_fix(
        nearest_point.y.wrapping_sub(ft_26d6_16d16(p.y)),
        direction.x,
    )) as Ft16D16;

    /* assign the values */
    out.distance = min;
    out.sign = if cross < 0 { 1 } else { -1 };

    if min_factor != 0 && min_factor as FtPos != ft_int_16d16(1) {
        out.cross = ft_int_16d16(1) as Ft16D16; /* the two are perpendicular */
    } else {
        /* convert to nearest vector */
        nearest_point.x = nearest_point.x.wrapping_sub(ft_26d6_16d16(p.x));
        nearest_point.y = nearest_point.y.wrapping_sub(ft_26d6_16d16(p.y));

        /* compute `cross` if not perpendicular */
        ft_vector_norm_len(&mut direction);
        ft_vector_norm_len(&mut nearest_point);

        out.cross = ft_mul_fix(direction.x, nearest_point.y)
            .wrapping_sub(ft_mul_fix(direction.y, nearest_point.x)) as Ft16D16;
    }

    Ok(())
}

/// `sdf_edge_get_min_distance`: Find shortest distance from `point` to
/// any type of `edge`.  It checks the edge type and then calls the
/// relevant `get_min_distance_*` function.
fn sdf_edge_get_min_distance(
    edge: &SdfEdge,
    point: Ft26D6Vec,
    out: &mut SdfSignedDistance,
) -> FtResult<()> {
    /* edge-specific distance calculation */
    match edge.edge_type {
        SdfEdgeType::Line => {
            let _ = get_min_distance_line(edge, point, out);
        }

        SdfEdgeType::Conic => {
            let _ = get_min_distance_conic(edge, point, out);
        }

        SdfEdgeType::Cubic => {
            let _ = get_min_distance_cubic(edge, point, out);
        }

        SdfEdgeType::Undefined => return Err(FT_ERR_INVALID_ARGUMENT),
    }

    Ok(())
}

/* `sdf_generate' is not used at the moment */
/* (sdf_contour_get_min_distance, sdf_generate: #if 0) */

/// `sdf_generate_bounding_box`: This function does basically the same
/// thing as `sdf_generate` above but more efficiently.
///
/// Instead of checking all pixels against all edges, we loop over all
/// edges and only check pixels around the control box of the edge; the
/// control box is increased by the spread in all directions.  Anything
/// outside of the control box that exceeds `spread` doesn't need to be
/// computed.
///
/// Lastly, to determine the sign of unchecked pixels, we do a single
/// pass of all rows starting with a '+' sign and flipping when we come
/// across a '-' sign and continue.  This also eliminates the possibility
/// of overflow because we only check the proximity of the curve.
/// Therefore we can use squared distanced safely.
fn sdf_generate_bounding_box(
    internal_params: &SdfParams,
    shape: &SdfShape,
    spread: FtUInt,
    bitmap: &mut FtBitmap,
) -> FtResult<()> {
    let width: FtInt;
    let rows: FtInt;
    let sp_sq: FtInt; /* max value to check   */

    let fixed_spread: Ft16D16 = ft_int_16d16(spread as FtPos) as Ft16D16;

    if !(MIN_SPREAD..=MAX_SPREAD).contains(&spread) {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    /* This buffer has the same size in indices as the    */
    /* bitmap buffer.  When we check a pixel position for */
    /* a shortest distance we keep it in this buffer.     */
    /* This way we can find out which pixel is set,       */
    /* and also determine the signs properly.             */
    let n = (bitmap.width as usize).checked_mul(bitmap.rows as usize);
    let mut dists: Vec<SdfSignedDistance> = Vec::new();
    match n {
        Some(n) if dists.try_reserve_exact(n).is_ok() => {
            dists.resize(n, SdfSignedDistance::default());
        }
        _ => return Err(FT_ERR_OUT_OF_MEMORY),
    }

    width = bitmap.width as FtInt;
    rows = bitmap.rows as FtInt;
    let buffer = &mut bitmap.buffer; /* the bitmap buffer    */

    /* (USE_SQUARED_DISTANCES is 0) */
    sp_sq = fixed_spread;

    if width == 0 || rows == 0 {
        return Err(FT_ERR_CANNOT_RENDER_GLYPH);
    }

    /* loop over all contours */
    for contours in &shape.contours {
        /* loop over all edges */
        for edges in &contours.edges {
            /* get the control box and increase it by `spread' */
            let mut cbox = get_control_box(*edges);

            cbox.xMin = cbox.xMin.wrapping_sub(63) / 64 - spread as FtPos;
            cbox.xMax = cbox.xMax.wrapping_add(63) / 64 + spread as FtPos;
            cbox.yMin = cbox.yMin.wrapping_sub(63) / 64 - spread as FtPos;
            cbox.yMax = cbox.yMax.wrapping_add(63) / 64 + spread as FtPos;

            /* now loop over the pixels in the control box. */
            /* (the pixels outside of the bitmap are skipped: this loops */
            /* over the box's part in the bitmap)                         */
            let y_start = cbox.yMin.max(0);
            let y_end = cbox.yMax.min(rows as FtPos);
            let x_start = cbox.xMin.max(0);
            let x_end = cbox.xMax.min(width as FtPos);
            for y in y_start..y_end {
                let y = y as FtInt;
                for x in x_start..x_end {
                    let x = x as FtInt;
                    let mut grid_point = ZERO_VECTOR;
                    let mut dist = MAX_SDF;
                    let index: FtUInt;
                    let diff: Ft16D16;

                    grid_point.x = ft_int_26d6(x as FtPos);
                    grid_point.y = ft_int_26d6(y as FtPos);

                    /* This `grid_point` is at the corner, but we */
                    /* use the center of the pixel.               */
                    grid_point.x += ft_int_26d6(1) / 2;
                    grid_point.y += ft_int_26d6(1) / 2;

                    sdf_edge_get_min_distance(edges, grid_point, &mut dist)?;

                    if internal_params.orientation == FT_ORIENTATION_FILL_LEFT {
                        dist.sign = dist.sign.wrapping_neg();
                    }

                    /* ignore if the distance is greater than spread;       */
                    /* otherwise it creates artifacts due to the wrong sign */
                    if dist.distance > sp_sq {
                        continue;
                    }

                    /* take the square root of the distance if required */
                    /* (USE_SQUARED_DISTANCES is 0) */

                    if internal_params.flip_y {
                        index = (y * width + x) as FtUInt;
                    } else {
                        index = ((rows - y - 1) * width + x) as FtUInt;
                    }
                    let index = index as usize;

                    /* check whether the pixel is set or not */
                    if dists[index].sign == 0 {
                        dists[index] = dist;
                    } else {
                        diff = dists[index]
                            .distance
                            .wrapping_sub(dist.distance)
                            .wrapping_abs();

                        if diff <= CORNER_CHECK_EPSILON {
                            dists[index] = resolve_corner(dists[index], dist);
                        } else if dists[index].distance > dist.distance {
                            dists[index] = dist;
                        }
                    }
                }
            }
        }
    }

    /* final pass */
    for j in 0..rows {
        /* We assume the starting pixel of each row is outside. */
        let mut current_sign: i8 = -1;

        if internal_params.overload_sign != 0 {
            current_sign = if internal_params.overload_sign < 0 {
                -1
            } else {
                1
            };
        }

        for i in 0..width {
            let index = (j * width + i) as usize;

            /* if the pixel is not set                     */
            /* its shortest distance is more than `spread` */
            if dists[index].sign == 0 {
                dists[index].distance = fixed_spread;
            } else {
                current_sign = dists[index].sign;
            }

            /* clamp the values */
            if dists[index].distance > fixed_spread {
                dists[index].distance = fixed_spread;
            }

            /* flip sign if required */
            let sign = if internal_params.flip_sign {
                current_sign.wrapping_neg()
            } else {
                current_sign
            };
            dists[index].distance = dists[index].distance.wrapping_mul(sign as Ft16D16);

            /* concatenate to appropriate format */
            if let Some(b) = buffer.get_mut(index) {
                *b = map_fixed_to_sdf(dists[index].distance, fixed_spread);
            }
        }
    }

    /* (dropping `dists' is FT_FREE) */
    Ok(())
}

/// `sdf_generate_subdivision`: Subdivide the shape into a number of
/// straight lines, then use the above `sdf_generate_bounding_box` function
/// to generate the SDF.
///
/// Note: After calling this function `shape` no longer has the original
/// edges, it only contains lines.
fn sdf_generate_subdivision(
    internal_params: &SdfParams,
    shape: &mut SdfShape,
    spread: FtUInt,
    bitmap: &mut FtBitmap,
) -> FtResult<()> {
    /*
     * Thanks to Alexei for providing the idea of this optimization.
     *
     * We take advantage of two facts.
     *
     * (1) Computing the shortest distance from a point to a line segment is
     *     very fast.
     * (2) We don't have to compute the shortest distance for the entire
     *     two-dimensional grid.
     *
     * Both ideas lead to the following optimization.
     *
     * (1) Split the outlines into a number of line segments.
     *
     * (2) For each line segment, only process its neighborhood.
     *
     * (3) Compute the closest distance to the line only for neighborhood
     *     grid points.
     *
     * This greatly reduces the number of grid points to check.
     */

    split_sdf_shape(shape)?;
    sdf_generate_bounding_box(internal_params, shape, spread, bitmap)?;

    Ok(())
}

/// `sdf_generate_with_overlaps`: This function can be used to generate SDF
/// for glyphs with overlapping contours.  The function generates SDF for
/// contours separately on separate bitmaps (to generate SDF it uses
/// `sdf_generate_subdivision`).  At the end it simply combines all the SDF
/// into the output bitmap; this fixes all the signs and removes overlaps.
///
/// The function cannot generate a proper SDF for glyphs with
/// self-intersecting contours because we cannot separate them into two
/// separate bitmaps.  In case of self-intersecting contours it is
/// necessary to remove the overlaps before generating the SDF.
fn sdf_generate_with_overlaps(
    mut internal_params: SdfParams,
    shape: &mut SdfShape,
    spread: FtUInt,
    bitmap: &mut FtBitmap,
) -> FtResult<()> {
    let num_contours: FtInt; /* total number of contours      */
    let width: FtInt; /* width and rows of the bitmap  */
    let rows: FtInt;

    /* Disable `flip_sign` to avoid extra complication */
    /* during the combination phase.                   */
    let flip_sign = internal_params.flip_sign; /* flip sign?                    */
    internal_params.flip_sign = false;

    width = bitmap.width as FtInt;
    rows = bitmap.rows as FtInt;

    /* find the number of contours in the shape */
    num_contours = shape.contours.len() as FtInt;

    /* allocate the bitmaps to generate SDF for separate contours */
    let mut bitmaps: Vec<FtBitmap> = Vec::new(); /* separate bitmaps for contours */
    if bitmaps.try_reserve_exact(num_contours as usize).is_err() {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }

    /* allocate array to hold orientation for all contours */
    let mut orientations: Vec<SdfContourOrientation> = Vec::new();
    if orientations
        .try_reserve_exact(num_contours as usize)
        .is_err()
    {
        return Err(FT_ERR_OUT_OF_MEMORY);
    }

    let mut head: VecDeque<SdfContour> = VecDeque::new(); /* head of the contour list      */
    let contours = std::mem::take(&mut shape.contours);

    /* Iterate over all contours and generate SDF separately. */
    /* (FIXME (upstream): C keeps iterating through the `next' field of */
    /* the contour that `split_sdf_shape' has freed; this iterates over */
    /* the original contours)                                          */
    for (i, contour) in contours.into_iter().enumerate() {
        /* initialize the corresponding bitmap */
        let mut bm = FtBitmap {
            width: bitmap.width,
            rows: bitmap.rows,
            pitch: bitmap.pitch,
            num_grays: bitmap.num_grays,
            pixel_mode: bitmap.pixel_mode,
            ..FtBitmap::default()
        };

        /* allocate memory for the buffer */
        bm.buffer = super::super::base::ftmemory::ft_alloc(
            (bitmap.rows as FtLong).wrapping_mul(bitmap.pitch as FtUInt as FtLong),
        )?;
        bitmaps.push(bm);

        /* determine the orientation */
        orientations.push(get_contour_orientation(&contour));

        /* The `overload_sign` property is specific to  */
        /* `sdf_generate_bounding_box`.  This basically */
        /* overloads the default sign of the outside    */
        /* pixels, which is necessary for               */
        /* counter-clockwise contours.                  */
        if orientations[i] == SdfContourOrientation::Ccw
            && internal_params.orientation == FT_ORIENTATION_FILL_RIGHT
        {
            internal_params.overload_sign = 1;
        } else if orientations[i] == SdfContourOrientation::Cw
            && internal_params.orientation == FT_ORIENTATION_FILL_LEFT
        {
            internal_params.overload_sign = 1;
        } else {
            internal_params.overload_sign = 0;
        }

        /* Make `contour->next` NULL so that there is   */
        /* one contour in the list.  Also hold the next */
        /* contour in a temporary variable so as to     */
        /* restore the original value.                  */

        /* Use `temp_shape` to hold the new contour. */
        /* Now, `temp_shape` has only one contour.   */
        let mut temp_shape = SdfShape {
            contours: VecDeque::from([contour]),
        };

        /* finally generate the SDF */
        sdf_generate_subdivision(&internal_params, &mut temp_shape, spread, &mut bitmaps[i])?;

        /* Restore the original `next` variable. */

        /* Since `split_sdf_shape` deallocated the original */
        /* contours list we need to assign the new value to */
        /* the shape's contour.                             */
        if let Some(c) = temp_shape.contours.pop_front() {
            sdf_push_front(&mut head, c)?;
        }

        /* Simply flip the orientation in case of post-script fonts */
        /* so as to avoid modificatons in the combining phase.      */
        if internal_params.orientation == FT_ORIENTATION_FILL_LEFT {
            if orientations[i] == SdfContourOrientation::Cw {
                orientations[i] = SdfContourOrientation::Ccw;
            } else if orientations[i] == SdfContourOrientation::Ccw {
                orientations[i] = SdfContourOrientation::Cw;
            }
        }
    }

    /* assign the new contour list to `shape->contours` */
    shape.contours = head;

    /* cast the output bitmap buffer */
    let t = &mut bitmap.buffer; /* target bitmap buffer          */

    /* Iterate over all pixels and combine all separate    */
    /* contours.  These are the rules for combining:       */
    /*                                                     */
    /* (1) For all clockwise contours, compute the largest */
    /*     value.  Name this as `val_c`.                   */
    /* (2) For all counter-clockwise contours, compute the */
    /*     smallest value.  Name this as `val_ac`.         */
    /* (3) Now, finally use the smaller value of `val_c'   */
    /*     and `val_ac'.                                   */
    for j in 0..rows {
        for i in 0..width {
            let id = (j * width + i) as usize; /* index of current pixel    */

            let mut val_c: FtSdfFormat = 0; /* max clockwise value       */
            let mut val_ac: FtSdfFormat = u8::MAX; /* min counter-clockwise val */

            /* iterate through all the contours */
            for c in 0..num_contours as usize {
                /* current contour value */
                let temp: FtSdfFormat = bitmaps[c].buffer.get(id).copied().unwrap_or(0);

                if orientations[c] == SdfContourOrientation::Cw {
                    val_c = val_c.max(temp); /* clockwise         */
                } else {
                    val_ac = val_ac.min(temp); /* counter-clockwise */
                }
            }

            /* Finally find the smaller of the two and assign to output. */
            /* Also apply `flip_sign` if set.                            */
            if let Some(t) = t.get_mut(id) {
                *t = val_c.min(val_ac);

                if flip_sign {
                    *t = invert_sign(*t);
                }
            }
        }
    }

    /* Exit: */
    /* deallocate orientations array */

    /* deallocate temporary bitmaps */
    /* (with no contours, C's `bitmaps' is a NULL allocation of 0 bytes, */
    /* so its `Raster_Corrupted' error is never returned)                */

    /* restore the `flip_sign` property */

    Ok(())
}

/**************************************************************************
 *
 * interface functions
 *
 */

/* sdf_raster_new, sdf_raster_reset, sdf_raster_done: the raster holds */
/* no state                                                            */

/// `sdf_raster_set_mode`
pub fn sdf_raster_set_mode(_mode: FtULong) -> FtResult<()> {
    Ok(())
}

/// `sdf_raster_render`
fn sdf_raster_render(source: FtRasterSource, params: &mut FtRasterParams) -> FtResult<()> {
    let sdf_params = params.sdf;

    let outline = match source {
        FtRasterSource::Outline(o) => o,
        /* check whether outline is valid */
        FtRasterSource::Bitmap(_) => return Err(FT_ERR_INVALID_OUTLINE),
    };

    /* if the outline is empty, return */
    if outline.n_points <= 0 || outline.n_contours <= 0 {
        return Ok(());
    }

    /* check whether the outline has valid fields */
    if outline.contours.is_empty() || outline.points.is_empty() {
        return Err(FT_ERR_INVALID_OUTLINE);
    }

    /* check whether spread is set properly */
    if sdf_params.spread > MAX_SPREAD || sdf_params.spread < MIN_SPREAD {
        return Err(FT_ERR_INVALID_ARGUMENT);
    }

    let Some(target) = params.target.as_deref_mut() else {
        return Err(FT_ERR_INVALID_ARGUMENT);
    };

    /* set up the parameters */
    let internal_params = SdfParams {
        orientation: ft_outline_get_orientation(outline),
        flip_sign: sdf_params.flip_sign,
        flip_y: sdf_params.flip_y,
        overload_sign: 0,
    };

    let mut shape = SdfShape::default();

    sdf_outline_decompose(outline, &mut shape)?;

    if sdf_params.overlaps {
        sdf_generate_with_overlaps(internal_params, &mut shape, sdf_params.spread, target)?;
    } else {
        sdf_generate_subdivision(&internal_params, &mut shape, sdf_params.spread, target)?;
    }

    /* (dropping `shape' is sdf_shape_done) */
    Ok(())
}

/// `ft_sdf_raster`
pub static FT_SDF_RASTER: FtRasterFuncs = FtRasterFuncs {
    glyph_format: FT_GLYPH_FORMAT_OUTLINE,

    raster_render: sdf_raster_render, /* raster_render   */
};
