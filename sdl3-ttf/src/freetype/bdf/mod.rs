// Rust translation of src/bdf/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it): the BDF font driver of Francesco Zappa
// Nardelli and the BDF font loader of the Computing Research Labs, New
// Mexico State University, under their own MIT-style licenses (see the
// files' headers), not the FreeType License.

//! The BDF font driver (`bdfdrivr`) and loader (`bdflib`).

pub mod bdfdrivr;
pub mod bdflib;
