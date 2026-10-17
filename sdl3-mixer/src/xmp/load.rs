// Rust translation of src/load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Testing and loading modules, and releasing them.
//!
//! Only the memory and callback entry points are translated: the path and
//! `FILE *` ones (`xmp_test_module()`, `xmp_load_module()`,
//! `xmp_test_module_from_file()`, `xmp_load_module_from_file()`, with
//! `get_dirname()` and `get_basename()`) open files by name, which
//! SDL_mixer doesn't do, and the depackers they'd run are compiled out.

use std::borrow::Cow;

use super::common::*;
use super::extras::libxmp_release_module_extras;
use super::format::FORMAT_LOADERS;
use super::hio::*;
use super::load_helpers::*;
use super::md5::Md5Ctx;
use super::player::xmp_end_player;
use super::scan::libxmp_scan_sequences;
use super::*;

const BUFLEN: usize = 16384;

/// Translation of `set_md5sum()`.
fn set_md5sum(f: &mut HioHandle<'_>, digest: &mut [u8; 16]) {
    let mut buf = [0u8; BUFLEN];

    hio_seek(f, 0, SEEK_SET);

    let mut ctx = Md5Ctx::new();
    loop {
        let bytes_read = hio_read(&mut buf, 1, BUFLEN, f);
        if bytes_read == 0 {
            break;
        }
        ctx.update(&buf[..bytes_read]);
    }
    *digest = ctx.finish();
}

/// Translation of `test_module()`.
fn test_module(mut info: Option<&mut XmpTestInfo>, h: &mut HioHandle<'_>) -> i32 {
    let mut buf = [0u8; XMP_NAME_SIZE];

    if let Some(info) = info.as_deref_mut() {
        info.name[0] = 0; /* reset name prior to testing */
        info.type_[0] = 0; /* reset type prior to testing */
    }

    for loader in FORMAT_LOADERS {
        hio_seek(h, 0, SEEK_SET);
        if (loader.test)(h, Some(&mut buf), 0) == 0 {
            if let Some(info) = info {
                strncpy(&mut info.name, &buf, XMP_NAME_SIZE - 1);
                info.name[XMP_NAME_SIZE - 1] = 0;

                strncpy(&mut info.type_, loader.name.as_bytes(), XMP_NAME_SIZE - 1);
                info.type_[XMP_NAME_SIZE - 1] = 0;
            }
            return 0;
        }
    }
    -XMP_ERROR_FORMAT
}

/// Translation of `xmp_test_module_from_memory()`.
pub(crate) fn xmp_test_module_from_memory(mem: &[u8], info: Option<&mut XmpTestInfo>) -> i32 {
    if mem.is_empty() {
        return -XMP_ERROR_INVALID;
    }

    let Some(mut h) = hio_open_const_mem(Cow::Borrowed(mem)) else {
        return -XMP_ERROR_SYSTEM;
    };

    test_module(info, &mut h)
}

/// Translation of `xmp_test_module_from_callbacks()`.
pub(crate) fn xmp_test_module_from_callbacks(
    priv_: &mut dyn XmpCallbacks,
    info: Option<&mut XmpTestInfo>,
) -> i32 {
    let Some(mut h) = hio_open_callbacks(priv_) else {
        return -XMP_ERROR_SYSTEM;
    };

    test_module(info, &mut h)
}

/// The pattern and track sanity checks of `load_module()`: whether they
/// fail.
fn bad_patterns(mod_: &XmpModule) -> bool {
    /* Sanity check: patterns */
    let Some(xxp) = &mod_.xxp else {
        return true;
    };
    for i in 0..mod_.pat as usize {
        let Some(Some(pat)) = xxp.get(i) else {
            return true;
        };
        for j in 0..mod_.chn as usize {
            let Some(&t) = pat.index.get(j) else {
                return true;
            };
            if t < 0
                || t >= mod_.trk
                || !matches!(
                    mod_.xxt.as_ref().and_then(|xxt| xxt.get(t as usize)),
                    Some(Some(_))
                )
            {
                return true;
            }
        }
    }
    false
}

/// Translation of `load_module()`.
fn load_module(ctx: &mut ContextData, h: &mut HioHandle<'_>) -> i32 {
    libxmp_load_prologue(ctx);

    let mut test_result = -1;
    let mut load_result = -1;
    for loader in FORMAT_LOADERS {
        hio_seek(h, 0, SEEK_SET);

        test_result = (loader.test)(h, None, 0);
        if test_result == 0 {
            hio_seek(h, 0, SEEK_SET);
            load_result = (loader.loader)(&mut ctx.m, h, 0);
            break;
        }
    }

    if test_result < 0 {
        xmp_release_module(ctx);
        return -XMP_ERROR_FORMAT;
    }

    'err_load: {
        if load_result < 0 {
            break 'err_load;
        }

        let mod_ = &mut ctx.m.mod_;

        /* Sanity check: number of channels, module length */
        if mod_.chn > XMP_MAX_CHANNELS as i32 || mod_.len > XMP_MAX_MOD_LENGTH as i32 {
            break 'err_load;
        }

        /* Sanity check: channel pan */
        for i in 0..mod_.chn as usize {
            if mod_.xxc[i].vol < 0 || mod_.xxc[i].vol > 0xff {
                break 'err_load;
            }
            if mod_.xxc[i].pan < 0 || mod_.xxc[i].pan > 0xff {
                break 'err_load;
            }
        }

        if bad_patterns(mod_) {
            break 'err_load;
        }

        {
            /* (name and type are adjacent in upstream's struct xmp_module,
             * so a name that fills its XMP_NAME_SIZE bytes runs on into the
             * type) */
            let mut both = [0u8; 2 * XMP_NAME_SIZE];
            both[..XMP_NAME_SIZE].copy_from_slice(&mod_.name);
            both[XMP_NAME_SIZE..].copy_from_slice(&mod_.type_);
            libxmp_adjust_string(&mut both);
            mod_.name.copy_from_slice(&both[..XMP_NAME_SIZE]);
            mod_.type_.copy_from_slice(&both[XMP_NAME_SIZE..]);
        }
        for i in 0..mod_.ins as usize {
            if let Some(xxi) = mod_.xxi.get_mut(i) {
                libxmp_adjust_string(&mut xxi.name);
            }
        }
        for i in 0..mod_.smp as usize {
            if let Some(xxs) = mod_.xxs.get_mut(i) {
                libxmp_adjust_string(&mut xxs.name);
            }
        }

        if test_result == 0 && load_result == 0 {
            set_md5sum(h, &mut ctx.m.md5);
        }

        libxmp_load_epilogue(ctx);

        let ret = libxmp_prepare_scan(ctx);
        if ret < 0 {
            xmp_release_module(ctx);
            return ret;
        }

        let ret = libxmp_scan_sequences(ctx);
        if ret < 0 {
            xmp_release_module(ctx);
            return -XMP_ERROR_LOAD;
        }

        ctx.state = XMP_STATE_LOADED;

        return 0;
    }

    xmp_release_module(ctx);
    -XMP_ERROR_LOAD
}

/// Translation of `xmp_load_module_from_memory()`.
pub(crate) fn xmp_load_module_from_memory(ctx: &mut ContextData, mem: &[u8]) -> i32 {
    if mem.is_empty() {
        return -XMP_ERROR_INVALID;
    }

    let Some(mut h) = hio_open_const_mem(Cow::Borrowed(mem)) else {
        return -XMP_ERROR_SYSTEM;
    };

    if ctx.state > XMP_STATE_UNLOADED {
        xmp_release_module(ctx);
    }

    let m = &mut ctx.m;
    m.filename = None;
    m.basename = None;
    m.dirname = None;
    m.size = mem.len() as i32;

    load_module(ctx, &mut h)
}

/// Translation of `xmp_load_module_from_callbacks()`.
pub(crate) fn xmp_load_module_from_callbacks(
    ctx: &mut ContextData,
    priv_: &mut dyn XmpCallbacks,
) -> i32 {
    let Some(mut h) = hio_open_callbacks(priv_) else {
        return -XMP_ERROR_SYSTEM;
    };

    if ctx.state > XMP_STATE_UNLOADED {
        xmp_release_module(ctx);
    }

    let m = &mut ctx.m;
    m.filename = None;
    m.basename = None;
    m.dirname = None;
    m.size = hio_size(&h) as i32;

    load_module(ctx, &mut h)
}

/// Translation of `xmp_release_module()`.
pub(crate) fn xmp_release_module(ctx: &mut ContextData) {
    /* can't test this here, we must call release_module to clean up
     * load errors
    if (ctx->state < XMP_STATE_LOADED)
        return;
     */

    if ctx.state > XMP_STATE_LOADED {
        xmp_end_player(ctx);
    }

    ctx.state = XMP_STATE_UNLOADED;

    libxmp_release_module_extras(ctx);

    let m = &mut ctx.m;
    let mod_ = &mut m.mod_;

    mod_.xxt = None;
    mod_.xxp = None;
    mod_.xxi = Vec::new();
    mod_.xxs = Vec::new();

    m.xtra = Vec::new();
    m.midi = None;

    libxmp_free_scan(ctx);

    let m = &mut ctx.m;
    m.comment = None;

    m.basename = None;
    m.dirname = None;
}
