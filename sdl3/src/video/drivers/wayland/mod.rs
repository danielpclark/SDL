// Rust translation of src/video/wayland/ from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The Wayland video driver.
//!
//! libwayland-client, libwayland-cursor, libxkbcommon and (optionally)
//! libdecor are loaded at run time ([`wldyn`]); their functions are declared
//! by hand ([`sys`]). The protocol code (`wl_interface` tables, typed
//! requests and decoded events) is generated from the XML files in
//! `tools/wayland-protocols/` by `tools/gen_wayland_protocols.py` into
//! [`protocols`], over the proxy runtime of [`client`]: owned proxies are
//! destroyed when they drop, and listeners are closures.
//!
//! One module per upstream file:
//!
//! | upstream                    | module            |
//! |-----------------------------|-------------------|
//! | `SDL_waylandvideo.c`        | [`video`]         |
//! | `SDL_waylandevents.c`       | [`events`]        |
//! | `SDL_waylandwindow.c`       | [`window`]        |
//! | `SDL_waylandmouse.c`        | [`mouse`]         |
//! | `SDL_waylandkeyboard.c`     | [`keyboard`]      |
//! | `SDL_waylandclipboard.c`    | [`clipboard`]     |
//! | `SDL_waylanddatamanager.c`  | [`datamanager`]   |
//! | `SDL_waylandcolor.c`        | [`color`]         |
//! | `SDL_waylandeventthread.c`  | [`eventthread`]   |
//! | `SDL_waylandshmbuffer.c`    | [`shmbuffer`]     |
//! | `SDL_waylandmessagebox.c`   | [`messagebox`]    |
//! | `SDL_waylandutil.c`         | [`util`]          |
//! | `SDL_waylandvulkan.c`       | [`vulkan`]        |
//! | `SDL_waylanddyn.c`          | [`wldyn`]         |
//! | `SDL_waylandsym.h`          | [`wldyn`], [`sys`] |
//!
//! Not translated: `SDL_waylandopengles.c` (OpenGL ES through EGL, as the
//! crate has no EGL layer); windows get no `wl_egl_window`. In its place,
//! [`framebuffer`] (not in upstream) gives windows a `wl_shm` framebuffer
//! for `Window::surface()`, where upstream draws the window surface through
//! a GLES texture.

pub(crate) mod client;
pub(crate) mod clipboard;
pub(crate) mod color;
pub(crate) mod datamanager;
pub(crate) mod events;
pub(crate) mod eventthread;
pub(crate) mod framebuffer;
pub(crate) mod keyboard;
pub(crate) mod messagebox;
pub(crate) mod mouse;
pub(crate) mod protocols;
pub(crate) mod shmbuffer;
pub(crate) mod sys;
pub(crate) mod util;
pub(crate) mod video;
pub(crate) mod vulkan;
pub(crate) mod window;
#[path = "dyn.rs"]
pub(crate) mod wldyn;

#[cfg(test)]
mod tests;
