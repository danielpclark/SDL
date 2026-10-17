// Rust translation of src/loaders/hmn_load.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2026 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

/*
 * From http://www.livet.se/mahoney/:
 *
 * Most modules from His Master's Noise uses special chip-sounds or
 * fine-tuning of samples that never was a part of the standard NoiseTracker
 * v2.0 command set. So if you want to listen to them correctly use an Amiga
 * emulator and run the demo! DeliPlayer does a good job of playing them
 * (there are some occasional error mostly concerning vibrato and portamento
 * effects, but I can live with that!), and it can be downloaded from
 * http://www.deliplayer.com
 */

/*
 * From http://www.cactus.jawnet.pl/attitude/index.php?action=readtext&issue=12&which=12
 *
 * [Bepp] For your final Amiga release, the music disk His Master's Noise,
 * you developed a special version of NoiseTracker. Could you tell us a
 * little about this project?
 *
 * [Mahoney] I wanted to make a music disk with loads of songs, without being
 * too repetitive or boring. So all of my "experimental features" that did not
 * belong to NoiseTracker v2.0 were put into a separate version that would
 * feature wavetable sounds, chord calculations, off-line filter calculations,
 * mixing, reversing, sample accurate delays, resampling, fades - calculations
 * that would be done on a standard setup of sounds instead of on individual
 * modules. This "compression technique" lead to some 100 songs fitting on two
 * standard 3.5" disks, written by 22 different composers. I'd say that writing
 * a music program does give you loads of talented friends - you should try
 * that yourself someday!
 */

/*
 * From: Pex Tufvesson
 * To: Claudio Matsuoka
 * Date: Sat, Jun 1, 2013 at 4:16 AM
 * Subject: Re: A question about (very) old stuff
 *
 * (...)
 * If I remember correctly, these chip sounds were done with several short
 * waveforms, and an index table that was loopable that would choose which
 * waveform to play each frame. And, you didn't have to "draw" every
 * waveform in the instrument - you would choose which waveforms to draw
 * and the replayer would (at startup) interpolate the waveforms that you
 * didn't draw.
 *
 * In the special noisetracker, you could draw all of these waveforms, draw
 * the index table, and the instrument would be stored in one of the
 * "patterns" of the song.
 */

//! The His Master's Noise loader.

use super::super::common::*;
use super::super::effects::*;
use super::super::extras::InstrumentExtras;
use super::super::hio::*;
use super::super::hmn_extras::*;
use super::super::*;
use super::common::*;
use super::mod_load::{ModHeader, ModInstrument};
use super::sample::libxmp_load_sample;
use super::*;

pub(crate) const LIBXMP_LOADER_HMN: FormatLoader = FormatLoader {
    name: "His Master's Noise",
    test: hmn_test,
    loader: hmn_load,
};

/* His Master's Noise M&K! will fail in regular Noisetracker loading
 * due to invalid finetune values.
 */
const MAGIC_FEST: u32 = magic4(b'F', b'E', b'S', b'T');
const MAGIC_MK: u32 = magic4(b'M', b'&', b'K', b'!');

/// Translation of `hmn_test()`.
fn hmn_test(f: &mut HioHandle<'_>, t: Option<&mut [u8]>, start: i32) -> i32 {
    hio_seek(f, start as i64 + 1080, SEEK_SET);
    let magic = hio_read32b(f);

    if magic != MAGIC_FEST && magic != MAGIC_MK {
        return -1;
    }

    hio_seek(f, start as i64, SEEK_SET);
    libxmp_read_title(f, t, 20);

    0
}

/// Translation of `struct mupp`.
#[derive(Clone, Copy, Default)]
struct Mupp {
    prgon: u8,
    pattno: u8,
    dataloopstart: u8,
    dataloopend: u8,
}

/// Translation of `hmn_load()`.
fn hmn_load(m: &mut ModuleData, f: &mut HioHandle<'_>, start: i32) -> i32 {
    let mut mh = ModHeader {
        name: [0; 20],
        ins: [ModInstrument::default(); 31],
        len: 0,
        restart: 0,
        order: [0; 128],
        magic: [0; 4],
    };
    let mut mupp = [Mupp::default(); 31];
    let mut mod_event = [0u8; 4];

    /*
     *    clr.b   $1c(a6) ;prog on/off
     *    CMP.L   #'Mupp',-$16(a3,d4.l)
     *    bne.s   noprgo
     *    move.l  a0,-(a7)
     *    move.b  #1,$1c(a6)      ;prog on
     *    move.l  l697,a0
     *    lea     $43c(a0),a0
     *    moveq   #0,d2
     *    move.b  -$16+$4(a3,d4.l),d2     ;pattno
     *    mulu    #$400,d2
     *    lea     (a0,d2.l),a0
     *    move.l  a0,4(a6)        ;proginstr data-start
     *    moveq   #0,d2
     *    MOVE.B  $3C0(A0),$12(A6)
     *    AND.B   #$7F,$12(A6)
     *    move.b  $380(a0),d2
     *    mulu    #$20,d2
     *    lea     (a0,d2.w),a0
     *    move.l  a0,$a(a6)       ;loopstartmempoi = startmempoi
     *    move.B  $3(a3,d4.l),$13(a6)     ;volume
     *    move.b  -$16+$5(a3,d4.l),8(a6)  ;dataloopstart
     *    move.b  -$16+$6(a3,d4.l),9(a6)  ;dataloopend
     *    move.w  #$10,$e(a6)     ;looplen
     *    move.l  (a7)+,a0
     *    MOVE.W  $12(A6),(A2)
     *    AND.W   #$FF,(A2)
     *    BRA.S   L505_LQ
     */

    /*
     * Wavetable structure is 22 * 32 byte waveforms and 32 byte
     * wave control data with looping.
     */

    hio_read(&mut mh.name, 20, 1, f);
    let mut num_mupp = 0;

    for i in 0..31 {
        hio_read(&mut mh.ins[i].name, 22, 1, f); /* Instrument name */
        if &mh.ins[i].name[..4] == b"Mupp" {
            mupp[i].prgon = 1;
            mupp[i].pattno = mh.ins[i].name[4];
            mupp[i].dataloopstart = mh.ins[i].name[5];
            mupp[i].dataloopend = mh.ins[i].name[6];
            num_mupp += 1;
        }

        mh.ins[i].size = hio_read16b(f);
        mh.ins[i].finetune = hio_read8(f) as i8;
        mh.ins[i].volume = hio_read8(f) as i8;
        mh.ins[i].loop_start = hio_read16b(f);
        mh.ins[i].loop_size = hio_read16b(f);
    }
    mh.len = hio_read8(f);
    mh.restart = hio_read8(f);
    hio_read(&mut mh.order, 128, 1, f);
    hio_read(&mut mh.magic, 4, 1, f);

    let mod_ = &mut m.mod_;
    mod_.chn = 4;
    mod_.ins = 31;
    mod_.smp = mod_.ins + 28 * num_mupp;
    mod_.len = mh.len as i32;
    mod_.rst = mh.restart as i32;
    mod_.xxo[..128].copy_from_slice(&mh.order);

    for i in 0..128 {
        if mod_.xxo[i] as i32 > mod_.pat {
            mod_.pat = mod_.xxo[i] as i32;
        }
    }

    mod_.pat += 1;
    mod_.trk = mod_.chn * mod_.pat;

    if libxmp_hmn_new_module_extras(m) != 0 {
        return -1;
    }

    strncpy(&mut m.mod_.name, &mh.name, 20);
    let mut type_ = b"His Master's Noise (".to_vec();
    type_.extend_from_slice(&printf_s(&mh.magic, 4, 4));
    type_.push(b')');
    libxmp_set_type(m, &type_);

    if libxmp_init_instrument(m) < 0 {
        return -1;
    }

    let mod_ = &mut m.mod_;
    for i in 0..mod_.ins {
        let iu = i as usize;
        if mupp[iu].prgon != 0 {
            mod_.xxi[iu].nsm = 28;
            let name = format!(
                "Mupp {:02x} {:02x} {:02x}",
                mupp[iu].pattno, mupp[iu].dataloopstart, mupp[iu].dataloopend
            );
            snprintf_bytes(&mut mod_.xxi[iu].name, 32, name.as_bytes());
            if libxmp_hmn_new_instrument_extras(&mut mod_.xxi[iu]) != 0 {
                return -1;
            }
        } else {
            mod_.xxi[iu].nsm = 1;
            libxmp_instrument_name(mod_, i, &mh.ins[iu].name, 22);

            let xxs = &mut mod_.xxs[iu];
            xxs.len = 2 * mh.ins[iu].size as i32;
            xxs.lps = 2 * mh.ins[iu].loop_start as i32;
            xxs.lpe = xxs.lps + 2 * mh.ins[iu].loop_size as i32;
            xxs.flg = if mh.ins[iu].loop_size > 1 {
                XMP_SAMPLE_LOOP
            } else {
                0
            };
        }

        let nsm = mod_.xxi[iu].nsm;
        if libxmp_alloc_subinstrument(mod_, i, nsm) < 0 {
            return -1;
        }

        for j in 0..nsm as usize {
            let sub = &mut mod_.xxi[iu].sub[j];
            sub.fin = -((((mh.ins[iu].finetune as u8 as i32) << 3) as u8 as i8) as i32);
            sub.vol = mh.ins[iu].volume as i32;
            sub.pan = XMP_INST_NO_DEFAULT_PAN;
            sub.sid = i;
        }
    }

    if libxmp_init_pattern(mod_) < 0 {
        return -1;
    }

    /* Load and convert patterns */

    for i in 0..mod_.pat {
        if libxmp_alloc_pattern_tracks(mod_, i, 64) < 0 {
            return -1;
        }

        for j in 0..(64 * 4) {
            let event = mod_.event_mut(i, j % 4, j / 4);
            if hio_read(&mut mod_event, 1, 4, f) < 4 {
                return -1;
            }
            libxmp_decode_protracker_event(event, &mod_event);

            match event.fxt {
                0x07 => {
                    event.fxt = FX_MEGAARP;
                }
                0x08 | 0x09 | 0x0e => {
                    event.fxt = 0;
                    event.fxp = 0;
                }
                _ => {}
            }
        }
    }

    /* Noisetracker does not support CIA timing (Glue Master/muppenkorva.mod) */
    m.quirk |= QUIRK_NOBPM;
    m.period_type = PERIOD_MODRNG;

    /* Load samples */

    for i in 0..31 {
        let smpctl = m.smpctl;
        if libxmp_load_sample(
            smpctl,
            Some(f),
            SAMPLE_FLAG_FULLREP,
            &mut m.mod_.xxs[i],
            None,
        ) < 0
        {
            return -1;
        }
    }

    /* Load Mupp samples */

    let mut mupp_index = 0;
    for i in 0..31 {
        if mupp[i].prgon == 0 {
            continue;
        }

        hio_seek(
            f,
            start as i64 + 1084 + 1024 * mupp[i].pattno as i64,
            SEEK_SET,
        );
        for j in 0..28 {
            let k = 31 + 28 * mupp_index + j;
            m.mod_.xxi[i].sub[j].sid = k as i32;
            let xxs = &mut m.mod_.xxs[k];
            xxs.len = 32;
            xxs.lps = 0;
            xxs.lpe = 32;
            xxs.flg = XMP_SAMPLE_LOOP;
            let smpctl = m.smpctl;
            if libxmp_load_sample(smpctl, Some(f), 0, &mut m.mod_.xxs[k], None) < 0 {
                return -1;
            }
        }

        let InstrumentExtras::Hmn(extra) = &mut m.mod_.xxi[i].extra else {
            unreachable!();
        };
        extra.dataloopstart = mupp[i].dataloopstart as i32;
        extra.dataloopend = mupp[i].dataloopend as i32;

        hio_read(&mut extra.data, 1, 64, f);
        hio_read(&mut extra.progvolume, 1, 64, f);

        mupp_index += 1;
    }

    0
}
