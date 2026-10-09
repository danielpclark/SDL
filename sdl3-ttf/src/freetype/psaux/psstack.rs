// Rust translation of src/psaux/psstack.c and src/psaux/psstack.h from
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

//! Adobe's code for emulating a CFF stack (body).
//!
//! The stack's shared error is given to the functions that set it.

use super::super::fttypes::*;
use super::pserror::cf2_set_error_code;
use super::psfixed::*;

/// `CF2_StackNumber`: CFF operand stack; specified maximum of 48 or 192
/// values (the union of `r`, `f` and `i` is its 32 bits)
#[derive(Debug, Clone, Copy, Default)]
pub struct Cf2StackNumber {
    /// `u.r` (16.16 fixed-point), `u.f` (2.30 fixed-point, for font
    /// matrix) or `u.i`
    pub u: i32,
    pub type_: Cf2NumberType,
}

/// `CF2_StackRec`
#[derive(Debug, Clone, Default)]
pub struct Cf2StackRec {
    pub buffer: Vec<Cf2StackNumber>,
    /// `top`, as an index into `buffer`
    pub top: usize,
    pub stackSize: FtUInt,
}

/// `cf2_stack_init`: allocate and initialize an instance of CF2_Stack.
/// Note: This function returns NULL on error (does not set `error').
///
/// FIXME (upstream): `cf2_stack_setReal` accepts an index equal to the
/// count, which writes one element past a full stack; the buffer has that
/// extra element here.
pub fn cf2_stack_init(stack_size: FtUInt) -> Option<Cf2StackRec> {
    /* allocate the stack buffer */
    let mut buffer = Vec::new();
    if buffer.try_reserve_exact(stack_size as usize + 1).is_err() {
        return None;
    }
    buffer.resize(stack_size as usize + 1, Cf2StackNumber::default());

    Some(Cf2StackRec {
        buffer,
        top: 0, /* empty stack */
        stackSize: stack_size,
    })
}

/// `cf2_stack_free`
pub fn cf2_stack_free(_stack: Option<Cf2StackRec>) {}

/// `cf2_stack_count`
pub fn cf2_stack_count(stack: &Cf2StackRec) -> Cf2UInt {
    stack.top as Cf2UInt
}

/// `cf2_stack_pushInt`
pub fn cf2_stack_push_int(stack: &mut Cf2StackRec, val: Cf2Int, error: &mut FtError) {
    if stack.top == stack.stackSize as usize {
        cf2_set_error_code(error, FT_ERR_STACK_OVERFLOW);
        return; /* stack overflow */
    }

    stack.buffer[stack.top].u = val;
    stack.buffer[stack.top].type_ = Cf2NumberType::Int;
    stack.top += 1;
}

/// `cf2_stack_pushFixed`
pub fn cf2_stack_push_fixed(stack: &mut Cf2StackRec, val: Cf2Fixed, error: &mut FtError) {
    if stack.top == stack.stackSize as usize {
        cf2_set_error_code(error, FT_ERR_STACK_OVERFLOW);
        return; /* stack overflow */
    }

    stack.buffer[stack.top].u = val;
    stack.buffer[stack.top].type_ = Cf2NumberType::Fixed;
    stack.top += 1;
}

/// `cf2_stack_popInt`: this function is only allowed to pop an integer
/// type
pub fn cf2_stack_pop_int(stack: &mut Cf2StackRec, error: &mut FtError) -> Cf2Int {
    if stack.top == 0 {
        cf2_set_error_code(error, FT_ERR_STACK_UNDERFLOW);
        return 0; /* underflow */
    }
    if stack.buffer[stack.top - 1].type_ != Cf2NumberType::Int {
        cf2_set_error_code(error, FT_ERR_SYNTAX_ERROR);
        return 0; /* type mismatch */
    }

    stack.top -= 1;

    stack.buffer[stack.top].u
}

/// The value of a stack number as 16.16 fixed-point.
fn cf2_stack_number_to_fixed(n: &Cf2StackNumber) -> Cf2Fixed {
    match n.type_ {
        Cf2NumberType::Int => cf2_int_to_fixed(n.u),
        Cf2NumberType::Frac => cf2_frac_to_fixed(n.u),
        Cf2NumberType::Fixed => n.u,
    }
}

/// `cf2_stack_popFixed`: Note: type mismatch is silently cast
/// TODO: check this
pub fn cf2_stack_pop_fixed(stack: &mut Cf2StackRec, error: &mut FtError) -> Cf2Fixed {
    if stack.top == 0 {
        cf2_set_error_code(error, FT_ERR_STACK_UNDERFLOW);
        return cf2_int_to_fixed(0); /* underflow */
    }

    stack.top -= 1;

    cf2_stack_number_to_fixed(&stack.buffer[stack.top])
}

/// `cf2_stack_getReal`: Note: type mismatch is silently cast
/// TODO: check this
pub fn cf2_stack_get_real(stack: &Cf2StackRec, idx: Cf2UInt, error: &mut FtError) -> Cf2Fixed {
    if idx >= cf2_stack_count(stack) {
        cf2_set_error_code(error, FT_ERR_STACK_OVERFLOW);
        return cf2_int_to_fixed(0); /* bounds error */
    }

    cf2_stack_number_to_fixed(&stack.buffer[idx as usize])
}

/// `cf2_stack_setReal`: provide random access to stack
pub fn cf2_stack_set_real(
    stack: &mut Cf2StackRec,
    idx: Cf2UInt,
    val: Cf2Fixed,
    error: &mut FtError,
) {
    if idx > cf2_stack_count(stack) {
        cf2_set_error_code(error, FT_ERR_STACK_OVERFLOW);
        return;
    }

    stack.buffer[idx as usize].u = val;
    stack.buffer[idx as usize].type_ = Cf2NumberType::Fixed;
}

/// `cf2_stack_pop`: discard (pop) num values from stack
pub fn cf2_stack_pop(stack: &mut Cf2StackRec, num: Cf2UInt, error: &mut FtError) {
    if num > cf2_stack_count(stack) {
        cf2_set_error_code(error, FT_ERR_STACK_UNDERFLOW);
        return;
    }
    stack.top -= num as usize;
}

/// `cf2_stack_roll`
pub fn cf2_stack_roll(
    stack: &mut Cf2StackRec,
    count: Cf2Int,
    mut shift: Cf2Int,
    error: &mut FtError,
) {
    /* we initialize this variable to avoid compiler warnings */
    let mut last = Cf2StackNumber {
        u: 0,
        type_: Cf2NumberType::Int,
    };

    if count < 2 {
        return; /* nothing to do (values 0 and 1), or undefined value */
    }

    if count as Cf2UInt > cf2_stack_count(stack) {
        cf2_set_error_code(error, FT_ERR_STACK_OVERFLOW);
        return;
    }

    /* before C99 it is implementation-defined whether    */
    /* the result of `%' is negative if the first operand */
    /* is negative                                        */
    if shift < 0 {
        shift = -(shift.wrapping_neg() % count);
    } else {
        shift %= count;
    }

    if shift == 0 {
        return; /* nothing to do */
    }

    /* We use the following algorithm to do the rolling, */
    /* which needs two temporary variables only.         */
    /*                                                   */
    /* Example:                                          */
    /*                                                   */
    /*   count = 8                                       */
    /*   shift = 2                                       */
    /*                                                   */
    /*   stack indices before roll:  7 6 5 4 3 2 1 0     */
    /*   stack indices after roll:   1 0 7 6 5 4 3 2     */
    /*                                                   */
    /* The value of index 0 gets moved to index 2, while */
    /* the old value of index 2 gets moved to index 4,   */
    /* and so on.  We thus have the following copying    */
    /* chains for shift value 2.                         */
    /*                                                   */
    /*   0 -> 2 -> 4 -> 6 -> 0                           */
    /*   1 -> 3 -> 5 -> 7 -> 1                           */
    /*                                                   */
    /* If `count' and `shift' are incommensurable, we    */
    /* have a single chain only.  Otherwise, increase    */
    /* the start index by 1 after the first chain, then  */
    /* do the next chain until all elements in all       */
    /* chains are handled.                               */

    let mut start_idx: Cf2Int = -1;
    let mut idx: Cf2Int = -1;
    for _ in 0..count {
        if start_idx == idx {
            start_idx += 1;
            idx = start_idx;
            last = stack.buffer[idx as usize];
        }

        idx += shift;
        if idx >= count {
            idx -= count;
        } else if idx < 0 {
            idx += count;
        }

        core::mem::swap(&mut stack.buffer[idx as usize], &mut last);
    }
}

/// `cf2_stack_clear`
pub fn cf2_stack_clear(stack: &mut Cf2StackRec) {
    stack.top = 0;
}
