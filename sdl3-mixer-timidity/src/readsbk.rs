/*

    TiMidity -- Experimental MIDI to WAVE converter
    Copyright (C) 1995 Tuukka Toivonen <toivonen@clinet.fi>

    readsbk.c: read soundfont file
    Copyright (C) 1996,1997 Takashi Iwai

    This program is free software; you can redistribute it and/or modify
    it under the terms of the Perl Artistic License, available in COPYING.
*/
// Modified 2026-10-07: translated into Rust from readsbk.c and sbk.h in SDL_mixer 3.3.0's TiMidity (see NOTICE).

//! Reading the SoundFont file's headers. Translation of `readsbk.c` (and
//! `sbk.h`).
//!
//! The arrays read here are as long as the counts in their chunks'
//! headers; their entries are only allocated as they are read (upstream
//! allocates them all first, which fails the same way when the count is
//! negative, and when the file is shorter than its headers say).

use sdl3::io::{IoStream, IoWhence};

/*----------------------------------------------------------------
 * sbk.h
 *----------------------------------------------------------------*/

/// Translation of `tchunk`.
#[derive(Clone, Copy, Default)]
struct Tchunk {
    id: [u8; 4],
    size: i32,
}

/// Translation of `tsamplenames`.
pub(crate) type Tsamplenames = [u8; 20];

/// Translation of `tpresethdr`.
#[derive(Clone, Copy, Default)]
pub(crate) struct Tpresethdr {
    pub name: [u8; 20],
    pub preset: u16,
    pub bank: u16,
    pub bag_ndx: u16,
    /*int lib, genre, morphology;*/ /* reserved */
}

/// Translation of `tsampleinfo`.
#[derive(Clone, Copy, Default)]
pub(crate) struct Tsampleinfo {
    pub startsample: i32,
    pub endsample: i32,
    pub startloop: i32,
    pub endloop: i32,
    /* ver.2 additional info */
    pub samplerate: i32,
    pub original_pitch: u8,
    pub pitch_correction: u8,
    pub samplelink: u16,
    pub sampletype: u16, /*1=mono, 2=right, 4=left, 8=linked, $8000=ROM*/
}

/// Translation of `tinsthdr`.
#[derive(Clone, Copy, Default)]
pub(crate) struct Tinsthdr {
    pub name: [u8; 20],
    pub bag_ndx: u16,
}

/// Translation of `tgenrec`.
#[derive(Clone, Copy, Default)]
pub(crate) struct Tgenrec {
    pub oper: i16,
    pub amount: i16,
}

/// Translation of `SFInfo`.
#[derive(Default)]
pub(crate) struct SFInfo {
    pub version: u16,
    pub minorversion: u16,
    pub samplepos: i32,
    pub samplesize: i32,

    pub nrsamples: i32,
    pub samplenames: Vec<Tsamplenames>,

    pub nrpresets: i32,
    pub presethdr: Vec<Tpresethdr>,

    pub nrinfos: i32,
    pub sampleinfo: Vec<Tsampleinfo>,

    pub nrinsts: i32,
    pub insthdr: Vec<Tinsthdr>,

    pub nrpbags: i32,
    pub nribags: i32,
    pub presetbag: Vec<u16>,
    pub instbag: Vec<u16>,

    pub nrpgens: i32,
    pub nrigens: i32,
    pub presetgen: Vec<Tgenrec>,
    pub instgen: Vec<Tgenrec>,

    /*tsbkheader sbkh;*/

    /*char *sf_name;*/
    pub in_rom: i32,
}

impl SFInfo {
    pub const fn new() -> SFInfo {
        SFInfo {
            version: 0,
            minorversion: 0,
            samplepos: 0,
            samplesize: 0,
            nrsamples: 0,
            samplenames: Vec::new(),
            nrpresets: 0,
            presethdr: Vec::new(),
            nrinfos: 0,
            sampleinfo: Vec::new(),
            nrinsts: 0,
            insthdr: Vec::new(),
            nrpbags: 0,
            nribags: 0,
            presetbag: Vec::new(),
            instbag: Vec::new(),
            nrpgens: 0,
            nrigens: 0,
            presetgen: Vec::new(),
            instgen: Vec::new(),
            in_rom: 0,
        }
    }
}

/*----------------------------------------------------------------
 * function prototypes
 *----------------------------------------------------------------*/

/// `NEW(type,nums)`: room for `nums` entries, or `None` where
/// `SDL_calloc()` would fail (a negative count is a huge `size_t`).
fn new_vec<T>(nums: i32) -> Option<Vec<T>> {
    let n = usize::try_from(nums).ok()?;
    let mut v = Vec::new();
    v.try_reserve_exact(n).ok()?;
    Some(v)
}

fn readchunk(vp: &mut Tchunk, io: &mut IoStream<'_>) -> i32 {
    let mut b = [0u8; 8];
    if io.read(&mut b) != 8 {
        *vp = Tchunk::default();
        return -1;
    }
    vp.id.copy_from_slice(&b[..4]);
    vp.size = i32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    0
}

fn readid(var: &mut [u8; 4], io: &mut IoStream<'_>) -> i32 {
    // (a short read leaves what it read in var, as SDL_ReadIO() does.)
    let mut b = [0u8; 4];
    let n = io.read(&mut b);
    var[..n].copy_from_slice(&b[..n]);
    if n != 4 {
        return -1;
    }
    0
}

fn readdw(vp: &mut i32, io: &mut IoStream<'_>) -> i32 {
    let mut b = [0u8; 4];
    if io.read(&mut b) != 4 {
        return -1;
    }
    *vp = i32::from_le_bytes(b);
    0
}

fn readw(vp: &mut u16, io: &mut IoStream<'_>) -> i32 {
    let mut b = [0u8; 2];
    if io.read(&mut b) != 2 {
        return -1;
    }
    *vp = u16::from_le_bytes(b);
    0
}

fn readb(vp: &mut u8, io: &mut IoStream<'_>) -> i32 {
    let mut b = [0u8; 1];
    if io.read(&mut b) != 1 {
        return -1;
    }
    *vp = b[0];
    0
}

fn readstr(s: &mut [u8; 20], io: &mut IoStream<'_>) -> i32 {
    if io.read(s) != 20 {
        return -1;
    }
    s[19] = 0;
    let mut n = s.iter().position(|&c| c == 0).unwrap_or(20);
    while n > 0 && s[n - 1] == b' ' {
        n -= 1;
    }
    s[n] = 0;
    0
}

fn skipw(io: &mut IoStream<'_>) {
    let _ = io.seek(2, IoWhence::Cur);
}

fn skipdw(io: &mut IoStream<'_>) {
    let _ = io.seek(4, IoWhence::Cur);
}

/* level 0 */
const UNKN_ID: i32 = 0;
const RIFF_ID: i32 = 1;
const LIST_ID: i32 = 2;
const SFBK_ID: i32 = 3;
/* level 1 */
const INFO_ID: i32 = 4;
const SDTA_ID: i32 = 5;
const PDTA_ID: i32 = 6;
/* info stuff */
const IFIL_ID: i32 = 7;
const ISNG_ID: i32 = 8;
const IROM_ID: i32 = 9;
const INAM_ID: i32 = 10;
const IVER_ID: i32 = 11;
const IPRD_ID: i32 = 12;
const ICOP_ID: i32 = 13;
const ICRD_ID: i32 = 14;
const IENG_ID: i32 = 15;
const ISFT_ID: i32 = 16;
const ICMT_ID: i32 = 17;
/* sample data stuff */
const SNAM_ID: i32 = 18;
const SMPL_ID: i32 = 19;
/* preset stuff */
const PHDR_ID: i32 = 20;
const PBAG_ID: i32 = 21;
const PMOD_ID: i32 = 22;
const PGEN_ID: i32 = 23;
/* inst stuff */
const INST_ID: i32 = 24;
const IBAG_ID: i32 = 25;
const IMOD_ID: i32 = 26;
const IGEN_ID: i32 = 27;
/* sample header */
const SHDR_ID: i32 = 28;

/*----------------------------------------------------------------
 * debug routine
 *----------------------------------------------------------------*/

// (debugid(), debugname() and debugval() are compiled out upstream.)

/*----------------------------------------------------------------
 * load sbk file
 *----------------------------------------------------------------*/

/// Translation of `load_sbk()`.
pub(crate) fn load_sbk(io: &mut IoStream<'_>, sf: &mut SFInfo) -> i32 {
    let ioend = io.size().unwrap_or(-1);
    let mut chunk = Tchunk::default();
    let mut subchunk = Tchunk::default();

    if ioend < 32 {
        /* better?? */
        return -1;
    }

    if readchunk(&mut chunk, io) < 0 {
        return -1;
    }
    if getchunk(&chunk.id) != RIFF_ID {
        return -1;
    }
    if chunk.size as i64 != ioend - 8 {
        return -1;
    }

    if readid(&mut chunk.id, io) < 0 {
        return -1;
    }
    if getchunk(&chunk.id) != SFBK_ID {
        return -1;
    }

    sf.in_rom = 1;
    while io.tell().unwrap_or(-1) < ioend {
        if readid(&mut chunk.id, io) < 0 {
            return -1;
        }
        if getchunk(&chunk.id) == LIST_ID {
            if readdw(&mut chunk.size, io) < 0 {
                return -1;
            }
            if readid(&mut subchunk.id, io) < 0 {
                return -1;
            }
            if process_chunk(getchunk(&subchunk.id), chunk.size.wrapping_sub(4), sf, io) < 0 {
                return -1;
            }
        }
    }

    if sf.version < 1 || sf.version > 2 {
        crate::snddbg!("Unsupported soundfont version {}.\n", sf.version);
        return -1;
    }

    0
}

/*----------------------------------------------------------------
 * free buffer
 *----------------------------------------------------------------*/

/// Translation of `free_sbk()`.
pub(crate) fn free_sbk(sf: &mut SFInfo) {
    *sf = SFInfo::new();
}

/*----------------------------------------------------------------
 * get id value
 *----------------------------------------------------------------*/

fn getchunk(id: &[u8; 4]) -> i32 {
    static IDLIST: [(&[u8; 4], i32); 28] = [
        (b"RIFF", RIFF_ID),
        (b"LIST", LIST_ID),
        (b"sfbk", SFBK_ID),
        (b"INFO", INFO_ID),
        (b"sdta", SDTA_ID),
        (b"snam", SNAM_ID),
        (b"smpl", SMPL_ID),
        (b"pdta", PDTA_ID),
        (b"phdr", PHDR_ID),
        (b"pbag", PBAG_ID),
        (b"pmod", PMOD_ID),
        (b"pgen", PGEN_ID),
        (b"inst", INST_ID),
        (b"ibag", IBAG_ID),
        (b"imod", IMOD_ID),
        (b"igen", IGEN_ID),
        (b"shdr", SHDR_ID),
        (b"ifil", IFIL_ID),
        (b"isng", ISNG_ID),
        (b"irom", IROM_ID),
        (b"iver", IVER_ID),
        (b"INAM", INAM_ID),
        (b"IPRD", IPRD_ID),
        (b"ICOP", ICOP_ID),
        (b"ICRD", ICRD_ID),
        (b"IENG", IENG_ID),
        (b"ISFT", ISFT_ID),
        (b"ICMT", ICMT_ID),
    ];

    for (s, i) in IDLIST.iter() {
        if *s == id {
            return *i;
        }
    }

    UNKN_ID
}

fn load_sample_names(size: i32, sf: &mut SFInfo, io: &mut IoStream<'_>) -> i32 {
    sf.nrsamples = size / 20;
    let Some(mut names) = new_vec::<Tsamplenames>(sf.nrsamples) else {
        return -1;
    };
    for _ in 0..sf.nrsamples {
        let mut name = [0u8; 20];
        let rc = readstr(&mut name, io);
        names.push(name);
        if rc < 0 {
            sf.samplenames = names;
            return -1;
        }
    }
    sf.samplenames = names;
    0
}

fn load_preset_header(size: i32, sf: &mut SFInfo, io: &mut IoStream<'_>) -> i32 {
    sf.nrpresets = size / 38;
    let Some(mut hdrs) = new_vec::<Tpresethdr>(sf.nrpresets) else {
        return -1;
    };
    let rc = (|| {
        for _ in 0..sf.nrpresets {
            let mut h = Tpresethdr::default();
            if readstr(&mut h.name, io) < 0 {
                return -1;
            }
            if readw(&mut h.preset, io) < 0 {
                return -1;
            }
            if readw(&mut h.bank, io) < 0 {
                return -1;
            }
            if readw(&mut h.bag_ndx, io) < 0 {
                return -1;
            }
            skipdw(io); /* lib */
            skipdw(io); /* genre */
            skipdw(io); /* morph */
            hdrs.push(h);
        }
        0
    })();
    sf.presethdr = hdrs;
    rc
}

fn load_inst_header(size: i32, sf: &mut SFInfo, io: &mut IoStream<'_>) -> i32 {
    sf.nrinsts = size / 22;
    let Some(mut hdrs) = new_vec::<Tinsthdr>(sf.nrinsts) else {
        return -1;
    };
    let rc = (|| {
        for _ in 0..sf.nrinsts {
            let mut h = Tinsthdr::default();
            if readstr(&mut h.name, io) < 0 {
                return -1;
            }
            if readw(&mut h.bag_ndx, io) < 0 {
                return -1;
            }
            hdrs.push(h);
        }
        0
    })();
    sf.insthdr = hdrs;
    rc
}

fn load_bag(size: i32, io: &mut IoStream<'_>, totalp: &mut i32, bufp: &mut Vec<u16>) -> i32 {
    let size = size / 4;
    let Some(mut buf) = new_vec::<u16>(size) else {
        return -1;
    };
    for _ in 0..size {
        let mut b = 0u16;
        if readw(&mut b, io) < 0 {
            return -1;
        }
        buf.push(b);
        skipw(io); /* mod */
    }
    *totalp = size;
    *bufp = buf;
    0
}

fn load_gen(size: i32, io: &mut IoStream<'_>, totalp: &mut i32, bufp: &mut Vec<Tgenrec>) -> i32 {
    let size = size / 4;
    let Some(mut buf) = new_vec::<Tgenrec>(size) else {
        return -1;
    };
    for _ in 0..size {
        let mut oper = 0u16;
        let mut amount = 0u16;
        if readw(&mut oper, io) < 0 {
            return -1;
        }
        if readw(&mut amount, io) < 0 {
            return -1;
        }
        buf.push(Tgenrec {
            oper: oper as i16,
            amount: amount as i16,
        });
    }
    *totalp = size;
    *bufp = buf;
    0
}

fn load_sample_info(size: i32, sf: &mut SFInfo, io: &mut IoStream<'_>) -> i32 {
    let mut infos;
    let mut names = None;
    if sf.version > 1 {
        sf.nrinfos = size / 46;
        sf.nrsamples = sf.nrinfos;
        let (Some(i), Some(n)) = (
            new_vec::<Tsampleinfo>(sf.nrinfos),
            new_vec::<Tsamplenames>(sf.nrsamples),
        ) else {
            return -1;
        };
        infos = i;
        names = Some(n);
    } else {
        sf.nrinfos = size / 16;
        let Some(i) = new_vec::<Tsampleinfo>(sf.nrinfos) else {
            return -1;
        };
        infos = i;
    }

    let rc = (|| {
        for _ in 0..sf.nrinfos {
            let mut si = Tsampleinfo::default();
            if let Some(names) = names.as_mut() {
                let mut name = [0u8; 20];
                if readstr(&mut name, io) < 0 {
                    return -1;
                }
                names.push(name);
            }
            if readdw(&mut si.startsample, io) < 0 {
                return -1;
            }
            if readdw(&mut si.endsample, io) < 0 {
                return -1;
            }
            if readdw(&mut si.startloop, io) < 0 {
                return -1;
            }
            if readdw(&mut si.endloop, io) < 0 {
                return -1;
            }
            if sf.version > 1 {
                if readdw(&mut si.samplerate, io) < 0 {
                    return -1;
                }
                if readb(&mut si.original_pitch, io) < 0 {
                    return -1;
                }
                if readb(&mut si.pitch_correction, io) < 0 {
                    return -1;
                }
                if readw(&mut si.samplelink, io) < 0 {
                    return -1;
                }
                if readw(&mut si.sampletype, io) < 0 {
                    return -1;
                }
            } else {
                if si.startsample == 0 {
                    sf.in_rom = 0;
                }
                si.startloop = si.startloop.wrapping_add(1);
                si.endloop = si.endloop.wrapping_add(2);
                si.samplerate = 44100;
                si.original_pitch = 60;
                si.pitch_correction = 0;
                si.samplelink = 0;
                if sf.in_rom != 0 {
                    si.sampletype = 0x8001;
                } else {
                    si.sampletype = 1;
                }
            }
            infos.push(si);
        }
        0
    })();
    sf.sampleinfo = infos;
    if let Some(names) = names {
        sf.samplenames = names;
    }
    rc
}

// FIXME (upstream): a chunk whose size seeks back over itself makes these
// loops read the same chunks for ever (as upstream's do).
fn process_chunk(id: i32, s: i32, sf: &mut SFInfo, io: &mut IoStream<'_>) -> i32 {
    let ioend = io.size().unwrap_or(-1);
    let mut subchunk = Tchunk::default();

    let _ = s;

    match id {
        INFO_ID => {
            readchunk(&mut subchunk, io);
            loop {
                let cid = getchunk(&subchunk.id);
                if cid == LIST_ID {
                    break;
                }
                match cid {
                    IFIL_ID => {
                        if readw(&mut sf.version, io) < 0 {
                            return -1;
                        }
                        if readw(&mut sf.minorversion, io) < 0 {
                            return -1;
                        }
                        if sf.version > 2 {
                            crate::snddbg!("Unsupported soundfont version {}.\n", sf.version);
                            return -1;
                        }
                    }
                    /*
                    case INAM_ID:
                        sf->sf_name = (char *)SDL_malloc(subchunk.size + 1);
                        if (!sf->sf_name) return -1;
                        SDL_ReadIO(io, sf->sf_name, subchunk.size);
                        sf->sf_name[subchunk.size] = 0;
                        break;
                    */
                    _ => {
                        let _ = io.seek(subchunk.size as i64, IoWhence::Cur);
                    }
                }
                readchunk(&mut subchunk, io);
                if io.tell().unwrap_or(-1) >= ioend {
                    return 0;
                }
            }
            let _ = io.seek(-8, IoWhence::Cur); /* seek back */
        }

        SDTA_ID => {
            readchunk(&mut subchunk, io);
            loop {
                let cid = getchunk(&subchunk.id);
                if cid == LIST_ID {
                    break;
                }
                match cid {
                    SNAM_ID => {
                        if sf.version > 1 {
                            crate::snddbg!("**** version 2 has obsolete format??\n");
                            let _ = io.seek(subchunk.size as i64, IoWhence::Cur);
                        } else if load_sample_names(subchunk.size, sf, io) < 0 {
                            return -1;
                        }
                    }
                    SMPL_ID => {
                        sf.samplepos = io.tell().unwrap_or(-1) as i32;
                        sf.samplesize = subchunk.size;
                        let _ = io.seek(subchunk.size as i64, IoWhence::Cur);
                    }
                    _ => {}
                }
                readchunk(&mut subchunk, io);
                if io.tell().unwrap_or(-1) >= ioend {
                    return 0;
                }
            }
            let _ = io.seek(-8, IoWhence::Cur); /* seek back */
        }

        PDTA_ID => {
            readchunk(&mut subchunk, io);
            loop {
                let cid = getchunk(&subchunk.id);
                if cid == LIST_ID {
                    break;
                }
                match cid {
                    PHDR_ID => {
                        if load_preset_header(subchunk.size, sf, io) < 0 {
                            return -1;
                        }
                    }

                    PBAG_ID => {
                        let (mut n, mut v) = (0, Vec::new());
                        let rc = load_bag(subchunk.size, io, &mut n, &mut v);
                        if rc == 0 {
                            sf.nrpbags = n;
                            sf.presetbag = v;
                        } else {
                            return -1;
                        }
                    }

                    PMOD_ID => {
                        /* ignored */
                        let _ = io.seek(subchunk.size as i64, IoWhence::Cur);
                    }

                    PGEN_ID => {
                        let (mut n, mut v) = (0, Vec::new());
                        if load_gen(subchunk.size, io, &mut n, &mut v) < 0 {
                            return -1;
                        }
                        sf.nrpgens = n;
                        sf.presetgen = v;
                    }

                    INST_ID => {
                        if load_inst_header(subchunk.size, sf, io) < 0 {
                            return -1;
                        }
                    }

                    IBAG_ID => {
                        let (mut n, mut v) = (0, Vec::new());
                        if load_bag(subchunk.size, io, &mut n, &mut v) < 0 {
                            return -1;
                        }
                        sf.nribags = n;
                        sf.instbag = v;
                    }

                    IMOD_ID => {
                        /* ingored */
                        let _ = io.seek(subchunk.size as i64, IoWhence::Cur);
                    }

                    IGEN_ID => {
                        let (mut n, mut v) = (0, Vec::new());
                        if load_gen(subchunk.size, io, &mut n, &mut v) < 0 {
                            return -1;
                        }
                        sf.nrigens = n;
                        sf.instgen = v;
                    }

                    SHDR_ID => {
                        if load_sample_info(subchunk.size, sf, io) < 0 {
                            return -1;
                        }
                    }

                    _ => {
                        crate::snddbg!("unknown id\n");
                        let _ = io.seek(subchunk.size as i64, IoWhence::Cur);
                    }
                }
                readchunk(&mut subchunk, io);
                if io.tell().unwrap_or(-1) >= ioend {
                    return 0;
                }
            }
            let _ = io.seek(-8, IoWhence::Cur); /* rewind */
        }

        _ => {}
    }
    0
}

#[allow(dead_code)]
const _UNUSED_IDS: [i32; 13] = [
    UNKN_ID, ISNG_ID, IROM_ID, INAM_ID, IVER_ID, IPRD_ID, ICOP_ID, ICRD_ID, IENG_ID, ISFT_ID,
    ICMT_ID, SFBK_ID, RIFF_ID,
];
