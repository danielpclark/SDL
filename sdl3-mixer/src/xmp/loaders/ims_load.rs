// Rust translation of src/loaders/ims_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* Loader for Images Music System modules based on the EP replayer.
 *
 * Date: Thu, 19 Apr 2001 19:13:06 +0200
 * From: Michael Doering <mldoering@gmx.net>
 *
 * I just "stumbled" upon something about the Unic.3C format when I was
 * testing replayers for the upcoming UADE 0.21 that might be also
 * interesting to you for xmp. The "Beastbusters" tune is not a UNIC file :)
 * It's actually a different Format, although obviously related, called
 * "Images Music System".
 *
 * I was testing the replayer from the Wanted Team with one of their test
 * tunes, among them also the beastbuster music. When I first listened to
 * it, I knew I have heard it somewhere, a bit different but it was alike.
 * This one had more/richer percussions and there was no strange beep in
 * the bg. ;) After some searching on my HD I found it among the xmp test
 * tunes as a UNIC file.
 */

//! The Images Music System loader.

use super::super::common::*;
use super::super::hio::*;
use super::super::*;
use super::common::*;
use super::sample::libxmp_load_sample;
use super::*;

/// Translation of `struct ims_instrument`.
#[derive(Clone, Copy, Default)]
struct ImsInstrument {
    name: [u8; 22],
    size: u16,
    finetune: u8, /* Presumably finetune; unsupported */
    volume: u8,
    loop_start: u16,
    loop_size: u16,
}

/// Translation of `struct ims_header`.
struct ImsHeader {
    title: [u8; 20],
    ins: [ImsInstrument; 31],
    len: u8,
    restart: u8, /* Only non-zero in shadow_dancer-am5-3ch.ims */
    orders: [u8; 128],
    sample_offset: u32,
}

impl ImsHeader {
    fn new() -> Self {
        ImsHeader {
            title: [0; 20],
            ins: [ImsInstrument::default(); 31],
            len: 0,
            restart: 0,
            orders: [0; 128],
            sample_offset: 0,
        }
    }
}

pub(crate) const LIBXMP_LOADER_IMS: FormatLoader = FormatLoader {
    name: "Images Music System",
    test: ims_test,
    loader: ims_load,
};

/// Translation of `ims_test()`.
fn ims_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    let mut ih = ImsHeader::new();

    let mut smp_size = 0;

    hio_read(&mut ih.title, 20, 1, f);

    for i in 0..31 {
        if hio_read(&mut ih.ins[i].name, 1, 22, f) < 22 {
            return -1;
        }

        ih.ins[i].size = hio_read16b(f);
        ih.ins[i].finetune = hio_read8(f);
        ih.ins[i].volume = hio_read8(f);
        ih.ins[i].loop_start = hio_read16b(f);
        ih.ins[i].loop_size = hio_read16b(f);

        smp_size += ih.ins[i].size as i32 * 2;

        if libxmp_test_name(&ih.ins[i].name, 22, 0) < 0 {
            return -1;
        }

        /* "Note that the lack of finetuned periods in this table would also
         * most likely confirm that there is indeed no support for finetune in
         * this format." -Saga Musix */
        if ih.ins[i].finetune != 0 {
            return -1;
        }

        if ih.ins[i].volume > 0x40 {
            return -1;
        }

        if ih.ins[i].size > 0x8000 {
            return -1;
        }

        if ih.ins[i].loop_start > ih.ins[i].size {
            return -1;
        }

        if ih.ins[i].size != 0 && ih.ins[i].loop_size as i32 > 2 * ih.ins[i].size as i32 {
            return -1;
        }
    }

    if smp_size < 8 {
        return -1;
    }

    ih.len = hio_read8(f);
    ih.restart = hio_read8(f);
    if hio_read(&mut ih.orders, 1, 128, f) < 128 {
        return -1;
    }
    ih.sample_offset = hio_read32b(f);

    if ih.restart >= ih.len {
        return -1;
    }

    if ih.len > 0x7f {
        return -1;
    }

    /* Modules may contain patterns after the end of the module,
     * same as regular MOD. (The Judge/redoctober-sub-docking.ims) */
    let mut pat = 0;
    for i in 0..128 {
        if ih.orders[i] as i32 > pat {
            pat = ih.orders[i] as i32;
        }
    }
    pat += 1;

    if pat > 0x7f || ih.len == 0 || ih.len > 0x7f {
        return -1;
    }

    /* Sample offset should always be the first byte after the pattern data.
     * Each pattern is 768 bytes and the header is 1084 bytes. */
    if (pat as u32) * 0x300 + 0x43c != ih.sample_offset {
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 20);

    0
}

/// Translation of `ims_load()`.
fn ims_load(m: &mut ModuleData, f: &mut HioHandle<'_>, _start: i32) -> i32 {
    let mut ih = ImsHeader::new();
    let mut ims_event = [0u8; 3];
    let xpo = 21; /* Tuned against UADE */

    let mod_ = &mut m.mod_;
    mod_.chn = 4;
    mod_.ins = 31;
    mod_.smp = mod_.ins;

    hio_read(&mut ih.title, 20, 1, f);

    for i in 0..31 {
        hio_read(&mut ih.ins[i].name, 22, 1, f);
        ih.ins[i].size = hio_read16b(f);
        ih.ins[i].finetune = hio_read8(f);
        ih.ins[i].volume = hio_read8(f);
        ih.ins[i].loop_start = hio_read16b(f);
        ih.ins[i].loop_size = hio_read16b(f);
    }

    ih.len = hio_read8(f);
    if ih.len > 128 {
        return -1;
    }
    ih.restart = hio_read8(f);
    hio_read(&mut ih.orders, 128, 1, f);
    ih.sample_offset = hio_read32b(f);

    mod_.rst = ih.restart as i32;
    mod_.len = ih.len as i32;
    mod_.xxo[..128].copy_from_slice(&ih.orders);

    for i in 0..128 {
        if mod_.xxo[i] as i32 > mod_.pat {
            mod_.pat = mod_.xxo[i] as i32;
        }
    }

    mod_.pat += 1;
    mod_.trk = mod_.chn * mod_.pat;

    strncpy(&mut mod_.name, &ih.title, 20);
    libxmp_set_type(m, b"Images Music System");

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;

        if libxmp_alloc_subinstrument(mod_, i, 1) < 0 {
            return -1;
        }

        let xxi = &mut mod_.xxi[iu];
        let sub = &mut xxi.sub[0];
        let xxs = &mut mod_.xxs[iu];

        xxs.len = 2 * ih.ins[iu].size as i32;
        xxs.lps = 2 * ih.ins[iu].loop_start as i32;
        xxs.lpe = xxs.lps + 2 * ih.ins[iu].loop_size as i32;
        xxs.flg = if ih.ins[iu].loop_size > 1 {
            XMP_SAMPLE_LOOP
        } else {
            0
        };
        sub.fin = (ih.ins[iu].finetune as i8 as i32) << 4;
        sub.vol = ih.ins[iu].volume as i32;
        sub.pan = XMP_INST_NO_DEFAULT_PAN;
        sub.sid = i;
        //mod->xxi[i].rls = 0xfff;

        if xxs.len > 0 {
            xxi.nsm = 1;
        }

        libxmp_instrument_name(mod_, i, &ih.ins[iu].name, 22);
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Load and convert patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..0x100 {
            let event = mod_.event_mut(i, j & 0x3, j >> 2);
            hio_read(&mut ims_event, 1, 3, f);

            /* Event format:
             *
             * 0000 0000  0000 0000  0000 0000
             *  |\     /  \  / \  /  \       /
             *  | note    ins   fx   parameter
             * ins
             *
             * 0x3f is a blank note.
             * 0x00 is "a valid period two octaves below middle-C" -Saga Musix
             */
            event.note = ims_event[0] & 0x3f;
            if event.note != 0x3f {
                event.note += xpo + 12;
            } else {
                event.note = 0;
            }
            event.ins = (((ims_event[0] & 0x40) >> 2) as i32 | msn(ims_event[1])) as u8;
            event.fxt = lsn(ims_event[1]) as u8;
            event.fxp = ims_event[2];

            libxmp_disable_continue_fx(event);

            /* According to Asle:
             * ``Just note that pattern break effect command (D**) uses
             * HEX value in UNIC format (while it is DEC values in PTK).
             * Thus, it has to be converted!''
             *
             * Is this valid for IMS as well? --claudio
             */
            if event.fxt == 0x0d {
                event.fxp = ((event.fxp / 10) << 4) | (event.fxp % 10);
            }
        }
    }

    m.period_type = PERIOD_MODRNG;

    /* Load samples */

    for i in 0..m.mod_.smp as usize {
        if m.mod_.xxs[i].len == 0 {
            continue;
        }
        let smpctl = m.smpctl;
        if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[i], None) < 0 {
            return -1;
        }
    }

    0
}
