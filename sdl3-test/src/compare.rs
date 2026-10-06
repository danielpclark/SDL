// Rust translation of src/test/SDL_test_compare.c and include/SDL3/SDL_test_compare.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Comparison function of SDL test framework.
//!
//! Defines comparison functions (i.e. for surfaces).
//!
//! Based on automated SDL_Surface tests originally written by Edgar Simo
//! 'bobbens'.
//!
//! Rewritten for test lib by Andreas Schiffler.

use std::sync::atomic::{AtomicI32, Ordering};

use sdl3::video::pixels::Color;
use sdl3::video::surface::Surface;
use sdl3::{Error, Result};

use crate::internal::isprint;
use crate::{assert_check, log_error};

/* Counter for _CompareSurface calls; used for filename creation when comparisons fail */
static COMPARE_SURFACE_COUNT: AtomicI32 = AtomicI32::new(0);

/// Run `f` on the surface locked (or, as the C code which ignores a failed
/// lock, unlocked).
fn with_locked<R>(surface: &mut Surface<'_>, f: impl FnOnce(&Surface<'_>) -> R) -> R {
    // (SDL_LockSurface(), and SDL_UnlockSurface() when the guard drops)
    if let Ok(locked) = surface.lock() {
        return f(locked.surface());
    }
    f(surface)
}

/// The comparison of [`compare_surfaces`] and
/// [`compare_surfaces_ignore_transparent_pixels`].
fn compare(
    surface: &mut Surface<'_>,
    reference_surface: &mut Surface<'_>,
    mut allowable_error: i32,
    ignore_transparent_pixels: bool,
) -> Result<u32> {
    /* Make sure surface size is the same. */
    if surface.width() != reference_surface.width()
        || surface.height() != reference_surface.height()
    {
        let message = format!(
            "Expected {}x{} surface, got {}x{}",
            reference_surface.width(),
            reference_surface.height(),
            surface.width(),
            surface.height()
        );
        log_error!("{}", message);
        return Err(Error::new(message));
    }

    /* Sanitize input value */
    if allowable_error < 0 {
        allowable_error = 0;
    }

    let mut sample_error_x = 0;
    let mut sample_error_y = 0;
    let mut sample_dist = 0;
    let mut sample_reference = Color::new(0, 0, 0, 0);
    let mut sample_actual = Color::new(0, 0, 0, 0);

    let mut ret = 0u32;
    with_locked(surface, |surface| {
        with_locked(reference_surface, |reference_surface| {
            /* Compare image - should be same format. */
            for j in 0..surface.height() {
                for i in 0..surface.width() {
                    let actual = match surface.read_pixel(i, j) {
                        Ok(actual) => actual,
                        Err(error) => {
                            log_error!(
                                "Failed to retrieve pixel ({},{}): {}",
                                i,
                                j,
                                error.message()
                            );
                            ret += 1;
                            continue;
                        }
                    };

                    let reference = match reference_surface.read_pixel(i, j) {
                        Ok(reference) => reference,
                        Err(error) => {
                            log_error!(
                                "Failed to retrieve reference pixel ({},{}): {}",
                                i,
                                j,
                                error.message()
                            );
                            ret += 1;
                            continue;
                        }
                    };
                    if ignore_transparent_pixels && reference.a == 0 {
                        // (SDL_ALPHA_TRANSPARENT)
                        continue;
                    }

                    let (r, g, b) = (actual.r as i32, actual.g as i32, actual.b as i32);
                    let (rd, gd, bd) = (reference.r as i32, reference.g as i32, reference.b as i32);
                    let mut dist = 0;
                    dist += (r - rd) * (r - rd);
                    dist += (g - gd) * (g - gd);
                    dist += (b - bd) * (b - bd);

                    /* Allow some difference in blending accuracy */
                    if dist > allowable_error {
                        ret += 1;
                        if ret == 1 {
                            sample_error_x = i;
                            sample_error_y = j;
                            sample_dist = dist;
                            sample_reference = reference;
                            sample_actual = actual;
                        }
                    }
                }
            }
        })
    });

    /* Save test image and reference for analysis on failures */
    let count = COMPARE_SURFACE_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if ret != 0 {
        log_error!(
            "Comparison of pixels with allowable error of {} failed {} times.",
            allowable_error,
            ret
        );
        log_error!(
            "Reference surface format: {}",
            reference_surface.format().name()
        );
        log_error!("Actual surface format: {}", surface.format().name());
        log_error!(
            "First detected occurrence at position {},{} with a squared RGB-difference of {}.",
            sample_error_x,
            sample_error_y,
            sample_dist
        );
        log_error!(
            "Reference pixel: R={} G={} B={} A={}",
            sample_reference.r,
            sample_reference.g,
            sample_reference.b,
            sample_reference.a
        );
        log_error!(
            "Actual pixel   : R={} G={} B={} A={}",
            sample_actual.r,
            sample_actual.g,
            sample_actual.b,
            sample_actual.a
        );
        let image_filename = format!("CompareSurfaces{count:04}_TestOutput.bmp");
        let _ = surface.save_bmp(&image_filename);
        let reference_filename = format!("CompareSurfaces{count:04}_Reference.bmp");
        let _ = reference_surface.save_bmp(&reference_filename);
        log_error!(
            "Surfaces from failed comparison saved as '{}' and '{}'",
            image_filename,
            reference_filename
        );
    }

    Ok(ret)
}

/// Compares a surface and with reference image data for equality.
///
/// `allowable_error` is the allowable difference (=sum of squared
/// difference for each RGB component) in blending accuracy.
///
/// Returns the number of pixels for which the comparison failed (0 if it
/// succeeded), or an error if the surface sizes differ. When pixels differ,
/// the first is described in the log and both surfaces are saved as
/// `CompareSurfacesNNNN_TestOutput.bmp` and
/// `CompareSurfacesNNNN_Reference.bmp` in the current directory.
/// Translation of `SDLTest_CompareSurfaces()`.
pub fn compare_surfaces(
    surface: &mut Surface<'_>,
    reference_surface: &mut Surface<'_>,
    allowable_error: i32,
) -> Result<u32> {
    compare(surface, reference_surface, allowable_error, false)
}

/// As [`compare_surfaces`], but pixels that are transparent in the
/// reference surface are not compared. Translation of
/// `SDLTest_CompareSurfacesIgnoreTransparentPixels()`.
pub fn compare_surfaces_ignore_transparent_pixels(
    surface: &mut Surface<'_>,
    reference_surface: &mut Surface<'_>,
    allowable_error: i32,
) -> Result<u32> {
    compare(surface, reference_surface, allowable_error, true)
}

const WIDTH: usize = 16;

/// Compares 2 memory blocks for equality, with assertions on their sizes
/// and contents. When they differ, both are logged as hex dumps side by
/// side (`actual` displayed on the left, `reference` on the right).
///
/// Returns `true` if the blocks are equal. Translation of
/// `SDLTest_CompareMemory()` (which returns 0 if they are equal).
pub fn compare_memory(actual: &[u8], reference: &[u8]) -> bool {
    let size_max = actual.len().max(reference.len());
    const COLUMNS: usize = 2;
    let columns: [(&str, &[u8]); COLUMNS] = [("actual", actual), ("reference", reference)];
    const LINE_BUFFER_SIZE: usize = 16 + COLUMNS * (4 * WIDTH + 1) + (COLUMNS - 1) * 2 + 1;

    assert_check!(
        actual.len() == reference.len(),
        "Sizes of memory blocks must be equal (actual={} expected={})",
        actual.len() as u64,
        reference.len() as u64
    );
    if actual.len() == reference.len() {
        let equals = actual == reference;
        assert_check!(equals, "Memory blocks contain the same data");
        if equals {
            return true;
        }
    }

    let mut line_buffer = [b' '; LINE_BUFFER_SIZE - 1];
    for (i, (header, _)) in columns.iter().enumerate() {
        let start = 16 + 1 + i * (4 * WIDTH + 3);
        line_buffer[start..start + header.len()].copy_from_slice(header.as_bytes());
    }
    log_error!("{}", String::from_utf8_lossy(&line_buffer));

    for i in (0..size_max).step_by(WIDTH) {
        let mut line = format!("{:016x}", i as u64);

        for (col, (_, data)) in columns.iter().enumerate() {
            for j in 0..WIDTH {
                if i + j < data.len() {
                    line += &format!(" {:02x}", data[i + j]);
                } else {
                    line += "   ";
                }
            }
            line += " ";
            for j in 0..WIDTH {
                let mut c = ' ';
                if i + j < data.len() {
                    c = data[i + j] as char;
                    if !isprint(data[i + j]) {
                        c = '.';
                    }
                }
                line.push(c);
            }
            if col < columns.len() - 1 {
                line += " |";
            }
        }
        log_error!("{}", line);
        sdl3::sdl_assert!(line.len() == LINE_BUFFER_SIZE - 1);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{test_lock, Capture};
    use sdl3::video::pixels::PixelFormat;

    fn surface(w: i32, h: i32, format: PixelFormat, color: Color) -> Surface<'static> {
        let mut surface = Surface::new(w, h, format).unwrap();
        for y in 0..h {
            for x in 0..w {
                surface.write_pixel(x, y, color).unwrap();
            }
        }
        surface
    }

    /// Run in a directory of its own, as failures save BMP files.
    struct InTempDir(std::path::PathBuf, std::path::PathBuf);

    impl InTempDir {
        fn new(name: &str) -> InTempDir {
            let dir = std::env::temp_dir().join(format!("sdl3-test-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let old = std::env::current_dir().unwrap();
            std::env::set_current_dir(&dir).unwrap();
            InTempDir(old, dir)
        }
    }

    impl Drop for InTempDir {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    #[test]
    fn surfaces() {
        let _l = test_lock();
        let capture = Capture::new();
        let dir = InTempDir::new("compare");
        let red = Color::new(255, 0, 0, 255);
        let mut a = surface(4, 3, PixelFormat::RGBA8888, red);
        let mut b = surface(4, 3, PixelFormat::ARGB8888, red);
        assert_eq!(compare_surfaces(&mut a, &mut b, 0).unwrap(), 0);
        assert!(capture.lines().is_empty());

        // Two pixels off: one by a squared distance of 2, one by 400.
        b.write_pixel(1, 0, Color::new(254, 1, 0, 255)).unwrap();
        b.write_pixel(2, 2, Color::new(235, 0, 0, 255)).unwrap();
        let before = COMPARE_SURFACE_COUNT.load(Ordering::Relaxed);
        assert_eq!(compare_surfaces(&mut a, &mut b, 0).unwrap(), 2);
        assert_eq!(compare_surfaces(&mut a, &mut b, 2).unwrap(), 1);
        assert_eq!(compare_surfaces(&mut a, &mut b, 400).unwrap(), 0);
        // A negative allowance is 0.
        capture.clear();
        assert_eq!(compare_surfaces(&mut a, &mut b, -5).unwrap(), 2);
        let count = before + 4;
        assert_eq!(
            capture.messages(),
            [
                ": Comparison of pixels with allowable error of 0 failed 2 times.".to_owned(),
                ": Reference surface format: SDL_PIXELFORMAT_ARGB8888".to_owned(),
                ": Actual surface format: SDL_PIXELFORMAT_RGBA8888".to_owned(),
                ": First detected occurrence at position 1,0 with a squared RGB-difference of 2."
                    .to_owned(),
                ": Reference pixel: R=254 G=1 B=0 A=255".to_owned(),
                ": Actual pixel   : R=255 G=0 B=0 A=255".to_owned(),
                format!(
                    ": Surfaces from failed comparison saved as \
                     'CompareSurfaces{count:04}_TestOutput.bmp' and \
                     'CompareSurfaces{count:04}_Reference.bmp'"
                ),
            ]
        );
        let saved = Surface::load_bmp(format!("CompareSurfaces{count:04}_Reference.bmp")).unwrap();
        assert_eq!(saved.read_pixel(1, 0).unwrap(), Color::new(254, 1, 0, 255));
        assert!(
            std::path::Path::new(&format!("CompareSurfaces{count:04}_TestOutput.bmp")).exists()
        );

        // Alpha doesn't count.
        let mut c = surface(4, 3, PixelFormat::RGBA8888, Color::new(255, 0, 0, 7));
        assert_eq!(compare_surfaces(&mut a, &mut c, 0).unwrap(), 0);

        // Sizes.
        let mut d = surface(3, 4, PixelFormat::RGBA8888, red);
        assert_eq!(
            compare_surfaces(&mut a, &mut d, 0).unwrap_err().message(),
            "Expected 3x4 surface, got 4x3"
        );
        drop(dir);
    }

    #[test]
    fn transparent_pixels() {
        let _l = test_lock();
        let _capture = Capture::new();
        let _dir = InTempDir::new("transparent");
        let mut a = surface(2, 2, PixelFormat::RGBA8888, Color::new(0, 0, 255, 255));
        let mut reference = surface(2, 2, PixelFormat::RGBA8888, Color::new(0, 0, 255, 255));
        reference.write_pixel(0, 0, Color::new(9, 9, 9, 0)).unwrap();
        reference.write_pixel(1, 1, Color::new(9, 9, 9, 1)).unwrap();
        assert_eq!(compare_surfaces(&mut a, &mut reference, 0).unwrap(), 2);
        assert_eq!(
            compare_surfaces_ignore_transparent_pixels(&mut a, &mut reference, 0).unwrap(),
            1
        );
    }

    #[test]
    fn memory() {
        let _l = test_lock();
        let capture = Capture::new();
        crate::assert::reset_summary();
        assert!(compare_memory(b"same", b"same"));
        assert_eq!(crate::assert::summary(), (2, 0));
        assert!(capture.messages().iter().all(|m| m.ends_with("Passed")));

        capture.clear();
        let actual: Vec<u8> = (0u8..20).map(|i| b'A' + i).collect();
        let mut reference = actual.clone();
        reference[3] = 0;
        reference.push(0xff);
        assert!(!compare_memory(&actual, &reference));
        assert_eq!(crate::assert::summary(), (2, 1));
        let actual: Vec<u8> = (0..40u32).map(|i| (i * 37 + 5) as u8).collect();
        let mut reference = actual.clone();
        reference[39] = b'~';
        assert!(!compare_memory(&actual, &reference));
        assert_eq!(crate::assert::summary(), (3, 2));

        // The log of upstream's C for the same blocks (lines ending in `$`).
        let expected: Vec<String> = include_str!("testdata/compare_memory.txt")
            .lines()
            .map(|l| l.strip_suffix('$').unwrap().to_owned())
            .collect();
        assert_eq!(capture.messages(), expected);
        crate::assert::reset_summary();
    }
}
