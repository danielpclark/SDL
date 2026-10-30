// Rust translation of android-project/app/src/main/java/org/libsdl/app/
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Upstream's Java, one module per file, in its order: the calls to the
//! framework are JNI calls (through [`super::env`]), the calls between
//! upstream's classes and to its natives are Rust calls, and the methods
//! the framework calls are the native methods of the classes
//! [`super::classes`] declares. Java's statics are a module's state.
//!
//! A Java exception that upstream's code doesn't catch leaves the method
//! (here `Err(Thrown)`, through `?`) and, from a native method, reaches the
//! framework, as Java's would; `catch (Exception e)` is
//! [`super::env::Thrown::catch_exception`].

pub(crate) mod activity;
pub(crate) mod anonymous;
pub(crate) mod controller_manager;
pub(crate) mod sdl;
pub(crate) mod sensor_manager;
pub(crate) mod surface;
