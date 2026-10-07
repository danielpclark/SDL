/*

    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    sndfont.c: SoundFont file extension
    written by Takashi Iwai <iwai@dragon.mm.t.u-tokyo.ac.jp>
    Copyright (C) 1996,1997 Takashi Iwai

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.
*/
// Modified 2026-10-07: translated into Rust from sndfont.c, sndfont.h and sflayer.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! SoundFont instruments. Translation of `sndfont.c` (and `sndfont.h`
//! and `sflayer.h`).
//!
//! Upstream indexes its arrays (and the songs' banks) with the file's
//! numbers unchecked; here numbers out of range are skipped (the
//! `FIXME (upstream)` notes).

#![allow(dead_code)]

use std::sync::Arc;

use sdl3::io::{IoStream, IoWhence};
use sdl3::stdlib::math::{log10, pow};

use crate::common::{c_f64_to_i32, timi_openfile};
use crate::instrum::{MODES_16BIT, MODES_ENVELOPE, MODES_LOOPING, MODES_SUSTAIN};
use crate::options::{FRACTION_BITS, RATE_SHIFT, VIBRATO_RATE_TUNING};
use crate::readsbk::{free_sbk, load_sbk, SFInfo, Tgenrec};
use crate::resample::pre_resample;
use crate::tables::{BEND_FINE, FREQ_TABLE};
use crate::timidity::{tones, Globals};
use crate::{Instrument, MidiSong, Sample, ToneBank, ToneSrc, INST_SF2, VIBRATO_SAMPLE_INCREMENTS};

/*----------------------------------------------------------------
 * sflayer.h
 *----------------------------------------------------------------*/

const SF_START_ADDRS: usize = 0; /* sample start address -4 (0 to * 0xffffff) */
const SF_END_ADDRS: usize = 1;
const SF_STARTLOOP_ADDRS: usize = 2; /* loop start address -4 (0 to * 0xffffff) */
const SF_ENDLOOP_ADDRS: usize = 3; /* loop end address -3 (0 to * 0xffffff) */
const SF_START_ADDRS_HI: usize = 4; /* high word of startAddrs */
const SF_LFO1_TO_PITCH: usize = 5; /* main fm: lfo1-> pitch */
const SF_LFO2_TO_PITCH: usize = 6; /* aux fm:  lfo2-> pitch */
const SF_ENV1_TO_PITCH: usize = 7; /* pitch env: env1(aux)-> pitch */
const SF_INITIAL_FILTER_FC: usize = 8; /* initial filter cutoff */
const SF_INITIAL_FILTER_Q: usize = 9; /* filter Q */
const SF_LFO1_TO_FILTER_FC: usize = 10; /* filter modulation: lfo1 -> filter * cutoff */
const SF_ENV1_TO_FILTER_FC: usize = 11; /* filter env: env1(aux)-> filter * cutoff */
const SF_END_ADDRS_HI: usize = 12; /* high word of endAddrs */
const SF_LFO1_TO_VOLUME: usize = 13; /* tremolo: lfo1-> volume */
const SF_ENV2_TO_VOLUME: usize = 14; /* Env2Depth: env2-> volume */
const SF_CHORUS_EFFECTS_SEND: usize = 15; /* chorus */
const SF_REVERB_EFFECTS_SEND: usize = 16; /* reverb */
const SF_PAN_EFFECTS_SEND: usize = 17; /* pan */
const SF_AUX_EFFECTS_SEND: usize = 18; /* pan auxdata (internal) */
const SF_SAMPLE_VOLUME: usize = 19; /* used internally */
const SF_UNUSED3: usize = 20;
const SF_DELAY_LFO1: usize = 21; /* delay 0x8000-n*(725us) */
const SF_FREQ_LFO1: usize = 22; /* frequency */
const SF_DELAY_LFO2: usize = 23; /* delay 0x8000-n*(725us) */
const SF_FREQ_LFO2: usize = 24; /* frequency */
const SF_DELAY_ENV1: usize = 25; /* delay 0x8000 - n(725us) */
const SF_ATTACK_ENV1: usize = 26; /* attack */
const SF_HOLD_ENV1: usize = 27; /* hold */
const SF_DECAY_ENV1: usize = 28; /* decay */
const SF_SUSTAIN_ENV1: usize = 29; /* sustain */
const SF_RELEASE_ENV1: usize = 30; /* release */
const SF_AUTO_HOLD_ENV1: usize = 31;
const SF_AUTO_DECAY_ENV1: usize = 32;
const SF_DELAY_ENV2: usize = 33; /* delay 0x8000 - n(725us) */
const SF_ATTACK_ENV2: usize = 34; /* attack */
const SF_HOLD_ENV2: usize = 35; /* hold */
const SF_DECAY_ENV2: usize = 36; /* decay */
const SF_SUSTAIN_ENV2: usize = 37; /* sustain */
const SF_RELEASE_ENV2: usize = 38; /* release */
const SF_AUTO_HOLD_ENV2: usize = 39;
const SF_AUTO_DECAY_ENV2: usize = 40;
const SF_INSTRUMENT: usize = 41; /* */
const SF_NOP: usize = 42;
const SF_KEY_RANGE: usize = 43; /* */
const SF_VEL_RANGE: usize = 44; /* */
const SF_STARTLOOP_ADDRS_HI: usize = 45; /* high word of startloopAddrs */
const SF_KEYNUM: usize = 46; /* */
const SF_VELOCITY: usize = 47; /* */
const SF_INST_VOL: usize = 48; /* */
const SF_KEY_TUNING: usize = 49;
const SF_ENDLOOP_ADDRS_HI: usize = 50; /* high word of endloopAddrs */
const SF_COARSE_TUNE: usize = 51;
const SF_FINE_TUNE: usize = 52;
const SF_SAMPLE_ID: usize = 53;
const SF_SAMPLE_FLAGS: usize = 54;
const SF_SAMPLE_PITCH: usize = 55; /* SF1 only */
const SF_SCALE_TUNING: usize = 56;
const SF_KEY_EXCLUSIVE_CLASS: usize = 57;
const SF_ROOT_KEY: usize = 58;
const SF_EOF: usize = 59;

const SFPARM_SIZE: usize = SF_EOF;

/*----------------------------------------------------------------
 * compile flags
 *----------------------------------------------------------------*/

/*#define SF_SUPPRESS_ENVELOPE*/
const SF_SUPPRESS_ENVELOPE: bool = false;
/*#define SF_SUPPRESS_TREMOLO*/
const SF_SUPPRESS_TREMOLO: bool = false;
/*#define SF_SUPPRESS_VIBRATO*/
const SF_SUPPRESS_VIBRATO: bool = false;
const SF_SUPPRESS_CUTOFF: bool = true;

/*----------------------------------------------------------------
 * local parameters
 *----------------------------------------------------------------*/

/// Translation of `Layer`.
#[derive(Clone, Copy)]
struct Layer {
    val: [i16; SFPARM_SIZE],
    set: [i8; SFPARM_SIZE],
}

impl Layer {
    /// `SDL_memset(&lay, 0, sizeof(Layer))`.
    fn zeroed() -> Layer {
        Layer {
            val: [0; SFPARM_SIZE],
            set: [0; SFPARM_SIZE],
        }
    }
}

/// Translation of `SampleList` (the `next` pointer is the order in
/// [`InstList::slist`]).
#[derive(Clone, Default)]
struct SampleList {
    v: Sample,
    startsample: i32,
    endsample: i32,
    cutoff_freq: i32,
    resonance: f32,
}

/// Translation of `InstList`.
struct InstList {
    bank: i32,
    preset: i32,
    keynote: i32,
    samples: i32,
    order: i32,
    /// The samples, the newest last (upstream's list has it first).
    slist: Vec<SampleList>,
}

/// Translation of `SFInsts`.
pub(crate) struct SFInsts {
    fname: Option<Vec<u8>>,
    io: Option<IoStream<'static>>,
    version: u16,
    minorversion: u16,
    samplepos: i32,
    samplesize: i32,
    /// The instruments, the newest last (upstream's list has it first).
    instlist: Vec<InstList>,
}

/// Translation of `SFExclude`.
struct SFExclude {
    bank: i32,
    preset: i32,
    keynote: i32,
}

/// Translation of `SFOrder`.
struct SFOrder {
    bank: i32,
    preset: i32,
    keynote: i32,
    order: i32,
}

/*----------------------------------------------------------------*/

/// `sndfont.c`'s `static` variables: `sfrec`, `sfinfo`, `sfexclude`
/// and `sforder` (the lists with the newest last).
pub(crate) struct SfState {
    sfrec: SFInsts,
    sfinfo: SFInfo,
    sfexclude: Vec<SFExclude>,
    sforder: Vec<SFOrder>,
}

impl SfState {
    pub const fn new() -> SfState {
        SfState {
            sfrec: SFInsts {
                fname: None,
                io: None,
                version: 0,
                minorversion: 0,
                samplepos: 0,
                samplesize: 0,
                instlist: Vec::new(),
            },
            sfinfo: SFInfo::new(),
            sfexclude: Vec::new(),
            sforder: Vec::new(),
        }
    }
}

const CUTOFF_ALLOWED: i32 = 0;

/// Translation of `init_sbk()`.
pub(crate) fn init_sbk(g: &mut Globals, fname: &[u8]) -> i32 {
    crate::snddbg!("init soundfonts `{}'\n", String::from_utf8_lossy(fname));

    let sf = &mut g.sf;
    sf.sfinfo = SFInfo::new();

    sf.sfrec.io = timi_openfile(fname);
    let Some(io) = sf.sfrec.io.as_mut() else {
        crate::snddbg!(
            "can't open soundfont file {}\n",
            String::from_utf8_lossy(fname)
        );
        return -1;
    };

    sf.sfrec.fname = Some(fname.to_vec());

    if load_sbk(io, &mut sf.sfinfo) < 0 {
        crate::snddbg!("{}: bad soundfont file\n", String::from_utf8_lossy(fname));
        end_sbk(g);
        return -1;
    }

    0
}

/// Translation of `end_sbk()`.
pub(crate) fn end_sbk(g: &mut Globals) {
    let sf = &mut g.sf;
    sf.sfrec.io = None;
    sf.sfrec.fname = None;
    free_sbk(&mut sf.sfinfo);
}

/// Translation of `init_soundfont()`.
pub(crate) fn init_soundfont(song: &mut MidiSong, g: &mut Globals, order: i32) -> i32 {
    for i in 0..g.sf.sfinfo.nrpresets - 1 {
        let Some(hdr) = g.sf.sfinfo.presethdr.get(i as usize) else {
            break;
        };
        let bank = hdr.bank as i32;
        let preset = hdr.preset as i32;
        if is_excluded(&g.sf, bank, preset, -1) {
            continue;
        }
        if bank == 128 {
            // FIXME (upstream): a drum set number over 127 indexes past
            // song->drumset; skip the preset.
            let Some(slot) = song.drumset.get_mut(preset as usize) else {
                continue;
            };
            if slot.is_none() {
                *slot = Some(ToneBank::new(ToneSrc::Own(vec![Default::default(); 128])));
            }
        } else {
            // FIXME (upstream): likewise for a bank over 127.
            let Some(slot) = song.tonebank.get_mut(bank as usize) else {
                continue;
            };
            if slot.is_none() {
                *slot = Some(ToneBank::new(ToneSrc::Own(vec![Default::default(); 128])));
            }
        }
        parse_preset(song, g, i, order);
    }

    /* copy header info */
    let sf = &mut g.sf;
    sf.sfrec.version = sf.sfinfo.version;
    sf.sfrec.minorversion = sf.sfinfo.minorversion;
    sf.sfrec.samplepos = sf.sfinfo.samplepos;
    sf.sfrec.samplesize = sf.sfinfo.samplesize;

    0
}

// (free_sample() is InstList's Drop.)

/// Translation of `end_soundfont()`.
pub(crate) fn end_soundfont(g: &mut Globals) {
    g.sf.sfrec.instlist = Vec::new();

    free_exclude(&mut g.sf);
    free_order(&mut g.sf);
}

/*----------------------------------------------------------------
 * get converted instrument info and load the wave data from file
 *----------------------------------------------------------------*/

/// Translation of `load_soundfont()`.
pub(crate) fn load_soundfont(
    song: &mut MidiSong,
    g: &mut Globals,
    order: i32,
    bank: i32,
    preset: i32,
    keynote: i32,
) -> Option<Box<Instrument>> {
    let rec = &mut g.sf.sfrec;
    if rec.io.is_none() {
        crate::snddbg!("NULL soundfont file pointer\n");
        return None;
    }

    let ip = rec.instlist.iter().rposition(|ip| {
        ip.bank == bank
            && ip.preset == preset
            && (keynote < 0 || keynote == ip.keynote)
            && ip.order == order
    })?;
    if rec.instlist[ip].samples != 0 {
        return load_from_file(song, rec, ip);
    }
    None
}

fn load_from_file(song: &mut MidiSong, rec: &mut SFInsts, ipi: usize) -> Option<Box<Instrument>> {
    let ip = &rec.instlist[ipi];
    let io = rec.io.as_mut()?;

    crate::snddbg!(
        "Loading SF bank{} prg{} note{}\n",
        ip.bank,
        ip.preset,
        ip.keynote
    );

    let mut inst = Box::new(Instrument {
        type_: INST_SF2,
        samples: ip.samples,
        sample: Vec::new(),
    });
    if inst
        .sample
        .try_reserve_exact(ip.samples.max(0) as usize)
        .is_err()
    {
        song.oom = 1; /* nomem */
        return None;
    }
    for sp in ip.slist.iter().rev().take(ip.samples.max(0) as usize) {
        let mut sample = sp.v.clone();
        // SDL_malloc(sp->endsample + 6): a negative size_t fails.
        if sp.endsample.wrapping_add(6) < 0 {
            song.oom = 1; /* nomem */
            return None;
        }
        if sp.endsample < 0 {
            // FIXME (upstream): a length of -6 to -1 allocates 0 to 5
            // bytes, then reads the rest of the file into them (it can't
            // read -1 bytes, which fails the read).
            return None; /* badread */
        }
        let _ = io.seek(sp.startsample as i64, IoWhence::Set);
        // (read in pieces, so a length the file doesn't have fails the
        // read, as it would upstream, without first allocating it all.)
        let want = sp.endsample as usize;
        let mut bytes: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 65536];
        while bytes.len() < want {
            let n = (want - bytes.len()).min(chunk.len());
            let got = io.read(&mut chunk[..n]);
            if bytes.try_reserve(got).is_err() {
                song.oom = 1; /* nomem */
                return None;
            }
            bytes.extend_from_slice(&chunk[..got]);
            if got < n {
                return None; /* badread */
            }
        }
        /* initialize the 3 extra samples at the end (those +6 bytes) */
        let n = (want + 6) / 2;
        let mut data: Vec<i16> = Vec::new();
        if data.try_reserve_exact(n).is_err() {
            song.oom = 1; /* nomem */
            return None;
        }
        for k in 0..n {
            let lo = bytes.get(2 * k).copied().unwrap_or(0);
            let hi = bytes.get(2 * k + 1).copied().unwrap_or(0);
            data.push(i16::from_le_bytes([lo, hi]));
        }
        for k in want / 2..want / 2 + 3 {
            if let Some(s) = data.get_mut(k) {
                *s = 0;
            }
        }
        sample.data = data;

        /* do some filtering if necessary */
        if !SF_SUPPRESS_CUTOFF && sp.cutoff_freq > 0 && CUTOFF_ALLOWED != 0 {
            /* restore the normal value */
            sample.data_length >>= FRACTION_BITS;
            crate::snddbg!(
                "bank={}, preset={}, keynote={} / cutoff = {} / resonance = {}\n",
                ip.bank,
                ip.preset,
                ip.keynote,
                sp.cutoff_freq,
                sp.resonance
            );
            do_lowpass(&mut sample, sp.cutoff_freq, sp.resonance);
            /* convert again to the fractional value */
            sample.data_length <<= FRACTION_BITS;
        }

        /* resample it if possible */
        if sample.note_to_use != 0 && sample.modes & MODES_LOOPING == 0 {
            pre_resample(song, &mut sample);
        }
        inst.sample.push(Arc::new(sample));
    }
    Some(inst)
}

/*----------------------------------------------------------------
 * excluded samples
 *----------------------------------------------------------------*/

/// Translation of `exclude_soundfont()`.
pub(crate) fn exclude_soundfont(g: &mut Globals, bank: i32, preset: i32, keynote: i32) -> i32 {
    if g.sf.sfexclude.try_reserve(1).is_err() {
        return -1;
    }
    g.sf.sfexclude.push(SFExclude {
        bank,
        preset,
        keynote,
    });
    0
}

/* check the instrument is specified to be excluded */
fn is_excluded(sf: &SfState, bank: i32, preset: i32, keynote: i32) -> bool {
    for p in sf.sfexclude.iter().rev() {
        if p.bank == bank
            && (p.preset < 0 || p.preset == preset)
            && (p.keynote < 0 || p.keynote == keynote)
        {
            return true;
        }
    }
    false
}

/* free exclude list */
fn free_exclude(sf: &mut SfState) {
    sf.sfexclude = Vec::new();
}

/*----------------------------------------------------------------
 * ordered samples
 *----------------------------------------------------------------*/

/// Translation of `order_soundfont()`.
pub(crate) fn order_soundfont(
    g: &mut Globals,
    bank: i32,
    preset: i32,
    keynote: i32,
    order: i32,
) -> i32 {
    if g.sf.sforder.try_reserve(1).is_err() {
        return -1;
    }
    g.sf.sforder.push(SFOrder {
        bank,
        preset,
        keynote,
        order,
    });
    0
}

/* check the instrument is specified to be ordered */
fn is_ordered(sf: &SfState, bank: i32, preset: i32, keynote: i32) -> i32 {
    for p in sf.sforder.iter().rev() {
        if p.bank == bank
            && (p.preset < 0 || p.preset == preset)
            && (p.keynote < 0 || p.keynote == keynote)
        {
            return p.order;
        }
    }
    -1
}

/* free order list */
fn free_order(sf: &mut SfState) {
    sf.sforder = Vec::new();
}

/*----------------------------------------------------------------
 * parse a preset
 *----------------------------------------------------------------*/

fn parse_preset(song: &mut MidiSong, g: &mut Globals, preset: i32, order: i32) {
    let sf = &g.sf.sfinfo;
    // FIXME (upstream): bag indexes past the bags read past them.
    let (Some(from), Some(to)) = (
        sf.presethdr.get(preset as usize),
        sf.presethdr.get(preset as usize + 1),
    ) else {
        return;
    };
    let from_ndx = from.bag_ndx as i32;
    let to_ndx = to.bag_ndx as i32;

    let mut glay = Layer::zeroed();
    for i in from_ndx..to_ndx {
        let mut lay = Layer::zeroed();
        parse_preset_layer(&mut lay, &g.sf.sfinfo, i);
        let inst = search_inst(&lay);
        if inst < 0 {
            /* global layer */
            glay = lay;
        } else {
            append_layer(&mut lay, &glay, &g.sf.sfinfo);
            parse_inst(song, g, &lay, preset, inst, order);
        }
    }
}

/* map a generator operation to the layer structure */
fn parse_gen(lay: &mut Layer, gen: &Tgenrec) {
    // FIXME (upstream): an operator past the ones known writes past the
    // layer; skip it.
    let Ok(oper) = usize::try_from(gen.oper) else {
        return;
    };
    if oper >= SFPARM_SIZE {
        return;
    }
    lay.set[oper] = 1;
    lay.val[oper] = gen.amount;
}

/// `sf->bag[idx]` to `sf->bag[idx+1]`, the generators of a bag.
fn bag_range(bag: &[u16], idx: i32) -> std::ops::Range<usize> {
    match (bag.get(idx as usize), bag.get(idx as usize + 1)) {
        (Some(&from), Some(&to)) => from as usize..to as usize,
        _ => 0..0,
    }
}

/* parse preset generator layers */
fn parse_preset_layer(lay: &mut Layer, sf: &SFInfo, idx: i32) {
    for i in bag_range(&sf.presetbag, idx) {
        let Some(gen) = sf.presetgen.get(i) else {
            break;
        };
        parse_gen(lay, gen);
    }
}

/* merge two layers; never overrides on the destination */
// (merge_layer() is not used.)

/* search instrument id from the layer */
fn search_inst(lay: &Layer) -> i32 {
    if lay.set[SF_INSTRUMENT] != 0 {
        lay.val[SF_INSTRUMENT] as i32
    } else {
        -1
    }
}

/* parse an instrument */
fn parse_inst(
    song: &mut MidiSong,
    g: &mut Globals,
    pr_lay: &Layer,
    preset: i32,
    inst: i32,
    order: i32,
) {
    let sf = &g.sf.sfinfo;
    let (Some(from), Some(to)) = (
        sf.insthdr.get(inst as usize),
        sf.insthdr.get(inst as usize + 1),
    ) else {
        return;
    };
    let from_ndx = from.bag_ndx as i32;
    let to_ndx = to.bag_ndx as i32;

    let mut glay = *pr_lay;
    for i in from_ndx..to_ndx {
        let mut lay = Layer::zeroed();
        parse_inst_layer(&mut lay, &g.sf.sfinfo, i);
        let sample = search_sample(&lay);
        if sample < 0 {
            /* global layer */
            append_layer(&mut glay, &lay, &g.sf.sfinfo);
        } else {
            append_layer(&mut lay, &glay, &g.sf.sfinfo);
            make_inst(song, g, &lay, preset, inst, order);
        }
    }
}

/* parse instrument generator layers */
fn parse_inst_layer(lay: &mut Layer, sf: &SFInfo, idx: i32) {
    for i in bag_range(&sf.instbag, idx) {
        let Some(gen) = sf.instgen.get(i) else {
            break;
        };
        parse_gen(lay, gen);
    }
}

/* search a sample id from instrument layers */
fn search_sample(lay: &Layer) -> i32 {
    if lay.set[SF_SAMPLE_ID] != 0 {
        lay.val[SF_SAMPLE_ID] as i32
    } else {
        -1
    }
}

/* two (high/low) 8 bit values in 16 bit parameter */
fn lo_val(val: i16) -> i32 {
    val as i32 & 0xff
}
fn hi_val(val: i16) -> i32 {
    (val as i32 >> 8) & 0xff
}
fn set_lo(vp: &mut i16, val: i32) {
    *vp = ((*vp as i32 & 0xff00) | val) as i16;
}
fn set_hi(vp: &mut i16, val: i32) {
    *vp = ((*vp as i32 & 0xff) | (val << 8)) as i16;
}

/* append two layers; parameters are added to the original value */
fn append_layer(dst: &mut Layer, src: &Layer, sf: &SFInfo) {
    for i in 0..SFPARM_SIZE {
        if src.set[i] != 0 {
            if sf.version == 1 && i == SF_INST_VOL {
                dst.val[i] = ((src.val[i] as i32 * 127) / 127) as i16;
            } else if i == SF_KEY_RANGE || i == SF_VEL_RANGE {
                /* high limit */
                if hi_val(dst.val[i]) > hi_val(src.val[i]) {
                    set_hi(&mut dst.val[i], hi_val(src.val[i]));
                }
                /* low limit */
                if lo_val(dst.val[i]) < lo_val(src.val[i]) {
                    set_lo(&mut dst.val[i], lo_val(src.val[i]));
                }
            } else {
                dst.val[i] = dst.val[i].wrapping_add(src.val[i]);
            }
            dst.set[i] = 1;
        }
    }
}

/* convert layer info to timidity instrument strucutre */
fn make_inst(
    song: &mut MidiSong,
    g: &mut Globals,
    lay: &Layer,
    pr_idx: i32,
    in_idx: i32,
    mut order: i32,
) {
    let sf = &g.sf.sfinfo;
    let bank = sf.presethdr[pr_idx as usize].bank as i32;
    let preset = sf.presethdr[pr_idx as usize].preset as i32;
    let keynote;
    let n_order;

    // FIXME (upstream): a sample number past the samples reads past them.
    let Some(&sample) = sf.sampleinfo.get(lay.val[SF_SAMPLE_ID] as usize) else {
        return;
    };
    if sample.sampletype & 0x8000 != 0 {
        /* is ROM sample? */
        return;
    }

    /* set bank/preset name */
    let (tone_bank, master) = if bank == 128 {
        keynote = lo_val(lay.val[SF_KEY_RANGE]);
        (
            song.drumset.get_mut(preset as usize),
            g.master_drumset.get_mut(preset as usize),
        )
    } else {
        keynote = -1;
        (
            song.tonebank.get_mut(bank as usize),
            g.master_tonebank.get_mut(bank as usize),
        )
    };
    let tone_index = if bank == 128 { keynote } else { preset } as usize;
    if is_excluded(&g.sf, bank, preset, keynote) {
        return;
    }
    n_order = is_ordered(&g.sf, bank, preset, keynote);
    if n_order >= 0 {
        order = n_order;
    }

    // FIXME (upstream): a drum key, or a preset, over 127 indexes past the
    // bank's tones; skip it.
    let (Some(Some(tone_bank)), Some(master)) = (tone_bank, master) else {
        return;
    };
    let Some(namep) = tones(tone_bank, master).and_then(|t| t.get_mut(tone_index)) else {
        return;
    };
    let sf = &g.sf.sfinfo;
    if namep.name.is_none() {
        let mut name = sf.insthdr[in_idx as usize].name.to_vec();
        // (*namep)[20] = 0: the name is up to the first NUL.
        if let Some(n) = name.iter().position(|&c| c == 0) {
            name.truncate(n);
        }
        namep.name = Some(name);
    }

    /* search current instrument list */
    let rec = &mut g.sf.sfrec;
    let ip = match rec.instlist.iter().rposition(|ip| {
        ip.bank == bank && ip.preset == preset && (keynote < 0 || keynote == ip.keynote)
    }) {
        Some(ip) => ip,
        None => {
            if rec.instlist.try_reserve(1).is_err() {
                song.oom = 1;
                return;
            }
            rec.instlist.push(InstList {
                bank,
                preset,
                keynote,
                order,
                samples: 0,
                slist: Vec::new(),
            });
            rec.instlist.len() - 1
        }
    };

    /* add a sample */
    let mut sp = SampleList::default();

    let val = &lay.val;
    /* set sample position */
    sp.startsample = (val[SF_START_ADDRS_HI] as i32)
        .wrapping_shl(16)
        .wrapping_add(val[SF_START_ADDRS] as i32)
        .wrapping_add(sample.startsample);
    sp.endsample = (val[SF_END_ADDRS_HI] as i32)
        .wrapping_shl(16)
        .wrapping_add(val[SF_END_ADDRS] as i32)
        .wrapping_add(sample.endsample)
        .wrapping_sub(sp.startsample);

    /* set loop position */
    // (upstream adds the high words twice, and not the low ones.)
    sp.v.loop_start = (val[SF_STARTLOOP_ADDRS_HI] as i32)
        .wrapping_shl(16)
        .wrapping_add(val[SF_STARTLOOP_ADDRS_HI] as i32)
        .wrapping_add(sample.startloop)
        .wrapping_sub(sp.startsample);
    sp.v.loop_end = (val[SF_ENDLOOP_ADDRS_HI] as i32)
        .wrapping_shl(16)
        .wrapping_add(val[SF_ENDLOOP_ADDRS_HI] as i32)
        .wrapping_add(sample.endloop)
        .wrapping_sub(sp.startsample);
    sp.v.data_length = sp.endsample;

    sp.v.sample_rate = sample.samplerate;
    if lay.set[SF_KEY_RANGE] != 0 {
        // (LO_VAL and HI_VAL are at most 255; freq_table has 128 entries.)
        sp.v.low_freq = freq_table(lo_val(lay.val[SF_KEY_RANGE]));
        sp.v.high_freq = freq_table(hi_val(lay.val[SF_KEY_RANGE]));
    } else {
        sp.v.low_freq = FREQ_TABLE[0];
        sp.v.high_freq = FREQ_TABLE[127];
    }

    /* scale tuning: 0  - 100 */
    sp.v.scale_tuning = 100;
    if lay.set[SF_SCALE_TUNING] != 0 {
        if sf.version == 1 {
            sp.v.scale_tuning = if lay.val[SF_SCALE_TUNING] != 0 {
                50
            } else {
                100
            };
        } else {
            sp.v.scale_tuning = lay.val[SF_SCALE_TUNING];
        }
    }

    /* root pitch */
    sp.v.root_freq = calc_root_pitch(lay, sf, &sp, &sample);

    sp.v.modes = MODES_16BIT;

    /* volume envelope & total volume */
    sp.v.volume = calc_volume(lay, sf);
    if lay.val[SF_SAMPLE_FLAGS] == 1 || lay.val[SF_SAMPLE_FLAGS] == 3 {
        sp.v.modes |= MODES_LOOPING | MODES_SUSTAIN;
        if !SF_SUPPRESS_ENVELOPE {
            convert_volume_envelope(song, lay, sf, &mut sp);
        }
        if lay.val[SF_SAMPLE_FLAGS] == 3 {
            /* strip the tail */
            sp.v.data_length = sp.v.loop_end.wrapping_add(1);
        }
    }

    /* panning position: 0 to 127 */
    sp.v.panning = 64;
    if lay.set[SF_PAN_EFFECTS_SEND] != 0 {
        if sf.version == 1 {
            sp.v.panning = lay.val[SF_PAN_EFFECTS_SEND] as i8;
        } else {
            sp.v.panning = (((lay.val[SF_PAN_EFFECTS_SEND] as i32) + 500) * 127 / 1000) as i8;
        }
    }

    /* tremolo & vibrato */
    sp.v.tremolo_sweep_increment = 0;
    sp.v.tremolo_phase_increment = 0;
    sp.v.tremolo_depth = 0;
    if !SF_SUPPRESS_TREMOLO {
        convert_tremolo(song, lay, sf, &mut sp);
    }
    sp.v.vibrato_sweep_increment = 0;
    sp.v.vibrato_control_ratio = 0;
    sp.v.vibrato_depth = 0;
    if !SF_SUPPRESS_VIBRATO {
        convert_vibrato(song, lay, sf, &mut sp);
    }

    /* set note to use for drum voices */
    if bank == 128 {
        sp.v.note_to_use = keynote as i8;
    } else {
        sp.v.note_to_use = 0;
    }

    /* convert to fractional samples */
    sp.v.data_length = sp.v.data_length.wrapping_shl(FRACTION_BITS as u32);
    sp.v.loop_start = sp.v.loop_start.wrapping_shl(FRACTION_BITS as u32);
    sp.v.loop_end = sp.v.loop_end.wrapping_shl(FRACTION_BITS as u32);

    /* point to the file position */
    sp.startsample = sp.startsample.wrapping_mul(2).wrapping_add(sf.samplepos);
    sp.endsample = sp.endsample.wrapping_mul(2);

    /* set cutoff frequency */
    sp.cutoff_freq = 0;
    if lay.set[SF_INITIAL_FILTER_FC] != 0 || lay.set[SF_ENV1_TO_FILTER_FC] != 0 {
        calc_cutoff(lay, sf, &mut sp);
    }
    if lay.set[SF_INITIAL_FILTER_Q] != 0 {
        calc_filter_q(lay, sf, &mut sp);
    }

    let ipr = &mut g.sf.sfrec.instlist[ip];
    if ipr.slist.try_reserve(1).is_err() {
        song.oom = 1;
        return;
    }
    ipr.slist.push(sp);
    ipr.samples += 1;
}

/// `freq_table[i]`.
fn freq_table(i: i32) -> i32 {
    // FIXME (upstream): a key over 127 reads past freq_table; use 0.
    FREQ_TABLE.get(i as usize).copied().unwrap_or(0)
}

/* calculate root pitch */
fn calc_root_pitch(
    lay: &Layer,
    sf: &SFInfo,
    sp: &SampleList,
    sample: &crate::readsbk::Tsampleinfo,
) -> i32 {
    let mut root: i32 = sample.original_pitch as i32;
    let mut tune: i32 = sample.pitch_correction as i32;
    if sf.version == 1 {
        if lay.set[SF_SAMPLE_PITCH] != 0 {
            root = lay.val[SF_SAMPLE_PITCH] as i32 / 100;
            tune = -(lay.val[SF_SAMPLE_PITCH] as i32) % 100;
            if tune <= -50 {
                root += 1;
                tune += 100;
            }
            if sp.v.scale_tuning == 50 {
                tune /= 2;
            }
        }
        /* orverride root key */
        if lay.set[SF_ROOT_KEY] != 0 {
            root += lay.val[SF_ROOT_KEY] as i32 - 60;
        }
        /* tuning */
        tune += lay.val[SF_COARSE_TUNE] as i32 * sp.v.scale_tuning as i32
            + lay.val[SF_FINE_TUNE] as i32 * sp.v.scale_tuning as i32 / 100;
    } else {
        /* orverride root key */
        if lay.set[SF_ROOT_KEY] != 0 {
            root = lay.val[SF_ROOT_KEY] as i32;
        }
        /* tuning */
        tune += lay.val[SF_COARSE_TUNE] as i32 * 100 + lay.val[SF_FINE_TUNE] as i32;
    }
    /* it's too high.. */
    if lay.set[SF_KEY_RANGE] != 0 && root >= hi_val(lay.val[SF_KEY_RANGE]) + 60 {
        root -= 60;
    }

    while tune <= -100 {
        root += 1;
        tune += 100;
    }
    while tune > 0 {
        root -= 1;
        tune -= 100;
    }
    c_f64_to_i32(freq_table(root) as f64 * BEND_FINE[((-tune * 255) / 100) as usize])
}

/*----------------------------------------------------------------
 * convert volume envelope
 *----------------------------------------------------------------*/

fn convert_volume_envelope(song: &MidiSong, lay: &Layer, sf: &SFInfo, sp: &mut SampleList) {
    let sustain = calc_sustain(lay, sf);
    /*int delay = to_msec(lay, sf, SF_delayEnv2);*/
    let attack = to_msec(lay, sf, SF_ATTACK_ENV2);
    let hold = to_msec(lay, sf, SF_HOLD_ENV2);
    let decay = to_msec(lay, sf, SF_DECAY_ENV2);
    let release = to_msec(lay, sf, SF_RELEASE_ENV2);

    sp.v.envelope_offset[0] = to_offset(255);
    sp.v.envelope_rate[0] = calc_rate(song, 255, attack).wrapping_mul(2);

    sp.v.envelope_offset[1] = to_offset(250);
    sp.v.envelope_rate[1] = calc_rate(song, 5, hold);
    sp.v.envelope_offset[2] = to_offset(sustain);
    sp.v.envelope_rate[2] = calc_rate(song, 250 - sustain, decay);
    sp.v.envelope_offset[3] = to_offset(5);
    sp.v.envelope_rate[3] = calc_rate(song, 255, release);
    sp.v.envelope_offset[4] = to_offset(4);
    sp.v.envelope_rate[4] = to_offset(200);
    sp.v.envelope_offset[5] = to_offset(4);
    sp.v.envelope_rate[5] = to_offset(200);

    sp.v.modes |= MODES_ENVELOPE;
}

/* convert from 8bit value to fractional offset (15.15) */
fn to_offset(offset: i32) -> i32 {
    offset.wrapping_shl(7 + 15)
}

/* calculate ramp rate in fractional unit;
 * diff = 8bit, time = msec
 */
fn calc_rate(song: &MidiSong, mut diff: i32, mut time: i32) -> i32 {
    if time < 6 {
        time = 6;
    }
    if diff == 0 {
        diff = 255;
    }
    diff = diff.wrapping_shl(7 + 15);
    let mut rate = (diff / song.rate).wrapping_mul(song.control_ratio);
    rate = rate.wrapping_mul(1000) / time;
    if crate::options::FAST_DECAY {
        rate = rate.wrapping_mul(2);
    }

    rate
}

/// `TO_MSEC(tcents)`.
fn to_msec_(tcents: i32) -> i32 {
    c_f64_to_i32(1000.0 * pow(2.0, tcents as f64 / 1200.0))
}
/// `TO_MHZ(abscents)`.
fn to_mhz(abscents: i32) -> i32 {
    c_f64_to_i32(8176.0 * pow(2.0, abscents as f64 / 1200.0))
}
/// `TO_HZ(abscents)`.
fn to_hz(abscents: i32) -> i32 {
    c_f64_to_i32(8.176 * pow(2.0, abscents as f64 / 1200.0))
}
/// `TO_LINEAR(centibel)`.
fn to_linear(centibel: f64) -> f64 {
    pow(10.0, -centibel / 200.0)
}
/// `TO_VOLUME(centibel)`.
fn to_volume(centibel: i32) -> u8 {
    c_f64_to_i32(255.0 * (1.0 - (centibel as f64) / (1200.0 * log10(2.0)))) as u8
}

/* convert the value to milisecs */
fn to_msec(lay: &Layer, sf: &SFInfo, index: usize) -> i32 {
    if lay.set[index] == 0 {
        return 6; /* 6msec minimum */
    }
    let value = lay.val[index];
    if sf.version == 1 {
        value as i32
    } else {
        to_msec_(value as i32)
    }
}

/* convert peak volume to linear volume (0-255) */
fn calc_volume(lay: &Layer, sf: &SFInfo) -> f32 {
    if sf.version == 1 {
        ((lay.val[SF_INST_VOL] as i32 * 2) as f32 as f64 / 255.0) as f32
    } else {
        to_linear(lay.val[SF_INST_VOL] as f64 / 10.0) as f32
    }
}

/* convert sustain volume to linear volume */
fn calc_sustain(lay: &Layer, sf: &SFInfo) -> i32 {
    if lay.set[SF_SUSTAIN_ENV2] == 0 {
        return 250;
    }
    let mut level: i32 = lay.val[SF_SUSTAIN_ENV2] as i32;
    if sf.version == 1 {
        if level < 96 {
            level = 1000 * (96 - level) / 96;
        } else {
            return 0;
        }
    }
    to_volume(level) as i32
}

/*----------------------------------------------------------------
 * tremolo (LFO1) conversion
 *----------------------------------------------------------------*/

fn convert_tremolo(song: &MidiSong, lay: &Layer, sf: &SFInfo, sp: &mut SampleList) {
    let _ = song;

    if lay.set[SF_LFO1_TO_VOLUME] == 0 {
        return;
    }

    let mut level: i32 = lay.val[SF_LFO1_TO_VOLUME] as i32;
    if sf.version == 1 {
        level = (120 * level) / 64; /* to centibel */
    }
    /* centibel to linear */
    sp.v.tremolo_depth = c_f64_to_i32(to_linear(level as f64)) as u8;

    /* frequency in mHz */
    // (Note (upstream): the test is the wrong way round: a frequency the
    // SoundFont sets is ignored, and when it sets none, 0 is used:
    // TO_MHZ(0), 8.176 Hz.)
    let mut freq: i32;
    if lay.set[SF_FREQ_LFO1] != 0 {
        if sf.version == 1 {
            freq = to_mhz(-725);
        } else {
            freq = 0;
        }
    } else {
        freq = lay.val[SF_FREQ_LFO1] as i32;
        if freq > 0 && sf.version == 1 {
            freq = c_f64_to_i32(3986.0 * log10(freq as f64) - 7925.0);
        }
        freq = to_mhz(freq);
    }
    /* convert mHz to sine table increment; 1024<<rate_shift=1wave */
    sp.v.tremolo_phase_increment = freq.wrapping_mul(1024).wrapping_shl(RATE_SHIFT as u32);

    sp.v.tremolo_sweep_increment = 0;
}

/*----------------------------------------------------------------
 * vibrato (LFO2) conversion
 *----------------------------------------------------------------*/

fn convert_vibrato(song: &MidiSong, lay: &Layer, sf: &SFInfo, sp: &mut SampleList) {
    if lay.set[SF_LFO2_TO_PITCH] == 0 {
        return;
    }

    /* pitch shift in cents (= 1/100 semitone) */
    let mut shift: i32 = lay.val[SF_LFO2_TO_PITCH] as i32;
    if sf.version == 1 {
        shift = (1200 * shift / 64 + 1) / 2;
    }

    /* cents to linear; 400cents = 256 */
    sp.v.vibrato_depth = (shift * 256 / 400) as i8 as u8;

    /* frequency in mHz */
    // (Note (upstream): as in convert_tremolo().)
    let mut freq: i32;
    if lay.set[SF_FREQ_LFO2] != 0 {
        if sf.version == 1 {
            freq = to_mhz(-725);
        } else {
            freq = 0;
        }
    } else {
        freq = lay.val[SF_FREQ_LFO2] as i32;
        if freq > 0 && sf.version == 1 {
            freq = c_f64_to_i32(3986.0 * log10(freq as f64) - 7925.0);
        }
        freq = to_mhz(freq);
    }
    /* convert mHz to control ratio */
    sp.v.vibrato_control_ratio = freq.wrapping_mul(VIBRATO_RATE_TUNING.wrapping_mul(song.rate))
        / (2 * VIBRATO_SAMPLE_INCREMENTS as i32);

    sp.v.vibrato_sweep_increment = 0;
}

/* calculate cutoff/resonance frequency */
fn calc_cutoff(lay: &Layer, sf: &SFInfo, sp: &mut SampleList) {
    let mut val: i16;
    if lay.set[SF_INITIAL_FILTER_FC] == 0 {
        val = 13500;
    } else {
        val = lay.val[SF_INITIAL_FILTER_FC];
        if sf.version == 1 {
            if val == 127 {
                val = 14400;
            } else if val > 0 {
                val = (50 * val as i32 + 4366) as i16;
            }
        }
    }
    if lay.set[SF_ENV1_TO_FILTER_FC] != 0 {
        val = val.wrapping_add(lay.val[SF_ENV1_TO_FILTER_FC]);
    }
    if val >= 13500 {
        sp.cutoff_freq = 0;
    } else {
        sp.cutoff_freq = to_hz(val as i32);
    }
}

fn calc_filter_q(lay: &Layer, sf: &SFInfo, sp: &mut SampleList) {
    let mut val: i16 = lay.val[SF_INITIAL_FILTER_Q];
    if sf.version == 1 {
        val = (val as i32 * 3 / 2) as i16; /* to centibels */
    }
    sp.resonance = (pow(10.0, val as f64 / 2.0 / 200.0) - 1.0) as f32;
    if sp.resonance < 0.0 {
        sp.resonance = 0.0;
    }
}

/*----------------------------------------------------------------
 * low-pass filter:
 * 	y(n) = A * x(n) + B * y(n-1)
 * 	A = 2.0 * pi * center
 * 	B = exp(-A / frequency)
 *----------------------------------------------------------------
 * resonance filter:
 *	y(n) = a * x(n) - b * y(n-1) - c * y(n-2)
 *	c = exp(-2 * pi * width / rate)
 *	b = -4 * c / (1+c) * cos(2 * pi * center / rate)
 *	a = sqt(1-b*b/(4 * c)) * (1-c)
 *----------------------------------------------------------------*/

const MAX_DATAVAL: f64 = 32767.0;
const MIN_DATAVAL: f64 = -32768.0;

// (built only without SF_SUPPRESS_CUTOFF, which upstream defines.)
fn do_lowpass(sp: &mut Sample, freq: i32, resonance: f32) {
    let mut a: f64;
    let mut b: f64;
    let c: f64;

    if freq > sp.sample_rate * 2 {
        crate::snddbg!("Lowpass: center must be < data rate*2\n");
        return;
    }
    a = 2.0 * std::f64::consts::PI * freq as f64 * 2.5 / sp.sample_rate as f64;
    b = sdl3::stdlib::math::exp(-a / sp.sample_rate as f64);
    a *= 0.8;
    b *= 0.8;
    c = 0.0;

    /*
    if (resonance) {
        double a, b, c;
        Sint32 width;
        width = freq / 5;
        c = exp(-2.0 * M_PI * width / sp->sample_rate);
        b = -4.0 * c / (1+c) * cos(2.0 * M_PI * freq / sp->sample_rate);
        a = sqrt(1 - b * b / (4 * c)) * (1 - c);
        b = -b; c = -c;

        A += a * resonance;
        B += b;
        C = c;
    }
    */
    let _ = resonance;

    let mut pv1: i16 = 0;
    let mut pv2: i16 = 0;
    for i in 0..sp.data_length.max(0) as usize {
        let Some(&l) = sp.data.get(i) else {
            break;
        };
        let mut d = a * l as f64 + b * pv1 as f64 + c * pv2 as f64;
        if d > MAX_DATAVAL {
            d = MAX_DATAVAL;
        } else if d < MIN_DATAVAL {
            d = MIN_DATAVAL;
        }
        pv2 = pv1;
        pv1 = d as i16;
        sp.data[i] = pv1;
    }
}
