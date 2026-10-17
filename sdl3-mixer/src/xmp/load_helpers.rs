// Rust translation of src/load_helpers.c from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! What every load does before and after the format's loader: defaults,
//! sanity checks, module quirks and the scan counters.

use super::common::*;
use super::extras::ModuleExtras;
use super::loaders::common::libxmp_alloc_pattern;
use super::*;

/*
 * Handle special "module quirks" that can't be detected automatically
 * such as Protracker 2.x compatibility, vblank timing, etc.
 */

/// Translation of `struct module_quirk`.
struct ModuleQuirk {
    md5: [u8; 16],
    flags: i32,
    mode: i32,
}

const MQ: &[ModuleQuirk] = &[
    /* "No Mercy" by Alf/VTL (added by Martin Willers) */
    ModuleQuirk {
        md5: [
            0x36, 0x6e, 0xc0, 0xfa, 0x96, 0x2a, 0xeb, 0xee, 0x03, 0x4a, 0xa2, 0xdb, 0xaa, 0x49,
            0xaa, 0xea,
        ],
        flags: 0,
        mode: XMP_MODE_PROTRACKER,
    },
    /* mod.souvenir of china */
    ModuleQuirk {
        md5: [
            0x93, 0xf1, 0x46, 0xae, 0xb7, 0x58, 0xc3, 0x9d, 0x8b, 0x5f, 0xbc, 0x98, 0xbf, 0x23,
            0x7a, 0x43,
        ],
        flags: XMP_FLAGS_FIXLOOP,
        mode: XMP_MODE_AUTO,
    },
    /* (upstream keeps "siedler ii" here under #if 0: timing fixed by
     * vblank scan compare) */
    /* "Klisje paa klisje" (added by Kjetil Torgrim Homme) */
    ModuleQuirk {
        md5: [
            0xe9, 0x98, 0x01, 0x2c, 0x70, 0x0e, 0xb4, 0x3a, 0xf0, 0x32, 0x17, 0x11, 0x30, 0x58,
            0x29, 0xb2,
        ],
        flags: 0,
        mode: XMP_MODE_NOISETRACKER,
    },
    /* (upstream keeps another version of Klisje paa klisje here under
     * #if 0: already covered by Noisetracker fingerprinting) */
    /* "((((( nebulos )))))" sent by Tero Auvinen (AMP version) */
    ModuleQuirk {
        md5: [
            0x51, 0x6e, 0x8d, 0xcc, 0x35, 0x7d, 0x50, 0xde, 0xa9, 0x85, 0xbe, 0xbf, 0x90, 0x2e,
            0x42, 0xdc,
        ],
        flags: 0,
        mode: XMP_MODE_NOISETRACKER,
    },
    /* Purple Motion's Sundance.mod, Music Channel BBS edit */
    ModuleQuirk {
        md5: [
            0x5d, 0x3e, 0x1e, 0x08, 0x28, 0x52, 0x12, 0xc7, 0x17, 0x64, 0x95, 0x75, 0x98, 0xe6,
            0x95, 0xc1,
        ],
        flags: 0,
        mode: XMP_MODE_ST3,
    },
    /* Asle's Ode to Protracker */
    ModuleQuirk {
        md5: [
            0x97, 0xa3, 0x7d, 0x30, 0xd7, 0xae, 0x6d, 0x50, 0xc9, 0x62, 0xe9, 0xd8, 0x87, 0x1b,
            0x7e, 0x8a,
        ],
        flags: 0,
        mode: XMP_MODE_PROTRACKER,
    },
    /* grooving3.mod */
    /* length 150778 crc32c 0xfdcf9aadU */
    ModuleQuirk {
        md5: [
            0xdb, 0x61, 0x22, 0x44, 0x39, 0x85, 0x74, 0xe9, 0xfa, 0x11, 0xb8, 0xfb, 0x87, 0xe8,
            0xde, 0xc5,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* mod.Rundgren */
    /* length 195078 crc32c 0x8fa827a4U */
    ModuleQuirk {
        md5: [
            0x9a, 0xdb, 0xb2, 0x09, 0x07, 0x1c, 0x44, 0x82, 0xc5, 0xdf, 0x83, 0x52, 0xcc, 0x73,
            0x9f, 0x20,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* dance feeling by Audiomonster */
    /* length 169734 crc32c 0x79fa2c9bU */
    ModuleQuirk {
        md5: [
            0x31, 0x2c, 0x3d, 0xaa, 0x5f, 0x1a, 0x54, 0x44, 0x9d, 0xf7, 0xc4, 0x41, 0x8a, 0xc5,
            0x01, 0x02,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* knights melody by Audiomonster */
    /* length 77798 crc32c 0x7bf19c5bU */
    ModuleQuirk {
        md5: [
            0x31, 0xc3, 0x0e, 0x32, 0xfc, 0x99, 0x95, 0xd2, 0x97, 0x20, 0xb3, 0x77, 0x50, 0x05,
            0xfe, 0xa5,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* hcomme by Bouffon */
    /* length 71346 crc32c 0x4ad49cb3U */
    ModuleQuirk {
        md5: [
            0x6e, 0xf9, 0x78, 0xc1, 0x80, 0xae, 0x51, 0x06, 0x05, 0x7c, 0x6e, 0xd0, 0x26, 0x7e,
            0xfe, 0x3d,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* ((((aquapool)))) by Dolphin */
    /* length 62932 crc32c 0x05b103fcU */
    ModuleQuirk {
        md5: [
            0xff, 0x0b, 0xe0, 0x26, 0xc6, 0x31, 0xb5, 0x9b, 0x94, 0x83, 0x94, 0x99, 0x7e, 0x24,
            0x7c, 0xdd,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* 100yarddash by Dr. Awesome */
    /* length 104666 crc32c 0xd2b0e4a6U */
    ModuleQuirk {
        md5: [
            0x5b, 0xff, 0x2f, 0xb8, 0xef, 0x3c, 0xbe, 0x55, 0xa8, 0xe2, 0xa7, 0xcf, 0x5c, 0xbd,
            0xdd, 0xb2,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* jazz-reggae-funk by Droid */
    /* length 115564 crc32c 0x41ff635fU */
    ModuleQuirk {
        md5: [
            0xe5, 0x6e, 0x31, 0x2f, 0x62, 0x80, 0xc1, 0x9d, 0x2f, 0x24, 0x54, 0xf3, 0x89, 0x3f,
            0x94, 0x6c,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* hard and heavy by Fish */
    /* length 69814 crc32c 0x1f09d3d5U */
    ModuleQuirk {
        md5: [
            0x6b, 0xce, 0x39, 0x94, 0x75, 0x42, 0x06, 0x74, 0xd2, 0x83, 0xbc, 0x5e, 0x7b, 0x42,
            0x1f, 0xa0,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* crazy valley by Julius and Droid */
    /* length 97496 crc32c 0xb8eec40eU */
    ModuleQuirk {
        md5: [
            0x23, 0x77, 0x18, 0x1d, 0x21, 0x9b, 0x41, 0x8f, 0xc1, 0xb4, 0xf4, 0xf8, 0x22, 0xdd,
            0xd8, 0xb6,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* THE ILLOGICAL ONE by Rhino */
    /* length 173432 crc32c 0xcb4e2987U */
    ModuleQuirk {
        md5: [
            0xd8, 0xc2, 0xbb, 0xe6, 0x11, 0xd0, 0x5c, 0x02, 0x8e, 0x3b, 0xcb, 0x7c, 0x4a, 0x7d,
            0x43, 0xa0,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* sounds of holiday by Spacebrain */
    /* length 309520 crc32c 0x28804a57U */
    ModuleQuirk {
        md5: [
            0x36, 0x18, 0x19, 0xa4, 0x9d, 0xa2, 0xa2, 0x6f, 0x58, 0x60, 0xc4, 0xd9, 0x0d, 0xa2,
            0x9f, 0x49,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* sunisinus by Speed-Head */
    /* length 175706 crc32c 0x2e56451bU */
    ModuleQuirk {
        md5: [
            0x7e, 0x69, 0x44, 0xb6, 0x38, 0x0d, 0x27, 0x14, 0x70, 0x5d, 0x44, 0xce, 0xce, 0xdd,
            0x37, 0x31,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* eat the fulcrum bop by The Assassin */
    /* length 160286 crc32c 0x583a4683U */
    ModuleQuirk {
        md5: [
            0x11, 0xe9, 0x6f, 0x62, 0xe1, 0xc3, 0xc5, 0xcc, 0x3b, 0xaf, 0xea, 0x69, 0x4b, 0xce,
            0x5f, 0xec,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* obvious disaster by Tip */
    /* length 221086 crc32c 0x51c6d489U */
    ModuleQuirk {
        md5: [
            0x06, 0x8e, 0x69, 0x01, 0x49, 0x8f, 0xbd, 0x0f, 0xfc, 0xb7, 0x8f, 0x2a, 0x91, 0xe1,
            0x8b, 0xe8,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* alien nation by Turtle */
    /* length 167548 crc32c 0xc9ec1674U */
    ModuleQuirk {
        md5: [
            0x71, 0xdf, 0x11, 0xac, 0x5d, 0xec, 0x07, 0xf8, 0x10, 0x6f, 0x28, 0x8d, 0x47, 0x59,
            0x54, 0x9b,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
    /* illusions!2 by Zuhl */
    /* length 289770 crc32c 0x6bf5fbcfU */
    ModuleQuirk {
        md5: [
            0xca, 0x37, 0x8c, 0x0e, 0x87, 0x4f, 0x1e, 0xcd, 0xa3, 0xe9, 0x8b, 0xdd, 0x11, 0x46,
            0x8d, 0x69,
        ],
        flags: XMP_FLAGS_VBLANK,
        mode: XMP_MODE_AUTO,
    },
];

/// Translation of `module_quirks()`.
fn module_quirks(ctx: &mut ContextData) {
    for mq in MQ {
        if ctx.m.md5 == mq.md5 {
            ctx.p.flags |= mq.flags;
            ctx.p.mode = mq.mode;
        }
    }
}

/// Translation of `libxmp_adjust_string()`, on a NUL-terminated buffer.
pub(crate) fn libxmp_adjust_string(s: &mut [u8]) {
    let len = strlen(s);
    for c in &mut s[..len] {
        if !isprint(*c) || *c > 127 {
            *c = b' ';
        }
    }

    let mut len = len;
    while len > 0 && s[len - 1] == b' ' {
        s[len - 1] = 0;
        len -= 1;
    }
}

/// Translation of `check_envelope()`.
fn check_envelope(env: &mut XmpEnvelope) {
    /* Disable envelope if invalid number of points */
    if env.npt <= 0 || env.npt > XMP_MAX_ENV_POINTS as i32 {
        env.flg &= !XMP_ENVELOPE_ON;
    }

    /* Disable envelope loop if invalid loop parameters */
    if env.lps >= env.npt || env.lpe >= env.npt {
        env.flg &= !XMP_ENVELOPE_LOOP;
    }

    /* Disable envelope sustain if invalid sustain */
    if env.sus >= env.npt || env.sue >= env.npt {
        env.flg &= !XMP_ENVELOPE_SUS;
    }
}

/// Translation of `clamp_volume_envelope()`.
fn clamp_volume_envelope(volbase: i32, env: &mut XmpEnvelope) {
    /* Clamp broken values in the volume envelope to the expected range. */
    if env.flg & XMP_ENVELOPE_ON != 0 {
        for i in 0..env.npt as usize {
            let data = &mut env.data[i * 2 + 1];
            clamp(data, 0, volbase as i16);
        }
    }
}

/// Translation of `libxmp_load_prologue()`.
pub(crate) fn libxmp_load_prologue(ctx: &mut ContextData) {
    let m = &mut ctx.m;

    /* Reset variables */
    m.mod_ = XmpModule::default();
    m.rrate = PAL_RATE;
    m.c4rate = C4_PAL_RATE;
    m.volbase = 0x40;
    m.gvolbase = 0x40;
    m.gvol = m.gvolbase;
    m.vol_table = None;
    m.quirk = 0;
    m.flow_mode = FLOW_MODE_GENERIC;
    m.read_event_type = READ_EVENT_MOD;
    m.period_type = PERIOD_AMIGA;
    m.compare_vblank = 0;
    m.comment = None;
    m.scan_cnt = Vec::new();
    m.midi = None;

    /* Set defaults */
    m.mod_.pat = 0;
    m.mod_.trk = 0;
    m.mod_.chn = 4;
    m.mod_.ins = 0;
    m.mod_.smp = 0;
    m.mod_.spd = 6;
    m.mod_.bpm = 125;
    m.mod_.len = 0;
    m.mod_.rst = 0;

    m.extra = ModuleExtras::None;

    m.time_factor = DEFAULT_TIME_FACTOR;

    for i in 0..64 {
        let pan = (((i + 1) / 2) % 2) * 0xff;
        m.mod_.xxc[i as usize].pan = 0x80 + (pan - 0x80) * m.defpan / 100;
        m.mod_.xxc[i as usize].vol = 0x40;
        m.mod_.xxc[i as usize].flg = 0;
    }
}

/// Translation of `libxmp_load_epilogue()`.
pub(crate) fn libxmp_load_epilogue(ctx: &mut ContextData) {
    let m = &mut ctx.m;
    let mod_ = &mut m.mod_;

    mod_.gvl = m.gvol;

    /* Sanity check for module parameters */
    clamp(&mut mod_.len, 0, XMP_MAX_MOD_LENGTH as i32);
    clamp(&mut mod_.pat, 0, 257); /* some formats have an extra pattern */
    clamp(&mut mod_.ins, 0, 255);
    clamp(&mut mod_.smp, 0, MAX_SAMPLES);
    clamp(&mut mod_.chn, 0, XMP_MAX_CHANNELS as i32);

    /* Fix cases where the restart value is invalid e.g. kc_fall8.xm
     * from http://aminet.net/mods/mvp/mvp_0002.lha (reported by
     * Ralf Hoffmann <ralf@boomerangsworld.de>)
     */
    if mod_.rst >= mod_.len {
        mod_.rst = 0;
    }

    /* Sanity check for tempo and BPM */
    if mod_.spd <= 0 || mod_.spd > 255 {
        mod_.spd = 6;
    }
    clamp(&mut mod_.bpm, XMP_MIN_BPM, 1000);

    /* Set appropriate values for instrument volumes and subinstrument
     * global volumes when QUIRK_INSVOL is not set, to keep volume values
     * consistent if the user inspects struct xmp_module. We can later
     * set volumes in the loaders and eliminate the quirk.
     */
    let ins = (mod_.ins as usize).min(mod_.xxi.len());
    for i in 0..ins {
        let xxi = &mut mod_.xxi[i];
        if !m.quirk & QUIRK_INSVOL != 0 {
            xxi.vol = m.volbase;
        }
        let nsm = (xxi.nsm.max(0) as usize).min(xxi.sub.len());
        for j in 0..nsm {
            if !m.quirk & QUIRK_INSVOL != 0 {
                xxi.sub[j].gvl = m.volbase;
            }
        }
    }

    /* Sanity check for envelopes
     */
    for i in 0..ins {
        let xxi = &mut mod_.xxi[i];
        check_envelope(&mut xxi.aei);
        check_envelope(&mut xxi.fei);
        check_envelope(&mut xxi.pei);
        clamp_volume_envelope(m.volbase, &mut xxi.aei);
    }

    /* TODO: there's no unintrusive and clean way to get this struct into
     * libxmp_load_sample currently, so bound these fields here for now. */
    let smp = (mod_.smp as usize).min(mod_.xxs.len()).min(m.xtra.len());
    for i in 0..smp {
        let xxs = &mut mod_.xxs[i];
        let xtra = &mut m.xtra[i];
        if xtra.sus < 0 {
            xtra.sus = 0;
        }
        if xtra.sue > xxs.len {
            xtra.sue = xxs.len;
        }
        if xtra.sus >= xxs.len || xtra.sus >= xtra.sue {
            xtra.sus = 0;
            xtra.sue = 0;
            xxs.flg &= !(XMP_SAMPLE_SLOOP | XMP_SAMPLE_SLOOP_BIDIR);
        }
    }

    let p = &mut ctx.p;
    p.filter = 0;
    p.mode = XMP_MODE_AUTO;
    p.flags = p.player_flags;
    p.scan_time_factor = ctx.m.time_factor;
    module_quirks(ctx);
    libxmp_set_player_mode(ctx);
}

/// Translation of `libxmp_prepare_scan()`.
pub(crate) fn libxmp_prepare_scan(ctx: &mut ContextData) -> i32 {
    let m = &mut ctx.m;
    let mod_ = &mut m.mod_;

    if mod_.xxp.is_none() || mod_.xxt.is_none() {
        return -XMP_ERROR_LOAD;
    }
    let mut ord = 0;
    while ord < mod_.len && mod_.xxo[ord as usize] as i32 >= mod_.pat {
        ord += 1;
    }

    if ord >= mod_.len {
        mod_.len = 0;
        return 0;
    }

    let Ok(mut scan_cnt) = try_vec(mod_.len as usize, Vec::new()) else {
        return -XMP_ERROR_SYSTEM;
    };

    for i in 0..mod_.len as usize {
        let pat_idx = mod_.xxo[i] as i32;

        /* Add pattern if referenced in orders */
        if pat_idx < mod_.pat
            && mod_
                .xxp
                .as_ref()
                .is_some_and(|xxp| xxp[pat_idx as usize].is_none())
            && libxmp_alloc_pattern(mod_, pat_idx) < 0
        {
            m.scan_cnt = scan_cnt;
            return -XMP_ERROR_SYSTEM;
        }

        let rows = if pat_idx >= mod_.pat {
            0
        } else {
            mod_.pattern(pat_idx).rows
        };
        let Ok(cnt) = try_vec(if rows != 0 { rows as usize } else { 1 }, 0u8) else {
            m.scan_cnt = scan_cnt;
            return -XMP_ERROR_SYSTEM;
        };
        scan_cnt[i] = cnt;
    }

    m.scan_cnt = scan_cnt;
    0
}

/// Translation of `libxmp_free_scan()`.
pub(crate) fn libxmp_free_scan(ctx: &mut ContextData) {
    ctx.m.scan_cnt = Vec::new();
    ctx.p.scan = Vec::new();
}

/* Process player personality flags */
/// Translation of `libxmp_set_player_mode()`.
pub(crate) fn libxmp_set_player_mode(ctx: &mut ContextData) -> i32 {
    let p = &mut ctx.p;
    let m = &mut ctx.m;

    match p.mode {
        XMP_MODE_AUTO => {}
        XMP_MODE_MOD => {
            m.c4rate = C4_PAL_RATE;
            m.quirk = 0;
            m.flow_mode = FLOW_MODE_GENERIC;
            m.read_event_type = READ_EVENT_MOD;
            m.period_type = PERIOD_AMIGA;
        }
        XMP_MODE_NOISETRACKER => {
            m.c4rate = C4_PAL_RATE;
            m.quirk = QUIRK_NOBPM;
            m.flow_mode = FLOW_MODE_GENERIC;
            m.read_event_type = READ_EVENT_MOD;
            m.period_type = PERIOD_MODRNG;
        }
        XMP_MODE_PROTRACKER => {
            m.c4rate = C4_PAL_RATE;
            m.quirk = QUIRK_PROTRACK;
            m.flow_mode = FLOW_MODE_GENERIC;
            m.read_event_type = READ_EVENT_MOD;
            m.period_type = PERIOD_MODRNG;
        }
        XMP_MODE_S3M => {
            let q = m.quirk & (QUIRK_VSALL | QUIRK_ARPMEM);
            m.c4rate = C4_NTSC_RATE;
            m.quirk = QUIRKS_ST3 | q;
            m.flow_mode = FLOW_MODE_ST3_321;
            m.read_event_type = READ_EVENT_ST3;
        }
        XMP_MODE_ST3 => {
            let q = m.quirk & (QUIRK_VSALL | QUIRK_ARPMEM);
            m.c4rate = C4_NTSC_RATE;
            m.quirk = QUIRKS_ST3 | QUIRK_ST3BUGS | q;
            m.flow_mode = FLOW_MODE_ST3_321;
            m.read_event_type = READ_EVENT_ST3;
        }
        XMP_MODE_ST3GUS => {
            let q = m.quirk & (QUIRK_VSALL | QUIRK_ARPMEM);
            m.c4rate = C4_NTSC_RATE;
            m.quirk = QUIRKS_ST3 | QUIRK_ST3BUGS | q;
            m.quirk &= !QUIRK_RSTCHN;
            m.flow_mode = FLOW_MODE_ST3_321;
            m.read_event_type = READ_EVENT_ST3;
        }
        XMP_MODE_XM => {
            m.c4rate = C4_NTSC_RATE;
            m.quirk = QUIRKS_FT2;
            m.flow_mode = FLOW_MODE_GENERIC;
            m.read_event_type = READ_EVENT_FT2;
        }
        XMP_MODE_FT2 => {
            m.c4rate = C4_NTSC_RATE;
            m.quirk = QUIRKS_FT2 | QUIRK_FT2BUGS;
            m.flow_mode = FLOW_MODE_GENERIC;
            m.read_event_type = READ_EVENT_FT2;
        }
        XMP_MODE_IT => {
            m.c4rate = C4_NTSC_RATE;
            m.quirk = QUIRKS_IT | QUIRK_VIBHALF | QUIRK_VIBINV;
            m.flow_mode = FLOW_MODE_IT_210;
            m.read_event_type = READ_EVENT_IT;
        }
        XMP_MODE_ITSMP => {
            m.c4rate = C4_NTSC_RATE;
            m.quirk = QUIRKS_IT | QUIRK_VIBHALF | QUIRK_VIBINV;
            m.quirk &= !(QUIRK_VIRTUAL | QUIRK_RSTCHN);
            m.flow_mode = FLOW_MODE_IT_210;
            m.read_event_type = READ_EVENT_IT;
        }
        _ => return -1,
    }

    if p.mode != XMP_MODE_AUTO {
        m.compare_vblank = 0;
    }

    0
}
