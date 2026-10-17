// Rust translation of src/format.c and src/format.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2024 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The format loaders, in the order they're tried. (The ProWizard
//! loader that ends upstream's list is compiled out by SDL_mixer's build.)

use super::loaders::it_load::LIBXMP_LOADER_IT;
use super::loaders::mod_load::LIBXMP_LOADER_MOD;
use super::loaders::s3m_load::LIBXMP_LOADER_S3M;
use super::loaders::xm_load::LIBXMP_LOADER_XM;
use super::loaders::FormatLoader;

/// Translation of `format_loaders`.
pub(crate) static FORMAT_LOADERS: &[&FormatLoader] = &[
    &LIBXMP_LOADER_XM,
    &LIBXMP_LOADER_MOD,
    &LIBXMP_LOADER_IT,
    &LIBXMP_LOADER_S3M,
];
