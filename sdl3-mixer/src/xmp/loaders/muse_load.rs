// Rust translation of src/loaders/muse_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2022 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The MUSE container loader: a zlib stream of a Galaxy Music System 5.0
//! or 4.0 module.

use super::super::common::*;
use super::super::dataio::*;
use super::super::hio::*;
use super::super::miniz_tinfl::*;
use super::gal4_load::LIBXMP_LOADER_GAL4;
use super::gal5_load::LIBXMP_LOADER_GAL5;
use super::*;

pub(crate) const LIBXMP_LOADER_MUSE: FormatLoader = FormatLoader {
    name: "MUSE container",
    test: muse_test,
    loader: muse_load,
};

/// Translation of `muse_test()`.
fn muse_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, _start: i32) -> i32 {
    let mut in_ = [0u8; 8];

    if hio_read(&mut in_, 1, 8, f) != 8 {
        return -1;
    }
    if &in_[..4] != b"MUSE" {
        return -1;
    }
    let r = readmem32b(&in_[4..]);
    if r != 0xdeadbeaf && r != 0xdeadbabe {
        return -1;
    }
    if let Some(t) = t {
        t[0] = b'\0'; /* FIXME */
    }
    0
}

/// Translation of `muse_load()`.
fn muse_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut inlen = hio_size(f);
    if !(24..LIBXMP_DEPACK_LIMIT).contains(&inlen) {
        return -1;
    }
    if hio_seek(f, 24, SEEK_SET) < 0 {
        return -1;
    }

    inlen -= 24;
    let Ok(mut in_) = try_vec(inlen as usize, 0u8) else {
        return -1;
    };
    if hio_read(&mut in_, 1, inlen as usize, f) != inlen as usize {
        return -1;
    }

    let Some(out) = tinfl_decompress_mem_to_heap(&in_, TINFL_FLAG_PARSE_ZLIB_HEADER) else {
        return -1;
    };
    drop(in_);

    if hio_reopen_mem(out, f) < 0 {
        return -1;
    }
    let mut err = (LIBXMP_LOADER_GAL5.test)(f, None, 0);
    hio_seek(f, 0, SEEK_SET);
    if err == 0 {
        err = (LIBXMP_LOADER_GAL5.loader)(m, f, 0);
    } else {
        err = (LIBXMP_LOADER_GAL4.test)(f, None, 0);
        hio_seek(f, 0, SEEK_SET);
        if err == 0 {
            err = (LIBXMP_LOADER_GAL4.loader)(m, f, 0);
        }
    }

    err
}
