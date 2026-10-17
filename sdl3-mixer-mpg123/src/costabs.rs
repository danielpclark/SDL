// cos MPEG decoding tables (output of src/libmpg123/calctables cos)
// copyright by the mpg123 project - free software under the terms of the LGPL 2.1
// see COPYING and AUTHORS files in distribution or http://mpg123.org
// Modified 2026-10-17: translated into Rust from costabs.h in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).
// The float tables of a build without RUNTIME_TABLES, as calctables wrote them
// (generated from the C header by a script).

// (The C literals have no float suffix: they are doubles, rounded to float.)

/// `cos64`
#[rustfmt::skip]
pub(crate) static COS64: [f32; 16] = [
    5.00602998e-01_f64 as f32, 5.05470960e-01_f64 as f32,
    5.15447310e-01_f64 as f32, 5.31042591e-01_f64 as f32,
    5.53103896e-01_f64 as f32, 5.82934968e-01_f64 as f32,
    6.22504123e-01_f64 as f32, 6.74808341e-01_f64 as f32,
    7.44536271e-01_f64 as f32, 8.39349645e-01_f64 as f32,
    9.72568238e-01_f64 as f32, 1.16943993e+00_f64 as f32,
    1.48416462e+00_f64 as f32, 2.05778101e+00_f64 as f32,
    3.40760842e+00_f64 as f32, 1.01900081e+01_f64 as f32,
];

/// `cos32`
#[rustfmt::skip]
pub(crate) static COS32: [f32; 8] = [
    5.02419286e-01_f64 as f32, 5.22498615e-01_f64 as f32,
    5.66944035e-01_f64 as f32, 6.46821783e-01_f64 as f32,
    7.88154623e-01_f64 as f32, 1.06067769e+00_f64 as f32,
    1.72244710e+00_f64 as f32, 5.10114862e+00_f64 as f32,
];

/// `cos16`
#[rustfmt::skip]
pub(crate) static COS16: [f32; 4] = [
    5.09795579e-01_f64 as f32, 6.01344887e-01_f64 as f32,
    8.99976223e-01_f64 as f32, 2.56291545e+00_f64 as f32,
];

/// `cos8`
#[rustfmt::skip]
pub(crate) static COS8: [f32; 2] = [
    5.41196100e-01_f64 as f32, 1.30656296e+00_f64 as f32,
];

/// `cos4`
#[rustfmt::skip]
pub(crate) static COS4: [f32; 1] = [
    7.07106781e-01_f64 as f32,
];
