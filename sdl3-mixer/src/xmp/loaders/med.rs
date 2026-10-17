// Rust translation of src/loaders/med.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player (the header has no notice of its own; libxmp's:)
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The MED and OctaMED formats' versions.

pub(crate) const MMD_INST_TYPES: i32 = 9;

pub(crate) const MED_VER_210: i32 = 0x0210;
pub(crate) const MED_VER_300: i32 = 0x0300;
pub(crate) const MED_VER_320: i32 = 0x0320;
pub(crate) const MED_VER_OCTAMED_100: i32 = 0x1000;
pub(crate) const MED_VER_OCTAMED_200: i32 = 0x2000;
pub(crate) const MED_VER_OCTAMED_300: i32 = 0x3000;
pub(crate) const MED_VER_OCTAMED_400: i32 = 0x4000;
pub(crate) const MED_VER_OCTAMED_500: i32 = 0x5000;
pub(crate) const MED_VER_OCTAMED_502: i32 = 0x5002;
pub(crate) const MED_VER_OCTAMED_SS_1: i32 = 0x6000;
pub(crate) const MED_VER_OCTAMED_SS_2: i32 = 0x7000;
