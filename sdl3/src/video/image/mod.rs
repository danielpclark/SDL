// The PNG and JPEG codecs SDL bundles: translations of src/video/stb_image.h
// and src/video/miniz.h as SDL_stb.c configures them; and stb_image's other
// loaders, which SDL compiles out, from the copy of the same stb_image
// release PlutoVG bundles (for sdl3-ttf's SVG images).
// See each file for its origin and notice.

//! The image codecs behind [`Surface::load_png`](crate::video::Surface::load_png),
//! [`Surface::load_jpg`](crate::video::Surface::load_jpg) and
//! [`Surface::save_png`](crate::video::Surface::save_png), and
//! [`stb_image::load_from_memory_all_formats`](crate::video::stb_image::load_from_memory_all_formats).

pub(crate) mod bmp;
pub(crate) mod gif;
pub(crate) mod hdr;
pub(crate) mod jpeg;
pub(crate) mod miniz;
pub(crate) mod pic;
pub(crate) mod png;
pub(crate) mod pnm;
pub(crate) mod psd;
pub(crate) mod stb_image;
pub(crate) mod tga;
pub(crate) mod zlib;

#[cfg(test)]
mod tests;
