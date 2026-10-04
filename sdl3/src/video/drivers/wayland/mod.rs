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

#![allow(dead_code)] // (the driver using these comes next)

pub(crate) mod client;
pub(crate) mod protocols;
pub(crate) mod sys;
#[path = "dyn.rs"]
pub(crate) mod wldyn;
