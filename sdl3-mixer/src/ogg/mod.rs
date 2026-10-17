//! A Rust translation of the decoding half of libogg 1.3.5 (`src/framing.c`
//! and `src/bitwise.c`), as opusfile and libvorbis use it.

// The translation keeps upstream's loops, declarations and arithmetic,
// which these lints would have rewritten.
#![allow(
    clippy::assign_op_pattern,
    clippy::field_reassign_with_default,
    clippy::int_plus_one,
    clippy::manual_range_contains,
    clippy::needless_late_init,
    clippy::needless_range_loop
)]

pub(crate) mod bitwise;
pub(crate) mod framing;
