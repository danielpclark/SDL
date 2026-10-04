// The PNG and JPEG codecs SDL bundles: translations of src/video/stb_image.h
// and src/video/miniz.h as SDL_stb.c configures them.
// See each file for its origin and notice.

//! The image codecs behind [`Surface::load_png`](crate::video::Surface::load_png),
//! [`Surface::load_jpg`](crate::video::Surface::load_jpg) and
//! [`Surface::save_png`](crate::video::Surface::save_png).

pub(crate) mod jpeg;
pub(crate) mod miniz;
pub(crate) mod png;
pub(crate) mod stb_image;
pub(crate) mod zlib;

#[cfg(test)]
mod tests;
