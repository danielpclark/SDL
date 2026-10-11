// Rust translation of src/pcf/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it): the PCF font driver of Francesco Zappa
// Nardelli, under its own MIT-style license, and the bitmap utilities of
// The Open Group (`pcfutil`), under theirs (see the files' headers), not
// the FreeType License.

//! The PCF font driver (`pcfdrivr`), its font loader (`pcfread`) and
//! records (`pcf`), and bitmap utilities (`pcfutil`).

#[allow(clippy::module_inception)]
pub mod pcf;
pub mod pcfdrivr;
pub mod pcfread;
pub mod pcfutil;
