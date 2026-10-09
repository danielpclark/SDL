// Rust translation of the sort of src/hb-algs.hh (hb_qsort, from
// https://github.com/noporpoise/sort_r) from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it).
// Copyright © 2017  Google, Inc.
// Copyright © 2019  Facebook, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Google Author(s): Behdad Esfahbod
// Facebook Author(s): Behdad Esfahbod

//! `hb_qsort`: HarfBuzz's own (unstable) quicksort, translated exactly, as
//! the order it leaves equal items in matters where they are merged.

/* From https://github.com/noporpoise/sort_r
Feb 5, 2019 (c8c65c1e)
Modified to support optional argument using templates */

/* Isaac Turner 29 April 2014 Public Domain */

/// `sort_r_cmpswap`: swap a and b iff a>b
#[inline]
fn sort_r_cmpswap<T>(base: &mut [T], a: usize, b: usize, compar: &impl Fn(&T, &T) -> i32) -> bool {
    if compar(&base[a], &base[b]) > 0 {
        base.swap(a, b);
        return true;
    }
    false
}

/// `sort_r_swap_blocks`: Swap consecutive blocks of na and nb items
/// starting at ptr, with the smallest swap so that the blocks are in the
/// opposite order. Blocks may be internally re-ordered e.g.
///
/// ```text
///   12345ab  ->   ab34512
///   123abc   ->   abc123
///   12abcde  ->   deabc12
/// ```
#[inline]
fn sort_r_swap_blocks<T>(base: &mut [T], ptr: usize, na: usize, nb: usize) {
    if na > 0 && nb > 0 {
        if na > nb {
            /* sort_r_swap (ptr, ptr+na, nb) */
            for k in 0..nb {
                base.swap(ptr + k, ptr + na + k);
            }
        } else {
            /* sort_r_swap (ptr, ptr+nb, na) */
            for k in 0..na {
                base.swap(ptr + k, ptr + nb + k);
            }
        }
    }
}

/// `sort_r_simple`: recursive quicksort (not stable, equivalent values may
/// be swapped), on the `nel` items of `base` from `b`.
fn sort_r_simple<T>(base: &mut [T], b: usize, nel: usize, compar: &impl Fn(&T, &T) -> i32) {
    let end = b + nel;

    if nel < 10 {
        /* Insertion sort for arbitrarily small inputs */
        let mut pi = b + 1;
        while pi < end {
            let mut pj = pi;
            while pj > b && sort_r_cmpswap(base, pj - 1, pj, compar) {
                pj -= 1;
            }
            pi += 1;
        }
    } else {
        /* nel > 9; Quicksort */

        let last = b + (nel - 1);

        /*
        Use median of second, middle and second-last items as pivot.
        First and last may have been swapped with pivot and therefore be extreme
        */
        let mut l = [b + 1, b + nel / 2, last - 1];

        if compar(&base[l[0]], &base[l[1]]) > 0 {
            l.swap(0, 1);
        }
        if compar(&base[l[1]], &base[l[2]]) > 0 {
            l.swap(1, 2);
            if compar(&base[l[0]], &base[l[1]]) > 0 {
                l.swap(0, 1);
            }
        }

        /* swap mid value (l[1]), and last element to put pivot as last element */
        if l[1] != last {
            base.swap(l[1], last);
        }

        /*
        pl is the next item on the left to be compared to the pivot
        pr is the last item on the right that was compared to the pivot
        ple is the left position to put the next item that equals the pivot
        ple is the last right position where we put an item that equals the pivot
                                               v- end (beyond the array)
          EEEEEELLLLLLLLuuuuuuuuGGGGGGGEEEEEEEE.
          ^- b  ^- ple  ^- pl   ^- pr  ^- pre ^- last (where the pivot is)
        Pivot comparison key:
          E = equal, L = less than, u = unknown, G = greater than, E = equal
        */
        let pivot = last;
        let mut ple = b;
        let mut pl = b;
        let mut pre = last;
        let mut pr = last;

        /*
        Strategy:
        Loop into the list from the left and right at the same time to find:
        - an item on the left that is greater than the pivot
        - an item on the right that is less than the pivot
        Once found, they are swapped and the loop continues.
        Meanwhile items that are equal to the pivot are moved to the edges of the
        array.
        */
        while pl < pr {
            /* Move left hand items which are equal to the pivot to the far left.
            break when we find an item that is greater than the pivot */
            while pl < pr {
                let cmp = compar(&base[pl], &base[pivot]);
                if cmp > 0 {
                    break;
                } else if cmp == 0 {
                    if ple < pl {
                        base.swap(ple, pl);
                    }
                    ple += 1;
                }
                pl += 1;
            }
            /* break if last batch of left hand items were equal to pivot */
            if pl >= pr {
                break;
            }
            /* Move right hand items which are equal to the pivot to the far right.
            break when we find an item that is less than the pivot */
            while pl < pr {
                pr -= 1; /* Move right pointer onto an unprocessed item */
                let cmp = compar(&base[pr], &base[pivot]);
                if cmp == 0 {
                    pre -= 1;
                    if pr < pre {
                        base.swap(pr, pre);
                    }
                } else if cmp < 0 {
                    if pl < pr {
                        base.swap(pl, pr);
                    }
                    pl += 1;
                    break;
                }
            }
        }

        pl = pr; /* pr may have gone below pl */

        /*
        Now we need to go from: EEELLLGGGGEEEE
                            to: LLLEEEEEEEGGGG
        Pivot comparison key:
          E = equal, L = less than, u = unknown, G = greater than, E = equal
        */
        sort_r_swap_blocks(base, b, ple - b, pl - ple);
        sort_r_swap_blocks(base, pr, pre - pr, end - pre);

        sort_r_simple(base, b, pl - ple, compar);
        sort_r_simple(base, end - (pre - pr), pre - pr, compar);
    }
}

/// `hb_qsort`
pub(crate) fn hb_qsort<T>(base: &mut [T], compar: impl Fn(&T, &T) -> i32) {
    let nel = base.len();
    sort_r_simple(base, 0, nel, &compar);
}

#[cfg(test)]
mod tests {
    use super::hb_qsort;

    #[test]
    fn sorts() {
        let mut v: Vec<u32> = (0..200).map(|i| (i * 7919 + 13) % 101).collect();
        hb_qsort(&mut v, |a, b| (*a as i32) - (*b as i32));
        assert!(v.windows(2).all(|w| w[0] <= w[1]));
    }
}
