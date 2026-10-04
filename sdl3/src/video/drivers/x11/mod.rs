// Rust translation of src/video/x11/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The X11 video driver.
//!
//! Every X library is loaded at run time with [`SharedObject`](crate::loadso::SharedObject), as upstream's `SDL_x11dyn.c` does, so
//! programs start (and fall back to other drivers) without X11 installed:
//! libX11, libXext (MIT-SHM, XShape, XSync), libXcursor, libXi (XInput2),
//! libXfixes, libXrandr, libXss and libXtst; libvulkan and libX11-xcb for
//! Vulkan surfaces. The declarations are written by hand in [`sys`] and
//! [`x11dyn`] (`dyn.rs`).
//!
//! One module per upstream file: [`video`] (`SDL_x11video.c`), [`modes`],
//! [`window`], [`events`], [`keyboard`], [`mouse`], [`framebuffer`],
//! [`clipboard`], [`xinput2`], [`xfixes`], [`xsync`], [`xtest`],
//! [`settings`] with [`xsettings_client`], [`edid`], [`shape`], [`pen`],
//! [`touch`], [`vulkan`], and [`messagebox`] with its [`toolkit`]
//! (message boxes try zenity first, see [`crate::dialog`]; the toolkit is
//! a build without FriBidi and libthai).
//!
//! Not translated (yet):
//!
//! * OpenGL (`SDL_x11opengl.c`, `SDL_x11opengles.c`): the OpenGL front end
//!   behaves as a build without OpenGL, so GLX and EGL contexts are not
//!   provided and windows get the default visual.
//! * The D-Bus integration (the IBus/Fcitx input methods of
//!   `SDL_IBus_*`/`SDL_Fcitx_*`, the screensaver inhibition and the system
//!   theme): it comes with the D-Bus layer; X input methods (XIM) work.

#![allow(non_upper_case_globals, non_snake_case)] // (X11 names are kept)

pub(crate) mod clipboard;
pub(crate) mod edid;
pub mod events;
pub(crate) mod framebuffer;
pub(crate) mod keyboard;
pub(crate) mod messagebox;
pub(crate) mod modes;
pub(crate) mod mouse;
pub(crate) mod pen;
pub(crate) mod settings;
pub(crate) mod shape;
pub(crate) mod sys;
pub(crate) mod toolkit;
pub(crate) mod touch;
pub(crate) mod video;
pub(crate) mod vulkan;
pub(crate) mod window;
#[path = "dyn.rs"]
pub(crate) mod x11dyn;
pub(crate) mod xfixes;
pub(crate) mod xinput2;
pub(crate) mod xsettings_client;
pub(crate) mod xsync;
pub(crate) mod xtest;

#[cfg(test)]
mod tests;
