/*
    icy: support for SHOUTcast ICY meta info, an attempt to keep it organized

    copyright 2006-2023 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Thomas Orgis and modelled after patch by Honza
*/
// Modified 2026-10-17: translated into Rust from icy.h and icy.c in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The ICY metadata store. (`icy2utf8.c`, which converts it for
//! `mpg123_icy2utf8()` and `mpg123_store_utf8()`, isn't translated: the
//! library itself never calls it.)

/// Translation of `struct icy_meta`.
#[derive(Debug, Default)]
pub(crate) struct IcyMeta {
    pub data: Option<Vec<u8>>,
    pub interval: i64,
    pub next: i64,
}

/// Translation of `INT123_init_icy()`.
pub(crate) fn init_icy(icy: &mut IcyMeta) {
    icy.data = None;
}

/// Translation of `INT123_clear_icy()`.
pub(crate) fn clear_icy(icy: &mut IcyMeta) {
    init_icy(icy);
}

/// Translation of `INT123_reset_icy()`.
pub(crate) fn reset_icy(icy: &mut IcyMeta) {
    clear_icy(icy);
    init_icy(icy);
}
