// Rust translation of src/gzip/ from FreeType (2.13.2, as SDL_ttf's
// external/freetype pins it), with the zlib 1.3 inflater it bundles.

//! The `gzip` component: zlib decompression for WOFF tables and gzipped
//! SVG documents, and the gzip stream the PCF driver reads compressed
//! fonts with (`FT_CONFIG_OPTION_USE_ZLIB` with the bundled zlib, as
//! SDL_ttf builds FreeType).

pub mod ftgzip;
pub mod zlib;
