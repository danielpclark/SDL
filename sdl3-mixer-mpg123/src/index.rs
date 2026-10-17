/*
    index: frame index data structure and functions

    This is for keeping track of frame positions for accurate seeking.
    Now in it's own file, with initial code from frame.c and parse.c .

    The idea of the index with a certain amount of entries is to cover
    all yet-encountered frame positions with minimal coarseness.
    Meaning: At first every frame position is recorded, then, when
    the index is full, every second position is trown away to make
    space. Next time it is full, the same happens. And so on.
    In this manner we maintain a good resolution with the given
    maximum index size while covering the whole stream.

    copyright 2007-2023 by the mpg123 project
    -= free software under the terms of the LGPL 2.1 =-
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Thomas Orgis
*/
// Modified 2026-10-17: translated into Rust from index.h and index.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

/// Translation of `struct frame_index`. `data.len()` is `size`.
#[derive(Debug, Default)]
pub(crate) struct FrameIndex {
    /// actual data, the frame positions
    pub data: Vec<i64>,
    /// advancement in frame number per index point
    pub step: i64,
    /// frame offset supposed to come next into the index
    pub next: i64,
    /// number of used entries
    pub fill: usize,
    /// if > 0: index allowed to grow on need with these steps, instead of lowering resolution
    pub grow_size: usize,
}

impl FrameIndex {
    /// total number of possible entries
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

/// The condition for a framenum to be appended to the index (`FI_NEXT()`).
/// `if(FI_NEXT(fr->index, fr->num)) INT123_fi_add(offset);`
pub(crate) fn fi_next_cond(fi: &FrameIndex, framenum: i64) -> bool {
    fi.size() != 0 && framenum == fi.next
}

/* The next expected frame offset, one step ahead. */
fn fi_next(fi: &FrameIndex) -> i64 {
    fi.fill as i64 * fi.step
}

/* Shrink down the used index to the half.
Be careful with size = 1 ... there's no shrinking possible there. */
fn fi_shrink(fi: &mut FrameIndex) {
    if fi.fill < 2 {
        return; /* Won't shrink below 1. */
    } else {
        /* Double the step, half the fill. Should work as well for fill%2 = 1 */
        fi.step *= 2;
        fi.fill /= 2;
        /* Move the data down. */
        for c in 0..fi.fill {
            fi.data[c] = fi.data[2 * c];
        }
    }
    fi.next = fi_next(fi);
}

/// Translation of `INT123_fi_init()`.
pub(crate) fn fi_init(fi: &mut FrameIndex) {
    fi.data = Vec::new();
    fi.step = 1;
    fi.fill = 0;
    fi.grow_size = 0;
    fi.next = fi_next(fi);
}

/// Translation of `INT123_fi_exit()`.
pub(crate) fn fi_exit(fi: &mut FrameIndex) {
    fi_init(fi); /* Be prepared for further fun, still. */
}

/// Translation of `INT123_fi_resize()`.
pub(crate) fn fi_resize(fi: &mut FrameIndex, newsize: usize) -> i32 {
    if newsize == fi.size() {
        return 0;
    }

    if newsize > 0 && newsize < fi.size() {
        /* When we reduce buffer size a bit, shrink stuff. */
        while fi.fill > newsize {
            fi_shrink(fi);
        }
    }

    // (INT123_safe_reallocn())
    let ok = newsize <= isize::MAX as usize / 8
        && (newsize <= fi.data.len() || fi.data.try_reserve_exact(newsize - fi.data.len()).is_ok());
    if newsize == 0 || ok {
        fi.data.resize(newsize, 0);
        if fi.fill > fi.size() {
            fi.fill = fi.size();
        }
        fi.next = fi_next(fi);
        0
    } else {
        -1
    }
}

/// Translation of `INT123_fi_add()`.
pub(crate) fn fi_add(fi: &mut FrameIndex, pos: i64) {
    if fi.fill == fi.size() {
        /* Index is full, we need to shrink... or grow. */
        /* Store the current frame number to check later if we still want it. */
        let framenum = fi.fill as i64 * fi.step;
        /* If we want not / cannot grow, we shrink. */
        if !(fi.grow_size != 0 && fi_resize(fi, fi.size() + fi.grow_size) == 0) {
            fi_shrink(fi);
        }
        /* Now check if we still want to add this frame (could be that not, because of changed step). */
        if fi.next != framenum {
            return;
        }
    }
    /* When we are here, we want that frame. */
    if fi.fill < fi.size() {
        /* safeguard for size=1, or just generally */
        fi.data[fi.fill] = pos;
        fi.fill += 1;
        fi.next = fi_next(fi);
    }
}

/// Translation of `INT123_fi_reset()`.
pub(crate) fn fi_reset(fi: &mut FrameIndex) {
    fi.fill = 0;
    fi.step = 1;
    fi.next = fi_next(fi);
}
