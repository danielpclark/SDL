// Rust translation of src/lzw/ftzopen.c and src/lzw/ftzopen.h from
// FreeType (2.13.2, as SDL_ttf's external/freetype pins it).
// Copyright (C) 2005-2023 by David Turner.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! FreeType support for .Z compressed files.
//!
//! This optional component relies on NetBSD's zopen().  It should mainly
//! be used to parse compressed PCF fonts, as found with many X11 server
//! distributions.
//!
//! This is a complete re-implementation of the LZW file reader, since the
//! old one was incredibly badly written, using 400 KByte of heap memory
//! before decompressing anything.
//!
//! The `prefix` and `suffix` arrays, one block in C, are two vectors; the
//! character stack is a vector whose size grows as C's (from the 64 bytes
//! of `stack_0`). The source stream is given to the functions reading it.

use super::super::base::ftmemory::ft_renew_array;
use super::super::base::ftstream::FtStreamRec;
use super::super::fttypes::*;

pub const FT_LZW_IN_BUFF_SIZE: usize = 64;
pub const FT_LZW_DEFAULT_STACK_SIZE: usize = 64;

pub const LZW_INIT_BITS: FtUInt = 9;
pub const LZW_MAX_BITS: FtUInt = 16;

pub const LZW_CLEAR: FtUInt = 256;
pub const LZW_FIRST: FtUInt = 257;

pub const LZW_BIT_MASK: u8 = 0x1F;
pub const LZW_BLOCK_MASK: u8 = 0x80;

/// `LZW_MASK`
const fn lzw_mask(n: FtUInt) -> FtUInt {
    (1u32 << n).wrapping_sub(1)
}

/// `FT_LzwPhase`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FtLzwPhase {
    #[default]
    Start = 0,
    Code,
    Stack,
    Eof,
}

/*
 * state of LZW decompressor
 *
 *
 * small technical note
 * --------------------
 *
 * We use a few tricks in this implementation that are explained here to
 * ease debugging and maintenance.
 *
 * - First of all, the `prefix' and `suffix' arrays contain the suffix
 *   and prefix for codes over 256; this means that
 *
 *     prefix_of(code) == state->prefix[code-256]
 *     suffix_of(code) == state->suffix[code-256]
 *
 *   Each prefix is a 16-bit code, and each suffix an 8-bit byte.
 *
 *   Both arrays are stored in a single memory block, pointed to by
 *   `state->prefix'.  This means that the following equality is always
 *   true:
 *
 *     state->suffix == (FT_Byte*)(state->prefix + state->prefix_size)
 *
 *   Of course, state->prefix_size is the number of prefix/suffix slots
 *   in the arrays, corresponding to codes 256..255+prefix_size.
 *
 * - `free_ent' is the index of the next free entry in the `prefix'
 *   and `suffix' arrays.  This means that the corresponding `next free
 *   code' is really `256+free_ent'.
 *
 *   Moreover, `max_free' is the maximum value that `free_ent' can reach.
 *
 *   `max_free' corresponds to `(1 << max_bits) - 256'.  Note that this
 *   value is always <= 0xFF00, which means that both `free_ent' and
 *   `max_free' can be stored in an FT_UInt variable, even on 16-bit
 *   machines.
 *
 *   If `free_ent == max_free', you cannot add new codes to the
 *   prefix/suffix table.
 *
 * - `num_bits' is the current number of code bits, starting at 9 and
 *   growing each time `free_ent' reaches the value of `free_bits'.  The
 *   latter is computed as follows
 *
 *     if num_bits < max_bits:
 *        free_bits = (1 << num_bits)-256
 *     else:
 *        free_bits = max_free + 1
 *
 *   Since the value of `max_free + 1' can never be reached by
 *   `free_ent', `num_bits' cannot grow larger than `max_bits'.
 */

/// `FT_LzwStateRec`: state of LZW decompressor (see the technical note
/// above)
#[derive(Debug, Clone, Default)]
pub struct FtLzwStateRec {
    pub phase: FtLzwPhase,
    pub in_eof: FtInt,

    pub buf_tab: [FtByte; 16],
    pub buf_offset: FtUInt,
    pub buf_size: FtUInt,
    pub buf_clear: bool,
    pub buf_total: usize,

    pub max_bits: FtUInt,  /* max code bits, from file header   */
    pub block_mode: FtInt, /* block mode flag, from file header */
    pub max_free: FtUInt,  /* (1 << max_bits) - 256             */

    pub num_bits: FtUInt,  /* current code bit number */
    pub free_ent: FtUInt,  /* index of next free entry */
    pub free_bits: FtUInt, /* if reached by free_ent, increment num_bits */
    pub old_code: FtUInt,
    pub old_char: FtUInt,
    pub in_code: FtUInt,

    pub prefix: Vec<FtUShort>, /* always dynamically allocated / reallocated */
    pub suffix: Vec<FtByte>,   /* suffix = (FT_Byte*)(prefix + prefix_size)  */
    pub prefix_size: FtUInt,   /* number of slots in `prefix' or `suffix'    */

    /// `stack` (with `stack_0`, its first `FT_LZW_DEFAULT_STACK_SIZE`
    /// bytes)
    pub stack: Vec<FtByte>, /* character stack */
    pub stack_top: FtUInt,
    pub stack_size: usize,
}

/// `ft_lzwstate_refill`
fn ft_lzwstate_refill(state: &mut FtLzwStateRec, source: &mut FtStreamRec) -> i32 {
    if state.in_eof != 0 {
        return -1;
    }

    let n = (state.num_bits as usize).min(state.buf_tab.len());
    let count: FtULong = source.try_read(&mut state.buf_tab[..n]); /* WHY? */

    state.buf_size = count as FtUInt;
    state.buf_total = state.buf_total.wrapping_add(count as usize);
    state.in_eof = (count < state.num_bits as FtULong) as FtInt;
    state.buf_offset = 0;

    state.buf_size <<= 3;
    if state.buf_size > state.num_bits {
        state.buf_size -= state.num_bits - 1;
    } else {
        return -1; /* not enough data */
    }

    if count == 0 {
        /* end of file */
        return -1;
    }

    0
}

/// `ft_lzwstate_get_code`
fn ft_lzwstate_get_code(state: &mut FtLzwStateRec, source: &mut FtStreamRec) -> FtInt32 {
    let mut num_bits: FtUInt = state.num_bits;
    let mut offset: FtUInt = state.buf_offset;

    if state.buf_clear || offset >= state.buf_size || state.free_ent >= state.free_bits {
        if state.free_ent >= state.free_bits {
            num_bits += 1;
            state.num_bits = num_bits;
            if num_bits > LZW_MAX_BITS {
                return -1;
            }

            state.free_bits = if state.num_bits < state.max_bits {
                (1u64 << num_bits).wrapping_sub(256) as FtUInt
            } else {
                state.max_free + 1
            };
        }

        if state.buf_clear {
            num_bits = LZW_INIT_BITS;
            state.num_bits = num_bits;
            state.free_bits = (1u64 << num_bits).wrapping_sub(256) as FtUInt;
            state.buf_clear = false;
        }

        if ft_lzwstate_refill(state, source) < 0 {
            return -1;
        }

        offset = 0;
    }

    state.buf_offset = offset + num_bits;

    let tab = |i: usize| state.buf_tab.get(i).copied().unwrap_or(0) as FtInt;
    let mut p = (offset >> 3) as usize;
    offset &= 7;
    let mut result: FtInt = tab(p) >> offset;
    p += 1;
    offset = 8 - offset;
    num_bits -= offset;

    if num_bits >= 8 {
        result |= tab(p) << offset;
        p += 1;
        offset += 8;
        num_bits -= 8;
    }
    if num_bits > 0 {
        result |= (tab(p) & lzw_mask(num_bits) as FtInt) << offset;
    }

    result
}

/// `ft_lzwstate_stack_grow`: grow the character stack
fn ft_lzwstate_stack_grow(state: &mut FtLzwStateRec) -> i32 {
    if state.stack_top as usize >= state.stack_size {
        let old_size = state.stack_size;
        let mut new_size = old_size;

        new_size = new_size + (new_size >> 1) + 4;

        /* if relocating to heap */
        /* (the vector keeps its bytes) */

        /* requirement of the character stack larger than 1<<LZW_MAX_BITS */
        /* implies bug in the decompression code                          */
        if new_size > (1 << LZW_MAX_BITS) {
            new_size = 1 << LZW_MAX_BITS;
            if new_size == old_size {
                return -1;
            }
        }

        if ft_renew_array(&mut state.stack, new_size as FtLong).is_err() {
            return -1;
        }

        state.stack_size = new_size;
    }
    0
}

/// `ft_lzwstate_prefix_grow`: grow the prefix/suffix arrays
fn ft_lzwstate_prefix_grow(state: &mut FtLzwStateRec) -> i32 {
    let old_size: FtUInt = state.prefix_size;
    let mut new_size: FtUInt = old_size;

    if new_size == 0 {
        /* first allocation -> 9 bits */
        new_size = 512;
    } else {
        new_size += new_size >> 2; /* don't grow too fast */
    }

    /*
     * Note that the `suffix' array is located in the same memory block
     * pointed to by `prefix'.
     *
     * I know that sizeof(FT_Byte) == 1 by definition, but it is clearer
     * to write it literally.
     *
     */
    if ft_renew_array(&mut state.prefix, new_size as FtLong).is_err()
        || ft_renew_array(&mut state.suffix, new_size as FtLong).is_err()
    {
        return -1;
    }

    /* now adjust `suffix' and move the data accordingly */
    /* (two vectors: nothing to move) */

    state.prefix_size = new_size;
    0
}

/// `ft_lzwstate_reset`
pub fn ft_lzwstate_reset(state: &mut FtLzwStateRec) {
    state.in_eof = 0;
    state.buf_offset = 0;
    state.buf_size = 0;
    state.buf_clear = false;
    state.buf_total = 0;
    state.stack_top = 0;
    state.num_bits = LZW_INIT_BITS;
    state.phase = FtLzwPhase::Start;
}

/// `ft_lzwstate_init`
pub fn ft_lzwstate_init(state: &mut FtLzwStateRec) {
    *state = FtLzwStateRec::default();

    state.prefix = Vec::new();
    state.suffix = Vec::new();
    state.prefix_size = 0;

    state.stack = vec![0; FT_LZW_DEFAULT_STACK_SIZE];
    state.stack_size = FT_LZW_DEFAULT_STACK_SIZE;

    ft_lzwstate_reset(state);
}

/// `ft_lzwstate_done`
pub fn ft_lzwstate_done(state: &mut FtLzwStateRec) {
    ft_lzwstate_reset(state);

    *state = FtLzwStateRec::default();
}

/// `ft_lzwstate_io`: decompresses up to `out_size` bytes into `buffer`
/// (`None` to skip them); returns the number of bytes.
pub fn ft_lzwstate_io(
    state: &mut FtLzwStateRec,
    source: &mut FtStreamRec,
    mut buffer: Option<&mut [FtByte]>,
    out_size: FtULong,
) -> FtULong {
    let mut result: FtULong = 0;

    let mut old_char: FtUInt = state.old_char;
    let mut old_code: FtUInt = state.old_code;
    let mut in_code: FtUInt = state.in_code;

    /* FTLZW_STACK_PUSH: `false' goes to `Eof' */
    fn stack_push(state: &mut FtLzwStateRec, c: FtUInt) -> bool {
        if state.stack_top as usize >= state.stack_size && ft_lzwstate_stack_grow(state) < 0 {
            return false;
        }

        let top = state.stack_top as usize;
        state.stack[top] = c as FtByte;
        state.stack_top += 1;
        true
    }

    'exit: {
        if out_size == 0 {
            break 'exit;
        }

        let mut phase = state.phase;
        'eof: {
            loop {
                match phase {
                    FtLzwPhase::Start => {
                        /* skip magic bytes, and read max_bits + block_flag */
                        let mut max_bits = [0u8; 1];
                        if source.seek(2).is_err() || source.try_read(&mut max_bits) != 1 {
                            break 'eof;
                        }
                        let max_bits = max_bits[0];

                        state.max_bits = (max_bits & LZW_BIT_MASK) as FtUInt;
                        state.block_mode = (max_bits & LZW_BLOCK_MASK) as FtInt;
                        state.max_free = (1u64 << state.max_bits).wrapping_sub(256) as FtUInt;

                        if state.max_bits > LZW_MAX_BITS {
                            break 'eof;
                        }

                        state.num_bits = LZW_INIT_BITS;
                        state.free_ent = (if state.block_mode != 0 {
                            LZW_FIRST
                        } else {
                            LZW_CLEAR
                        }) - 256;
                        in_code = 0;

                        state.free_bits = if state.num_bits < state.max_bits {
                            (1u64 << state.num_bits).wrapping_sub(256) as FtUInt
                        } else {
                            state.max_free + 1
                        };

                        let c = ft_lzwstate_get_code(state, source);
                        if !(0..=255).contains(&c) {
                            break 'eof;
                        }

                        old_code = c as FtUInt;
                        old_char = c as FtUInt;

                        if let Some(b) = buffer.as_deref_mut() {
                            b[result as usize] = old_char as FtByte;
                        }

                        result += 1;
                        if result >= out_size {
                            break 'exit;
                        }

                        state.phase = FtLzwPhase::Code;
                        phase = FtLzwPhase::Code;
                        /* FALL_THROUGH */
                    }

                    FtLzwPhase::Code => {
                        /* NextCode: */
                        let c = ft_lzwstate_get_code(state, source);
                        if c < 0 {
                            break 'eof;
                        }

                        let mut code = c as FtUInt;

                        if code == LZW_CLEAR && state.block_mode != 0 {
                            /* why not LZW_FIRST-256 ? */
                            state.free_ent = (LZW_FIRST - 1) - 256;
                            state.buf_clear = true;

                            /* not quite right, but at least more predictable */
                            old_code = 0;
                            old_char = 0;

                            continue; /* goto NextCode */
                        }

                        in_code = code; /* save code for later */

                        if code >= 256 {
                            /* special case for KwKwKwK */
                            if code - 256 >= state.free_ent {
                                /* corrupted LZW stream */
                                if code - 256 > state.free_ent {
                                    break 'eof;
                                }

                                if !stack_push(state, old_char) {
                                    break 'eof;
                                }
                                code = old_code;
                            }

                            while code >= 256 {
                                if state.prefix.is_empty() {
                                    break 'eof;
                                }

                                let s = state
                                    .suffix
                                    .get((code - 256) as usize)
                                    .copied()
                                    .unwrap_or(0);
                                if !stack_push(state, s as FtUInt) {
                                    break 'eof;
                                }
                                code = state
                                    .prefix
                                    .get((code - 256) as usize)
                                    .copied()
                                    .unwrap_or(0) as FtUInt;
                            }
                        }

                        old_char = code;
                        if !stack_push(state, old_char) {
                            break 'eof;
                        }

                        state.phase = FtLzwPhase::Stack;
                        phase = FtLzwPhase::Stack;
                        /* FALL_THROUGH */
                    }

                    FtLzwPhase::Stack => {
                        while state.stack_top > 0 {
                            state.stack_top -= 1;

                            if let Some(b) = buffer.as_deref_mut() {
                                b[result as usize] = state.stack[state.stack_top as usize];
                            }

                            result += 1;
                            if result == out_size {
                                break 'exit;
                            }
                        }

                        /* now create new entry */
                        if state.free_ent < state.max_free {
                            if state.free_ent >= state.prefix_size
                                && ft_lzwstate_prefix_grow(state) < 0
                            {
                                break 'eof;
                            }

                            debug_assert!(state.free_ent < state.prefix_size);

                            let fe = state.free_ent as usize;
                            state.prefix[fe] = old_code as FtUShort;
                            state.suffix[fe] = old_char as FtByte;

                            state.free_ent += 1;
                        }

                        old_code = in_code;

                        state.phase = FtLzwPhase::Code;
                        phase = FtLzwPhase::Code;
                        /* goto NextCode */
                    }

                    FtLzwPhase::Eof => {
                        /* default:  state == EOF */
                        break 'exit;
                    }
                }
            }
        }

        /* Eof: */
        state.phase = FtLzwPhase::Eof;
    }

    /* Exit: */
    state.old_code = old_code;
    state.old_char = old_char;
    state.in_code = in_code;

    result
}
