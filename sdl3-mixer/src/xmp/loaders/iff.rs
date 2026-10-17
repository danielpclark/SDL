// Rust translation of src/loaders/iff.c and src/loaders/iff.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Reading the chunks of IFF-like containers, each with the loader
//! registered for its id.
//!
//! The loaders' `void *parm` is the parameter type `P` of [`IffHandle`].

use super::super::common::ModuleData;
use super::super::hio::*;

pub(crate) const IFF_NOBUFFER: i32 = 0x0001;

pub(crate) const IFF_LITTLE_ENDIAN: u32 = 0x01;
pub(crate) const IFF_FULL_CHUNK_SIZE: u32 = 0x02;
pub(crate) const IFF_CHUNK_ALIGN2: u32 = 0x04;
pub(crate) const IFF_CHUNK_ALIGN4: u32 = 0x08;
pub(crate) const IFF_SKIP_EMBEDDED: u32 = 0x10;
pub(crate) const IFF_CHUNK_TRUNC4: u32 = 0x20;

pub(crate) const IFF_MAX_CHUNK_SIZE: u32 = 0x800000;

/// `iff_loader`
pub(crate) type IffLoader<P> = fn(&mut ModuleData, u32, &mut HioHandle<'_>, &mut P) -> i32;

/// Translation of `struct iff_info`.
struct IffInfo<P> {
    id: [u8; 4],
    loader: IffLoader<P>,
}

/// Translation of `struct iff_data` (`iff_handle`): the registered
/// loaders, in order.
pub(crate) struct IffHandle<P> {
    infos: Vec<IffInfo<P>>,
    id_size: u32,
    flags: u32,
}

impl<P> IffHandle<P> {
    /// Translation of `iff_process()`.
    fn iff_process(
        &self,
        m: &mut ModuleData,
        id: &[u8; 17],
        size: u32,
        f: &mut HioHandle<'_>,
        parm: &mut P,
    ) -> i32 {
        let pos = hio_tell(f);

        let n = self.id_size as usize;
        for i in &self.infos {
            if id[..n] == i.id[..n] {
                if size > IFF_MAX_CHUNK_SIZE {
                    return -1;
                }
                if (i.loader)(m, size, f, parm) < 0 {
                    return -1;
                }
                break;
            }
        }

        if hio_seek(f, pos + size as i64, SEEK_SET) < 0 {
            /* IFF container issue--exit without error. */
            return 1;
        }

        0
    }

    /// Translation of `iff_chunk()`.
    fn iff_chunk(&self, m: &mut ModuleData, f: &mut HioHandle<'_>, parm: &mut P) -> i32 {
        let mut id = [0u8; 17];
        let n = self.id_size as usize;

        if hio_read(&mut id, 1, n, f) != n {
            /* End of file or IFF container issue--exit without error. */
            return 1;
        }

        if self.flags & IFF_SKIP_EMBEDDED != 0 {
            /* embedded RIFF hack */
            if &id[..4] == b"RIFF" {
                hio_read32b(f);
                hio_read32b(f);
                /* read first chunk ID instead */
                if hio_read(&mut id, 1, n, f) != n {
                    return 1;
                }
            }
        }

        let mut size = if self.flags & IFF_LITTLE_ENDIAN != 0 {
            hio_read32l(f)
        } else {
            hio_read32b(f)
        };

        if hio_error(f) != 0 {
            /* IFF container issue--exit without error. */
            return 1;
        }

        if self.flags & IFF_CHUNK_ALIGN2 != 0 {
            /* Sanity check */
            if size > 0xfffffffe {
                /* IFF container issue--exit without error. */
                return 1;
            }
            size = (size + 1) & !1;
        }

        if self.flags & IFF_CHUNK_ALIGN4 != 0 {
            /* Sanity check */
            if size > 0xfffffffc {
                /* IFF container issue--exit without error. */
                return 1;
            }
            size = (size + 3) & !3;
        }

        /* PT 3.6 hack: this does not seem to ever apply to "PTDT".
         * This broke several modules (city lights.pt36, acid phase.pt36) */
        if self.flags & IFF_FULL_CHUNK_SIZE != 0 && &id[..4] != b"PTDT" {
            if size < self.id_size + 4 {
                return -1;
            }
            size -= self.id_size + 4;
        }

        self.iff_process(m, &id, size, f, parm)
    }
}

/// Translation of `libxmp_iff_new()`.
pub(crate) fn libxmp_iff_new<P>() -> IffHandle<P> {
    IffHandle {
        infos: Vec::new(),
        id_size: 4,
        flags: 0,
    }
}

/// Translation of `libxmp_iff_load()`.
pub(crate) fn libxmp_iff_load<P>(
    data: &IffHandle<P>,
    m: &mut ModuleData,
    f: &mut HioHandle<'_>,
    parm: &mut P,
) -> i32 {
    while hio_eof(f) == 0 {
        let ret = data.iff_chunk(m, f, parm);
        if ret > 0 {
            break;
        }
        if ret < 0 {
            return -1;
        }
    }

    /* Reached end of file, or there was an IFF structural issue.
     * Either way, clear the error flag to allow loading to continue. */
    let _ = hio_error(f);
    0
}

/// Translation of `libxmp_iff_register()`.
pub(crate) fn libxmp_iff_register<P>(
    data: &mut IffHandle<P>,
    id: &[u8],
    loader: IffLoader<P>,
) -> i32 {
    /* Note: previously was an strncpy */
    let mut f = IffInfo { id: [0; 4], loader };
    for (i, &c) in id.iter().take(4).enumerate() {
        if c == 0 {
            break;
        }
        f.id[i] = c;
    }

    data.infos.push(f);
    0
}

/// Translation of `libxmp_iff_release()`.
pub(crate) fn libxmp_iff_release<P>(data: IffHandle<P>) {
    drop(data);
}

/* Functions to tune IFF mutations */

/// Translation of `libxmp_iff_id_size()`.
pub(crate) fn libxmp_iff_id_size<P>(data: &mut IffHandle<P>, n: i32) {
    data.id_size = n as u32;
}

/// Translation of `libxmp_iff_set_quirk()`.
pub(crate) fn libxmp_iff_set_quirk<P>(data: &mut IffHandle<P>, i: u32) {
    data.flags |= i;
}
