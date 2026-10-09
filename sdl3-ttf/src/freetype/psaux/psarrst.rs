// Rust translation of src/psaux/psarrst.c and src/psaux/psarrst.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright 2007-2013 Adobe Systems Incorporated.
//
// This software, and all works of authorship, whether in source or
// object code form as indicated by the copyright notice(s) included
// herein (collectively, the "Work") is made available, and may only be
// used, modified, and distributed under the FreeType Project License,
// LICENSE.TXT.  Additionally, subject to the terms and conditions of the
// FreeType Project License, each contributor to the Work hereby grants
// to any individual or legal entity exercising permissions granted by
// the FreeType Project License and this section (hereafter, "You" or
// "Your") a perpetual, worldwide, non-exclusive, no-charge,
// royalty-free, irrevocable (except as stated in this section) patent
// license to make, have made, use, offer to sell, sell, import, and
// otherwise transfer the Work, where such license applies only to those
// patent claims licensable by such contributor that are necessarily
// infringed by their contribution(s) alone or by combination of their
// contribution(s) with the Work to which such contribution(s) was
// submitted.  If You institute patent litigation against any entity
// (including a cross-claim or counterclaim in a lawsuit) alleging that
// the Work or a contribution incorporated within the Work constitutes
// direct or contributory patent infringement, then any patent licenses
// granted to You under this License for that Work shall terminate as of
// the date such litigation is filed.
//
// By using, modifying, or distributing the Work you indicate that you
// have read and understood the terms and conditions of the
// FreeType Project License as well as those provided in this section,
// and you accept them fully.
//
// This is an altered (translated) version of the original software; the
// FreeType Project License is in FTL.TXT (see also LICENSE.txt).

//! Adobe's code for Array Stacks (body).
//!
//! The stack's items are a `Vec` of `allocated` entries, of which `count`
//! are in use; its shared error is given to the functions that set it.
//! C's `getPointer` returns a pointer into the buffer; here it returns the
//! index, for the caller to use with [`Cf2ArrStackRec::items`].

use super::super::fttypes::*;
use super::pserror::cf2_set_error_code;

/// `CF2_ArrStackRec`: need to define the struct here (not opaque) so it
/// can be allocated by clients
#[derive(Debug, Clone, Default)]
pub struct Cf2ArrStackRec<T> {
    pub allocated: usize, /* items allocated               */
    pub count: usize,     /* number of elements allocated  */
    pub totalSize: usize, /* total bytes allocated         */

    /// `ptr` (ptr to data)
    pub items: Vec<T>,
}

/*
 * CF2_ArrStack uses an error pointer, to enable shared errors.
 * Shared errors are necessary when multiple objects allow the program
 * to continue after detecting errors.  Only the first error should be
 * recorded.
 */

/// `cf2_arrstack_init`
pub fn cf2_arrstack_init<T>(arrstack: &mut Cf2ArrStackRec<T>) {
    /* initialize the structure */
    arrstack.allocated = 0;
    arrstack.count = 0;
    arrstack.totalSize = 0;
    arrstack.items = Vec::new();
}

/// `cf2_arrstack_finalize`
pub fn cf2_arrstack_finalize<T>(arrstack: &mut Cf2ArrStackRec<T>) {
    arrstack.allocated = 0;
    arrstack.count = 0;
    arrstack.totalSize = 0;

    /* free the data buffer */
    arrstack.items = Vec::new();
}

/* allocate or reallocate the buffer size; */
/* return false on memory error */
fn cf2_arrstack_set_num_elements<T: Clone + Default>(
    arrstack: &mut Cf2ArrStackRec<T>,
    num_elements: usize,
    error: &mut FtError,
) -> bool {
    let size_item = std::mem::size_of::<T>().max(1);
    let new_size = num_elements.wrapping_mul(size_item);

    if num_elements <= (i64::MAX as usize) / size_item {
        let ok = if num_elements > arrstack.items.len() {
            arrstack
                .items
                .try_reserve_exact(num_elements - arrstack.items.len())
                .is_ok()
        } else {
            true
        };
        if ok {
            arrstack.items.resize(num_elements, T::default());

            arrstack.allocated = num_elements;
            arrstack.totalSize = new_size;

            if arrstack.count > num_elements {
                /* we truncated the list! */
                cf2_set_error_code(error, FT_ERR_STACK_OVERFLOW);
                arrstack.count = num_elements;
                return false;
            }

            return true; /* success */
        }
    }

    /* exit: */
    /* if there's not already an error, store this one */
    cf2_set_error_code(error, FT_ERR_OUT_OF_MEMORY);

    false
}

/// `cf2_arrstack_setCount`: set the count, ensuring allocation is
/// sufficient
pub fn cf2_arrstack_set_count<T: Clone + Default>(
    arrstack: &mut Cf2ArrStackRec<T>,
    num_elements: usize,
    error: &mut FtError,
) {
    if num_elements > arrstack.allocated {
        /* expand the allocation first */
        if !cf2_arrstack_set_num_elements(arrstack, num_elements, error) {
            return;
        }
    }

    arrstack.count = num_elements;
}

/// `cf2_arrstack_clear`: clear the count
pub fn cf2_arrstack_clear<T>(arrstack: &mut Cf2ArrStackRec<T>) {
    arrstack.count = 0;
}

/// `cf2_arrstack_size`: current number of items
pub fn cf2_arrstack_size<T>(arrstack: &Cf2ArrStackRec<T>) -> usize {
    arrstack.count
}

/// `cf2_arrstack_getBuffer`: the index of the first element
pub fn cf2_arrstack_get_buffer<T>(_arrstack: &Cf2ArrStackRec<T>) -> usize {
    0
}

/// `cf2_arrstack_getPointer`: return pointer to the given element (as its
/// index)
pub fn cf2_arrstack_get_pointer<T>(
    arrstack: &Cf2ArrStackRec<T>,
    mut idx: usize,
    error: &mut FtError,
) -> usize {
    if idx >= arrstack.count {
        /* overflow */
        cf2_set_error_code(error, FT_ERR_STACK_OVERFLOW);
        idx = 0; /* choose safe default */
    }

    idx
}

/// `cf2_arrstack_push`: push (append) an element at the end of the list;
/// on memory error, the push is ignored
/// TODO: should there be a length param for extra checking?
pub fn cf2_arrstack_push<T: Clone + Default>(
    arrstack: &mut Cf2ArrStackRec<T>,
    item: &T,
    error: &mut FtError,
) {
    if arrstack.count == arrstack.allocated {
        /* increase the buffer size */
        if !cf2_arrstack_set_num_elements(arrstack, arrstack.allocated * 2 + 16, error) {
            /* on error, ignore the push */
            return;
        }
    }

    arrstack.items[arrstack.count] = item.clone();
    arrstack.count += 1;
}
