// Rust translation of src/loaders/loader.h and src/format.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The module format loaders, and what they share.

use super::common::ModuleData;
use super::hio::HioHandle;

pub(crate) mod abk_load;
pub(crate) mod amf_load;
pub(crate) mod arch_load;
pub(crate) mod asylum_load;
pub(crate) mod c669_load;
pub(crate) mod chip_load;
pub(crate) mod coco_load;
pub(crate) mod common;
pub(crate) mod dbm_load;
pub(crate) mod digi_load;
pub(crate) mod dt_load;
pub(crate) mod emod_load;
pub(crate) mod far_load;
pub(crate) mod flt_load;
pub(crate) mod fnk_load;
pub(crate) mod gal4_load;
pub(crate) mod gal5_load;
pub(crate) mod gdm_load;
pub(crate) mod hmn_load;
pub(crate) mod ice_load;
pub(crate) mod iff;
pub(crate) mod imf_load;
pub(crate) mod ims_load;
pub(crate) mod it_load;
pub(crate) mod itsex;
pub(crate) mod liq_load;
pub(crate) mod lzw;
pub(crate) mod masi16_load;
pub(crate) mod masi_load;
pub(crate) mod mdl_load;
pub(crate) mod med;
pub(crate) mod med2_load;
pub(crate) mod med3_load;
pub(crate) mod med4_load;
pub(crate) mod mfp_load;
pub(crate) mod mgt_load;
pub(crate) mod mmd1_load;
pub(crate) mod mmd3_load;
pub(crate) mod mmd_common;
pub(crate) mod mod_load;
pub(crate) mod mtm_load;
pub(crate) mod muse_load;
pub(crate) mod no_load;
pub(crate) mod okt_load;
pub(crate) mod pt3_load;
pub(crate) mod ptm_load;
pub(crate) mod rtm_load;
pub(crate) mod s3m_load;
pub(crate) mod sample;
pub(crate) mod sfx_load;
pub(crate) mod st_load;
pub(crate) mod stim_load;
pub(crate) mod stm_load;
pub(crate) mod stx_load;
pub(crate) mod sym_load;
pub(crate) mod ult_load;
pub(crate) mod umx_load;
pub(crate) mod voltable;
pub(crate) mod xm_load;
pub(crate) mod xmf_load;

/* Sample flags */
pub(crate) const SAMPLE_FLAG_DIFF: i32 = 0x0001; /* Differential */
pub(crate) const SAMPLE_FLAG_UNS: i32 = 0x0002; /* Unsigned */
pub(crate) const SAMPLE_FLAG_8BDIFF: i32 = 0x0004;
pub(crate) const SAMPLE_FLAG_7BIT: i32 = 0x0008;
pub(crate) const SAMPLE_FLAG_NOLOAD: i32 = 0x0010; /* Get from buffer, don't load */
pub(crate) const SAMPLE_FLAG_BIGEND: i32 = 0x0040; /* Big-endian */
pub(crate) const SAMPLE_FLAG_VIDC: i32 = 0x0080; /* Archimedes VIDC logarithmic */
pub(crate) const SAMPLE_FLAG_INTERLEAVED: i32 = 0x0100; /* Interleaved stereo sample */
pub(crate) const SAMPLE_FLAG_FULLREP: i32 = 0x0200; /* Play full sample before looping */
pub(crate) const SAMPLE_FLAG_ADLIB: i32 = 0x1000; /* Adlib synth instrument */
pub(crate) const SAMPLE_FLAG_HSC: i32 = 0x2000; /* HSC Adlib synth instrument */
pub(crate) const SAMPLE_FLAG_ADPCM: i32 = 0x4000; /* ADPCM4 encoded samples */

/* libxmp_test_name flags */
pub(crate) const TEST_NAME_IGNORE_AFTER_0: i32 = 0x0001;
pub(crate) const TEST_NAME_IGNORE_AFTER_CR: i32 = 0x0002;

/// `DEFPAN(x)`
#[inline]
pub(crate) fn defpan(m: &ModuleData, x: i32) -> i32 {
    0x80 + (x - 0x80) * m.defpan / 100
}

/// `MAGIC4(a,b,c,d)`
#[inline]
pub(crate) const fn magic4(a: u8, b: u8, c: u8, d: u8) -> u32 {
    ((a as u32) << 24) | ((b as u32) << 16) | ((c as u32) << 8) | (d as u32)
}

/// A format loader's `test`: 0 if the file is in its format, writing the
/// module's title into the buffer if one is given.
pub(crate) type TestFn = fn(&mut HioHandle<'_>, Option<&mut [u8]>, i32) -> i32;

/// A format loader's `loader`.
pub(crate) type LoaderFn = fn(&mut ModuleData, &mut HioHandle<'_>, i32) -> i32;

/// Translation of `struct format_loader`.
pub(crate) struct FormatLoader {
    pub name: &'static str,
    pub test: TestFn,
    pub loader: LoaderFn,
}

/// `printf("%W.Ps", s)`: `s` up to its NUL or `prec` bytes, right-aligned
/// in `width` with spaces.
pub(crate) fn printf_s(s: &[u8], width: usize, prec: usize) -> Vec<u8> {
    let s = super::cstr(&s[..s.len().min(prec)]);
    let mut v = vec![b' '; width.saturating_sub(s.len())];
    v.extend_from_slice(s);
    v
}

/// `printf("%-W.Ps", s)`: `s` up to its NUL or `prec` bytes, left-aligned
/// in `width` with spaces.
pub(crate) fn printf_s_left(s: &[u8], width: usize, prec: usize) -> Vec<u8> {
    let mut v = super::cstr(&s[..s.len().min(prec)]).to_vec();
    while v.len() < width {
        v.push(b' ');
    }
    v
}
