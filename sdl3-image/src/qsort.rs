// Rust translation of SDL_qsort() from src/stdlib/SDL_qsort.c from Simple
// DirectMedia Layer (its generic, element-size path).
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.
//
// The sort itself came from Gareth McCaughan, under the zlib license:
//
// Copyright (c) 1998-2021 Gareth McCaughan
//
// This software is provided 'as-is', without any express or implied
// warranty. In no event will the authors be held liable for any
// damages arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented;
//    you must not claim that you wrote the original software.
//    If you use this software in a product, an acknowledgment
//    in the product documentation would be appreciated but
//    is not required.
//
// 2. Altered source versions must be plainly marked as such,
//    and must not be misrepresented as being the original software.
//
// 3. This notice may not be removed or altered from any source
//    distribution.

//! SDL's own quicksort, which nanosvg's rasterizer sorts its edges with.
//! It isn't stable: the order it leaves equal elements in decides how the
//! rasterizer meets edges that start on the same scanline, so it is
//! translated rather than replaced with the standard library's sort.
//!
//! (The `sdl3` crate sorts with `sort_unstable_by` where upstream calls
//! `SDL_qsort()`; this is the exact algorithm, for the one place here
//! where the order of equal elements shows.)
//!
//! Upstream has separate copies of the algorithm for unaligned, aligned
//! and `int`-sized elements, differing only in how they swap; this is the
//! one for any element type, on indices instead of byte pointers.

use std::cmp::Ordering;

/* Different situations have slightly different requirements,
 * and we make life epsilon easier by using different truncation
 * points for the three different cases.
 * So far, I have tuned TRUNC_words and guessed that the same
 * value might work well for the other two cases. Of course
 * what works well on my machine might work badly on yours.
 */
const TRUNC_ALIGNED: isize = 12;

/* We use a simple pivoting algorithm for shortish sub-arrays
 * and a more complicated one for larger ones. The threshold
 * is PIVOT_THRESHOLD.
 */
const PIVOT_THRESHOLD: isize = 40;

/// Translation of `pivot_big()`: the median of three medians of three.
fn pivot_big<T>(
    v: &[T],
    first: isize,
    mid: isize,
    last: isize,
    compare: &impl Fn(&T, &T) -> Ordering,
) -> isize {
    let d = (last - first) >> 3;
    let lt = |a: isize, b: isize| compare(&v[a as usize], &v[b as usize]) == Ordering::Less;
    let median = |a: isize, b: isize, c: isize| {
        if lt(a, b) {
            if lt(b, c) {
                b
            } else if lt(a, c) {
                c
            } else {
                a
            }
        } else if lt(a, c) {
            a
        } else if lt(b, c) {
            c
        } else {
            b
        }
    };
    let m1 = median(first, first + d, first + 2 * d);
    let m2 = median(mid - d, mid, mid + d);
    let m3 = median(last - 2 * d, last - d, last);
    median(m1, m2, m3)
}

/// Sort `v` with `compare`, exactly as `SDL_qsort()` does (in particular,
/// leaving equal elements in the same order). Translation of
/// `SDL_qsort()` (by way of `SDL_qsort_r()` and `qsort_r_aligned()`).
pub(crate) fn sdl_qsort<T: Copy>(v: &mut [T], compare: impl Fn(&T, &T) -> Ordering) {
    let nmemb = v.len() as isize;
    if nmemb <= 1 {
        return;
    }
    let cmp = |v: &[T], a: isize, b: &T| compare(&v[a as usize], b);
    let gt =
        |v: &[T], a: isize, b: isize| compare(&v[a as usize], &v[b as usize]) == Ordering::Greater;
    let lt =
        |v: &[T], a: isize, b: isize| compare(&v[a as usize], &v[b as usize]) == Ordering::Less;
    let swap = |v: &mut [T], a: isize, b: isize| v.swap(a as usize, b as usize);

    // (STACK_SIZE entries: one per bit of a size_t)
    let mut stack: Vec<(isize, isize)> = Vec::with_capacity(64);
    let trunc = TRUNC_ALIGNED;

    let mut first: isize = 0;
    let mut last: isize = nmemb - 1;

    if last - first >= trunc {
        let mut ffirst = first;
        let mut llast = last;
        loop {
            /* Select pivot */
            let pivot;
            {
                let mut mid = first + ((last - first) >> 1);
                if mid >= last {
                    break;
                }
                // Pivot()
                if last - first > PIVOT_THRESHOLD {
                    mid = pivot_big(v, first, mid, last, &compare);
                } else {
                    if lt(v, first, mid) {
                        if gt(v, mid, last) {
                            swap(v, mid, last);
                            if gt(v, first, mid) {
                                swap(v, first, mid);
                            }
                        }
                    } else if gt(v, mid, last) {
                        swap(v, first, last);
                    } else {
                        swap(v, first, mid);
                        if gt(v, mid, last) {
                            swap(v, mid, last);
                        }
                    }
                    first += 1;
                    last -= 1;
                }
                pivot = v[mid as usize];
            }
            /* Partition. */
            loop {
                while cmp(v, first, &pivot) == Ordering::Less {
                    first += 1;
                }
                while compare(&pivot, &v[last as usize]) == Ordering::Less {
                    last -= 1;
                }
                if first < last {
                    swap(v, first, last);
                    first += 1;
                    last -= 1;
                } else if first == last {
                    first += 1;
                    last -= 1;
                    break;
                }
                if first > last {
                    break;
                }
            }
            /* Prepare to recurse/iterate. */
            // Recurse(trunc): (size_t) differences, as upstream computes them
            let l = (last - ffirst) as usize;
            let r = (llast - first) as usize;
            let trunc = trunc as usize;
            if l < trunc {
                if r >= trunc {
                    // doRight
                    ffirst = first;
                    last = llast;
                    continue;
                }
                // pop
                let Some((f, l)) = stack.pop() else {
                    break;
                };
                first = f;
                ffirst = f;
                last = l;
                llast = l;
                continue;
            } else if l <= r {
                // pushRight; doLeft
                stack.push((first, llast));
                first = ffirst;
                llast = last;
                continue;
            } else if r >= trunc {
                // pushLeft; doRight
                stack.push((ffirst, last));
                ffirst = first;
                last = llast;
                continue;
            } else {
                // doLeft
                first = ffirst;
                llast = last;
                continue;
            }
        }
    }

    // PreInsertion(): the smallest of the first TRUNC elements to the front
    first = 0;
    last = nmemb.min(TRUNC_ALIGNED) - 1;
    while last != 0 {
        if gt(v, first, last) {
            first = last;
        }
        last -= 1;
    }
    if first != 0 {
        swap(v, first, 0);
    }

    // Insertion()
    for first in 1..nmemb {
        /* Find the right place for |first|.
         * My apologies for var reuse. */
        let mut test = first - 1;
        while test >= 0 && gt(v, test, first) {
            test -= 1;
        }
        test += 1;
        if test != first {
            /* Shift everything in [test,first)
             * up by one, and place |first|
             * where |test| is. */
            v[test as usize..=first as usize].rotate_right(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_like_a_sort() {
        let mut seed = 12345u32;
        for n in [0usize, 1, 2, 5, 11, 12, 13, 40, 41, 42, 100, 1000] {
            let mut v: Vec<(u32, usize)> = (0..n)
                .map(|i| {
                    seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                    ((seed >> 16) % 17, i)
                })
                .collect();
            let mut expected = v.clone();
            expected.sort_by_key(|e| e.0);
            sdl_qsort(&mut v, |a, b| a.0.cmp(&b.0));
            let keys: Vec<u32> = v.iter().map(|e| e.0).collect();
            let want: Vec<u32> = expected.iter().map(|e| e.0).collect();
            assert_eq!(keys, want, "{n}");
        }
    }
}
