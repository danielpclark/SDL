// Rust translation of lib/jxl/image.h, lib/jxl/image.cc and the parts of
// lib/jxl/image_ops.h the decoder uses, from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! SIMD/multicore-friendly planar image representation with row accessors.
//!
//! The planes own their storage in a `Vec`, which is allocated fallibly
//! (upstream aborts when an allocation fails; here constructing a plane
//! returns an error) and zeroed. Rows are separated by upstream's padding
//! (as `BytesPerRow()` computes it for highway's scalar target), so the
//! strides, and code that reads past a row's last valid sample, are
//! upstream's.

use super::base::{jxl_failure, round_up_to, StatusCode};

/// The types planes hold.
pub(crate) trait Pixel: Copy + Default + 'static {}
impl Pixel for f32 {}
impl Pixel for i32 {}
impl Pixel for i16 {}
impl Pixel for u8 {}
impl Pixel for i8 {}
impl Pixel for u32 {}
impl Pixel for u16 {}
impl Pixel for f64 {}

// (CacheAligned::kAlignment and kAlias)
const K_ALIGNMENT: usize = 2 * 64;
const K_ALIAS: usize = 2048;

/// Returns distance [bytes] between the start of two consecutive rows, a
/// multiple of vector/cache line size but NOT CacheAligned::kAlias - see
/// below. Translation of `BytesPerRow()` (for the scalar target, whose
/// vector size is one byte).
fn bytes_per_row(xsize: usize, sizeof_t: usize) -> usize {
    let vec_size = 1usize; // HWY_LANES(uint8_t) on HWY_SCALAR
    let mut valid_bytes = xsize.wrapping_mul(sizeof_t);

    // Allow unaligned accesses starting at the last valid value - this may raise
    // msan errors unless the user calls InitializePaddingForUnalignedAccesses.
    // Skip for the scalar case because no extra lanes will be loaded.
    if vec_size != 0 {
        valid_bytes = valid_bytes.wrapping_add(vec_size.wrapping_sub(sizeof_t));
    }

    // Round up to vector and cache line size.
    let align = vec_size.max(K_ALIGNMENT);
    let mut bytes_per_row = round_up_to(valid_bytes, align);

    // During the lengthy window before writes are committed to memory, CPUs
    // guard against read after write hazards by checking the address, but
    // only the lower 11 bits. We avoid a false dependency between writes to
    // consecutive rows by ensuring their sizes are not multiples of 2 KiB.
    // Avoid2K prevents the same problem for the planes of an Image3.
    if bytes_per_row % K_ALIAS == 0 {
        bytes_per_row += align;
    }

    bytes_per_row
}

/// Single channel, aligned rows separated by padding. Translation of
/// `Plane<T>` (with `PlaneBase`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Plane<T: Pixel> {
    xsize: u32, // In valid pixels, not including any padding.
    ysize: u32,
    orig_xsize: u32,
    orig_ysize: u32,
    // (bytes_per_row_ / sizeof(T))
    stride: usize,
    data: Vec<T>,
}

impl<T: Pixel> Plane<T> {
    /// An empty plane. Translation of the default constructor.
    pub(crate) fn empty() -> Self {
        Plane {
            xsize: 0,
            ysize: 0,
            orig_xsize: 0,
            orig_ysize: 0,
            stride: 0,
            data: Vec::new(),
        }
    }

    /// Translation of the `Plane(xsize, ysize)` constructor (failing where
    /// upstream's allocation check aborts).
    pub(crate) fn new(xsize: usize, ysize: usize) -> Result<Self, StatusCode> {
        if xsize > u32::MAX as usize || ysize > u32::MAX as usize {
            // JXL_CHECK(xsize == xsize_)
            return jxl_failure!("image too large");
        }
        let mut p = Plane {
            xsize: xsize as u32,
            ysize: ysize as u32,
            orig_xsize: xsize as u32,
            orig_ysize: ysize as u32,
            stride: 0,
            data: Vec::new(),
        };
        // Dimensions can be zero, e.g. for lazily-allocated images. Only allocate
        // if nonzero, because "zero" bytes still have padding/bookkeeping overhead.
        if xsize != 0 && ysize != 0 {
            let sizeof_t = std::mem::size_of::<T>();
            let bpr = bytes_per_row(xsize, sizeof_t);
            p.stride = bpr / sizeof_t;
            let Some(n) = p.stride.checked_mul(ysize) else {
                return jxl_failure!("image too large");
            };
            if n.checked_mul(sizeof_t)
                .is_none_or(|b| b > isize::MAX as usize)
            {
                return jxl_failure!("image too large");
            }
            if p.data.try_reserve_exact(n).is_err() {
                return jxl_failure!("out of memory");
            }
            p.data.resize(n, T::default());
        }
        Ok(p)
    }

    pub(crate) fn swap(&mut self, other: &mut Plane<T>) {
        std::mem::swap(self, other);
    }

    /// Useful for pre-allocating image with some padding for alignment
    /// purposes and later reporting the actual valid dimensions. May also be
    /// used to un-shrink the image. Translation of `ShrinkTo()`.
    pub(crate) fn shrink_to(&mut self, xsize: usize, ysize: usize) {
        debug_assert!(xsize <= self.orig_xsize as usize);
        debug_assert!(ysize <= self.orig_ysize as usize);
        self.xsize = xsize as u32;
        self.ysize = ysize as u32;
        // NOTE: we can't recompute bytes_per_row for more compact storage and
        // better locality because that would invalidate the image contents.
    }

    // How many pixels.
    #[inline]
    pub(crate) fn xsize(&self) -> usize {
        self.xsize as usize
    }
    #[inline]
    pub(crate) fn ysize(&self) -> usize {
        self.ysize as usize
    }

    /// Returns number of pixels (some of which are padding) per row.
    /// Translation of `PixelsPerRow()`.
    #[inline]
    pub(crate) fn pixels_per_row(&self) -> usize {
        self.stride
    }

    /// The row `y`, with its padding (to the end of the storage on the last
    /// row). Translation of `Row()`.
    #[inline]
    pub(crate) fn row(&self, y: usize) -> &[T] {
        &self.data[y * self.stride..]
    }

    /// Translation of `Row()` (mutable).
    #[inline]
    pub(crate) fn row_mut(&mut self, y: usize) -> &mut [T] {
        let s = y * self.stride;
        &mut self.data[s..]
    }

    /// The whole storage, rows `stride` apart.
    #[inline]
    pub(crate) fn data(&self) -> &[T] {
        &self.data
    }

    /// The whole storage (mutable).
    #[inline]
    pub(crate) fn data_mut(&mut self) -> &mut [T] {
        &mut self.data
    }
}

/// Translation of `SameSize()`.
pub(crate) fn same_size<T: Pixel, U: Pixel>(a: &Plane<T>, b: &Plane<U>) -> bool {
    a.xsize() == b.xsize() && a.ysize() == b.ysize()
}

pub(crate) type ImageB = Plane<u8>;
pub(crate) type ImageSB = Plane<i8>;
pub(crate) type ImageI = Plane<i32>;
pub(crate) type ImageF = Plane<f32>;

/// Rectangular region in image(s). Factoring this out of Image instead of
/// shifting the pointer by x0/y0 allows this to apply to multiple images
/// with different resolutions (e.g. color transform and quantization
/// field). Translation of `Rect` (`RectT<size_t>`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Rect {
    x0: usize,
    y0: usize,
    xsize: usize,
    ysize: usize,
}

impl Rect {
    /// Most windows are xsize_max * ysize_max, except those on the borders
    /// where begin + size_max > end.
    pub(crate) fn new_clamped(
        xbegin: usize,
        ybegin: usize,
        xsize_max: usize,
        ysize_max: usize,
        xend: usize,
        yend: usize,
    ) -> Rect {
        Rect {
            x0: xbegin,
            y0: ybegin,
            xsize: Self::clamped_size(xbegin, xsize_max, xend),
            ysize: Self::clamped_size(ybegin, ysize_max, yend),
        }
    }

    /// Construct with origin and known size (typically from another Rect).
    pub(crate) const fn new(xbegin: usize, ybegin: usize, xsize: usize, ysize: usize) -> Rect {
        Rect {
            x0: xbegin,
            y0: ybegin,
            xsize,
            ysize,
        }
    }

    /// Construct a rect that covers a whole plane.
    pub(crate) fn from_plane<T: Pixel>(image: &Plane<T>) -> Rect {
        Rect::new(0, 0, image.xsize(), image.ysize())
    }

    /// Construct a rect that covers a whole image.
    pub(crate) fn from_image3<T: Pixel>(image: &Image3<T>) -> Rect {
        Rect::new(0, 0, image.xsize(), image.ysize())
    }

    /// Construct a subrect that resides in the [0, ysize) x [0, xsize) region
    /// of the current rect. Translation of `Crop(xsize, ysize)`.
    pub(crate) fn crop(&self, area_xsize: usize, area_ysize: usize) -> Rect {
        self.intersection(&Rect::new(0, 0, area_xsize, area_ysize))
    }

    /// Construct a subrect that resides in a plane. Translation of
    /// `Crop(image)`.
    pub(crate) fn crop_plane<T: Pixel>(&self, image: &Plane<T>) -> Rect {
        self.intersection(&Rect::from_plane(image))
    }

    /// Returns a rect that only contains `num` lines with offset `y` from
    /// `y0()`.
    pub(crate) fn lines(&self, y: usize, num: usize) -> Rect {
        Rect::new(self.x0, self.y0 + y, self.xsize, num)
    }

    pub(crate) fn line(&self, y: usize) -> Rect {
        self.lines(y, 1)
    }

    pub(crate) fn intersection(&self, other: &Rect) -> Rect {
        Rect::new_clamped(
            self.x0.max(other.x0),
            self.y0.max(other.y0),
            self.xsize,
            self.ysize,
            self.x1().min(other.x1()),
            self.y1().min(other.y1()),
        )
    }

    pub(crate) fn translate(&self, x_offset: i64, y_offset: i64) -> Rect {
        Rect::new(
            (self.x0 as i64).wrapping_add(x_offset) as usize,
            (self.y0 as i64).wrapping_add(y_offset) as usize,
            self.xsize,
            self.ysize,
        )
    }

    pub(crate) fn is_inside(&self, other: &Rect) -> bool {
        self.x0 >= other.x0
            && self.x1() <= other.x1()
            && self.y0 >= other.y0
            && self.y1() <= other.y1()
    }

    pub(crate) fn is_inside_plane<T: Pixel>(&self, image: &Plane<T>) -> bool {
        self.is_inside(&Rect::from_plane(image))
    }

    #[inline]
    pub(crate) fn x0(&self) -> usize {
        self.x0
    }
    #[inline]
    pub(crate) fn y0(&self) -> usize {
        self.y0
    }
    #[inline]
    pub(crate) fn xsize(&self) -> usize {
        self.xsize
    }
    #[inline]
    pub(crate) fn ysize(&self) -> usize {
        self.ysize
    }
    #[inline]
    pub(crate) fn x1(&self) -> usize {
        self.x0.wrapping_add(self.xsize)
    }
    #[inline]
    pub(crate) fn y1(&self) -> usize {
        self.y0.wrapping_add(self.ysize)
    }

    pub(crate) fn shift_left(&self, shiftx: usize, shifty: usize) -> Rect {
        Rect::new(
            self.x0 * (1 << shiftx),
            self.y0 * (1 << shifty),
            self.xsize << shiftx,
            self.ysize << shifty,
        )
    }

    /// Requires x0(), y0() to be multiples of 1<<shiftx, 1<<shifty.
    pub(crate) fn ceil_shift_right(&self, shiftx: usize, shifty: usize) -> Rect {
        debug_assert!(self.x0 % (1 << shiftx) == 0);
        debug_assert!(self.y0 % (1 << shifty) == 0);
        Rect::new(
            self.x0 / (1 << shiftx),
            self.y0 / (1 << shifty),
            super::base::div_ceil(self.xsize, 1 << shiftx),
            super::base::div_ceil(self.ysize, 1 << shifty),
        )
    }

    // Returns size_max, or whatever is left in [begin, end).
    const fn clamped_size(begin: usize, size_max: usize, end: usize) -> usize {
        if begin.wrapping_add(size_max) <= end {
            size_max
        } else if end > begin {
            end - begin
        } else {
            0
        }
    }

    /// The index of `image`'s sample at (x0, y0 + y). Translation of
    /// `Rect::Row()` (as an index into the plane's storage).
    #[inline]
    pub(crate) fn row_index<T: Pixel>(&self, image: &Plane<T>, y: usize) -> usize {
        (y + self.y0) * image.pixels_per_row() + self.x0
    }

    /// The row of `image` this rect's row `y` starts at. Translation of
    /// `Rect::ConstRow()`.
    #[inline]
    pub(crate) fn const_row<'a, T: Pixel>(&self, image: &'a Plane<T>, y: usize) -> &'a [T] {
        &image.data()[self.row_index(image, y)..]
    }

    /// Translation of `Rect::Row()` (mutable).
    #[inline]
    pub(crate) fn row<'a, T: Pixel>(&self, image: &'a mut Plane<T>, y: usize) -> &'a mut [T] {
        let i = self.row_index(image, y);
        &mut image.data_mut()[i..]
    }

    /// Translation of `Rect::ConstPlaneRow()`.
    #[inline]
    pub(crate) fn const_plane_row<'a, T: Pixel>(
        &self,
        image: &'a Image3<T>,
        c: usize,
        y: usize,
    ) -> &'a [T] {
        self.const_row(image.plane(c), y)
    }

    /// Translation of `Rect::PlaneRow()`.
    #[inline]
    pub(crate) fn plane_row<'a, T: Pixel>(
        &self,
        image: &'a mut Image3<T>,
        c: usize,
        y: usize,
    ) -> &'a mut [T] {
        self.row(image.plane_mut(c), y)
    }
}

/// A bundle of 3 same-sized images. Translation of `Image3<T>`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Image3<T: Pixel> {
    planes: [Plane<T>; 3],
}

impl<T: Pixel> Image3<T> {
    pub(crate) fn empty() -> Self {
        Image3 {
            planes: [Plane::empty(), Plane::empty(), Plane::empty()],
        }
    }

    pub(crate) fn new(xsize: usize, ysize: usize) -> Result<Self, StatusCode> {
        Ok(Image3 {
            planes: [
                Plane::new(xsize, ysize)?,
                Plane::new(xsize, ysize)?,
                Plane::new(xsize, ysize)?,
            ],
        })
    }

    pub(crate) fn from_planes(p0: Plane<T>, p1: Plane<T>, p2: Plane<T>) -> Self {
        debug_assert!(same_size(&p0, &p1));
        debug_assert!(same_size(&p0, &p2));
        Image3 {
            planes: [p0, p1, p2],
        }
    }

    /// Returns row slice; usage: plane_row(idx_plane, y)[x] = val.
    #[inline]
    pub(crate) fn plane_row(&mut self, c: usize, y: usize) -> &mut [T] {
        self.planes[c].row_mut(y)
    }

    /// Returns const row slice.
    #[inline]
    pub(crate) fn const_plane_row(&self, c: usize, y: usize) -> &[T] {
        self.planes[c].row(y)
    }

    #[inline]
    pub(crate) fn plane(&self, idx: usize) -> &Plane<T> {
        &self.planes[idx]
    }
    #[inline]
    pub(crate) fn plane_mut(&mut self, idx: usize) -> &mut Plane<T> {
        &mut self.planes[idx]
    }

    /// The three planes, mutably.
    #[inline]
    pub(crate) fn planes_mut(&mut self) -> &mut [Plane<T>; 3] {
        &mut self.planes
    }

    pub(crate) fn swap(&mut self, other: &mut Image3<T>) {
        std::mem::swap(self, other);
    }

    pub(crate) fn shrink_to(&mut self, xsize: usize, ysize: usize) {
        for plane in self.planes.iter_mut() {
            plane.shrink_to(xsize, ysize);
        }
    }

    // Sizes of all three images are guaranteed to be equal.
    #[inline]
    pub(crate) fn xsize(&self) -> usize {
        self.planes[0].xsize()
    }
    #[inline]
    pub(crate) fn ysize(&self) -> usize {
        self.planes[0].ysize()
    }

    #[inline]
    pub(crate) fn pixels_per_row(&self) -> usize {
        self.planes[0].pixels_per_row()
    }
}

pub(crate) type Image3F = Image3<f32>;

// --- image_ops.h ---

/// Translation of `CopyImageTo(from, to)` on planes.
pub(crate) fn copy_image_to<T: Pixel>(from: &Plane<T>, to: &mut Plane<T>) {
    debug_assert!(same_size(from, to));
    if from.ysize() == 0 || from.xsize() == 0 {
        return;
    }
    for y in 0..from.ysize() {
        let n = from.xsize();
        to.row_mut(y)[..n].copy_from_slice(&from.row(y)[..n]);
    }
}

/// DEPRECATED - prefer to preallocate result. Translation of
/// `CopyImage()`.
pub(crate) fn copy_image<T: Pixel>(from: &Plane<T>) -> Result<Plane<T>, StatusCode> {
    let mut to = Plane::new(from.xsize(), from.ysize())?;
    copy_image_to(from, &mut to);
    Ok(to)
}

/// Copies `from:rect_from` to `to:rect_to`. Translation of
/// `CopyImageTo(rect_from, from, rect_to, to)`.
pub(crate) fn copy_image_to_rect<T: Pixel>(
    rect_from: &Rect,
    from: &Plane<T>,
    rect_to: &Rect,
    to: &mut Plane<T>,
) {
    if rect_from.xsize() == 0 {
        return;
    }
    let n = rect_from.xsize();
    for y in 0..rect_from.ysize() {
        let src = &rect_from.const_row(from, y)[..n];
        rect_to.row(to, y)[..n].copy_from_slice(src);
    }
}

/// Translation of `CopyImageTo(rect_from, from, rect_to, to)` on Image3.
pub(crate) fn copy_image3_to_rect<T: Pixel>(
    rect_from: &Rect,
    from: &Image3<T>,
    rect_to: &Rect,
    to: &mut Image3<T>,
) {
    for c in 0..3 {
        copy_image_to_rect(rect_from, from.plane(c), rect_to, to.plane_mut(c));
    }
}

/// Translation of `FillImage()` on a plane.
pub(crate) fn fill_image<T: Pixel>(value: T, image: &mut Plane<T>) {
    for y in 0..image.ysize() {
        let n = image.xsize();
        image.row_mut(y)[..n].fill(value);
    }
}

/// Translation of `ZeroFillImage()` on a plane.
pub(crate) fn zero_fill_image<T: Pixel>(image: &mut Plane<T>) {
    if image.xsize() == 0 {
        return;
    }
    fill_image(T::default(), image);
}

/// Translation of `ZeroFillImage()` on an Image3.
pub(crate) fn zero_fill_image3<T: Pixel>(image: &mut Image3<T>) {
    for c in 0..3 {
        zero_fill_image(image.plane_mut(c));
    }
}

/// Translation of `FillPlane(value, image, rect)`.
pub(crate) fn fill_plane_rect<T: Pixel>(value: T, image: &mut Plane<T>, rect: &Rect) {
    for y in 0..rect.ysize() {
        let n = rect.xsize();
        rect.row(image, y)[..n].fill(value);
    }
}

/// Translation of `ZeroFillPlane(image, rect)`.
pub(crate) fn zero_fill_plane_rect<T: Pixel>(image: &mut Plane<T>, rect: &Rect) {
    fill_plane_rect(T::default(), image, rect);
}

/// Translation of `FillImage(value, image, rect)` on an Image3.
#[allow(dead_code)]
pub(crate) fn fill_image3_rect<T: Pixel>(value: T, image: &mut Image3<T>, rect: &Rect) {
    for c in 0..3 {
        fill_plane_rect(value, image.plane_mut(c), rect);
    }
}

/// Mirrors out of bounds coordinates and returns valid coordinates
/// unchanged. We assume the radius (distance outside the image) is small
/// compared to the image size, otherwise this might not terminate. The
/// mirror is outside the last column (border pixel is also replicated).
/// Translation of `Mirror()`.
#[inline]
pub(crate) fn mirror(mut x: i64, xsize: i64) -> i64 {
    debug_assert!(xsize != 0);

    // TODO(janwas): replace with branchless version
    while x < 0 || x >= xsize {
        if x < 0 {
            x = -x - 1;
        } else {
            x = 2 * xsize - 1 - x;
        }
    }
    x
}
