// Rust translation of src/med_extras.c and src/med_extras.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2025 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! MED/OctaMED synth instruments (volume and waveform sequences) and
//! hold/decay.

use super::common::*;
use super::effects::*;
use super::extras::{ChannelExtras, InstrumentExtras, ModuleExtras};
use super::loaders::med::*;
use super::player::*;
use super::virt::libxmp_virt_setsmp;
use super::*;

/// Translation of `struct med_instrument_extras`.
#[derive(Clone, Debug, Default)]
pub(crate) struct MedInstrumentExtras {
    pub vts: i32,   /* Volume table speed */
    pub wts: i32,   /* Waveform table speed */
    pub vtlen: i32, /* Volume table length */
    pub wtlen: i32, /* Waveform table length */
    pub hold: i32,
    pub decay: i32,
}

/// Translation of `struct med_channel_extras`.
#[derive(Clone, Debug, Default)]
pub(crate) struct MedChannelExtras {
    pub vp: i32,             /* MED synth volume table pointer */
    pub vv: i32,             /* MED synth volume slide value */
    pub vs: i32,             /* MED synth volume speed */
    pub vc: i32,             /* MED synth volume speed counter */
    pub vw: i32,             /* MED synth volume wait counter */
    pub wp: i32,             /* MED synth waveform table pointer */
    pub wv: i32,             /* MED synth waveform slide value */
    pub ws: i32,             /* MED synth waveform speed */
    pub wc: i32,             /* MED synth waveform speed counter */
    pub ww: i32,             /* MED synth waveform wait counter */
    pub period: i32,         /* MED synth period for RES */
    pub arp: i32,            /* MED synth arpeggio start */
    pub aidx: i32,           /* MED synth arpeggio index */
    pub vwf: i32,            /* MED synth vibrato waveform */
    pub vib_depth: i32,      /* MED synth vibrato depth */
    pub vib_speed: i32,      /* MED synth vibrato speed */
    pub vib_idx: i32,        /* MED synth vibrato index */
    pub vib_wf: i32,         /* MED synth vibrato waveform */
    pub volume: i32,         /* MED synth note volume */
    pub hold_active: i32,    /* MED hold is active? (hack) */
    pub hold_sustained: i32, /* MED hold currently sustained by an event? */
    pub hold_count: i32,     /* MED hold remaining ticks counter */
    pub decay_value: i32,    /* MED decay value */
    pub env_wav: i32,        /* MED synth volume envelope waveform */
    pub env_idx: i32,        /* MED synth volume envelope index */
    pub flags: i32,          /* flags */
}

pub(crate) const MED_SYNTH_ENV_LOOP: i32 = 1 << 0;

/// Translation of `struct med_module_extras`.
#[derive(Clone, Debug, Default)]
pub(crate) struct MedModuleExtras {
    pub vol_table: Vec<Option<Vec<u8>>>, /* MED volume sequence table */
    pub wav_table: Vec<Option<Vec<u8>>>, /* MED waveform sequence table */
    pub tracker_version: i32,            /* Detected tracker (see mmd_tracker_version) */
}

/// `MED_INSTRUMENT_EXTRAS(xxi)`, if it has them.
pub(crate) fn med_instrument_extras(xxi: &XmpInstrument) -> Option<&MedInstrumentExtras> {
    match &xxi.extra {
        InstrumentExtras::Med(e) => Some(e),
        _ => None,
    }
}

/// `MED_INSTRUMENT_EXTRAS(xxi)`, mutably.
pub(crate) fn med_instrument_extras_mut(
    xxi: &mut XmpInstrument,
) -> Option<&mut MedInstrumentExtras> {
    match &mut xxi.extra {
        InstrumentExtras::Med(e) => Some(e),
        _ => None,
    }
}

/// `MED_CHANNEL_EXTRAS(xc)`, if it has them.
pub(crate) fn med_channel_extras(xc: &mut ChannelData) -> Option<&mut MedChannelExtras> {
    match &mut xc.extra {
        ChannelExtras::Med(e) => Some(e),
        _ => None,
    }
}

/// `MED_MODULE_EXTRAS(m)`, if it has them.
pub(crate) fn med_module_extras(m: &ModuleData) -> Option<&MedModuleExtras> {
    match &m.extra {
        ModuleExtras::Med(e) => Some(e),
        _ => None,
    }
}

/// `MED_MODULE_EXTRAS(m)`, mutably.
pub(crate) fn med_module_extras_mut(m: &mut ModuleData) -> Option<&mut MedModuleExtras> {
    match &mut m.extra {
        ModuleExtras::Med(e) => Some(e),
        _ => None,
    }
}

/* Commands in the volume and waveform sequence table:
 *
 *	Cmd	Vol	Wave	Action
 *
 *	0xff	    END		End sequence
 *	0xfe	    JMP		Jump
 *	0xfd	 -	ARE	End arpeggio definition
 *	0xfc	 -	ARP	Begin arpeggio definition
 *	0xfb	    HLT		Halt
 *	0xfa	JWS	JVS	Jump waveform/volume sequence
 *	0xf9	     -		-
 *	0xf8	     -		-
 *	0xf7	 -	VWF	Set vibrato waveform
 *	0xf6	EST	RES	?/reset pitch
 *	0xf5	EN2	VBS	Looping envelope/set vibrato speed
 *	0xf4	EN1	VBD	One shot envelope/set vibrato depth
 *	0xf3	    CHU		Change volume/pitch up speed
 *	0xf2	    CHD		Change volume/pitch down speed
 *	0xf1	    WAI		Wait
 *	0xf0	    SPD		Set speed
 */

static SINE: [i32; 32] = [
    0, 49, 97, 141, 180, 212, 235, 250, 255, 250, 235, 212, 180, 141, 97, 49, 0, -49, -97, -141,
    -180, -212, -235, -250, -255, -250, -235, -212, -180, -141, -97, -49,
];

/// The channel's MED extras (which a MED module's channels have).
fn ce_of(ctx: &mut ContextData, chn: usize) -> &mut MedChannelExtras {
    med_channel_extras(&mut ctx.p.xc_data[chn]).unwrap()
}

/// Translation of `libxmp_med_change_period()`.
pub(crate) fn libxmp_med_change_period(ctx: &mut ContextData, chn: usize) -> i32 {
    let ce = ce_of(ctx, chn);

    /* Vibrato */

    /* FIXME: always using sine waveform */

    let vib = (SINE[(ce.vib_idx >> 5) as usize] * ce.vib_depth) >> 10;
    ce.vib_idx += ce.vib_speed;
    ce.vib_idx %= 32 << 5;

    vib
}

/// Translation of `libxmp_med_linear_bend()`.
pub(crate) fn libxmp_med_linear_bend(ctx: &mut ContextData, chn: usize) -> i32 {
    let ins = ctx.p.xc_data[chn].ins as usize;
    /* (upstream only reads the instrument extras, in ARP(), when an
     * arpeggio is set; MED4's sampled instruments have none) */
    let wtlen = ctx
        .m
        .mod_
        .xxi
        .get(ins)
        .and_then(med_instrument_extras)
        .map_or(0, |ie| ie.wtlen);
    let me = med_module_extras(&ctx.m).unwrap();
    let wav = me.wav_table[ins].as_deref();
    let ce = med_channel_extras(&mut ctx.p.xc_data[chn]).unwrap();

    // ARP(idx)
    let arp_at = |idx: i32| -> i32 {
        if idx < wtlen {
            wav.unwrap()[idx as usize] as i32
        } else {
            0xfd
        }
    };

    /* Arpeggio */

    if ce.arp == 0 {
        return 0;
    }

    if arp_at(ce.arp) == 0xfd {
        /* empty arpeggio */
        return 0;
    }

    let mut arp = arp_at(ce.aidx);
    if arp == 0xfd {
        ce.aidx = ce.arp;
        arp = arp_at(ce.aidx);
    }
    ce.aidx += 1;

    (100 << 7) * arp
}

/* Hold/decay support functions.
 *
 * Hold/decay should only occur if the channel has a configured hold
 * value (from instrument or an initial hold/decay command). If the
 * decay is 0, the note volume is immediately set to 0 when hold ends.
 *
 * "Hold symbols" (instrument number with no note) sustain hold on
 * occuring on the line PRIOR to them. Hold countdown begins on the
 * last line with a hold symbol if present, otherwise immediately.
 *
 * TODO: MED bug: note delay doesn't delay the hold? It seems to play a
 * fragment of the prior note before the new note starts in some cases.
 * TODO: pre-OctaMED 3.00 FF2 note delay breaks hold entirely?
 */

/// Translation of `libxmp_med_set_hold_decay()`.
pub(crate) fn libxmp_med_set_hold_decay(xc: &mut ChannelData, mut hold: i32, mut decay: i32) {
    let Some(ce) = med_channel_extras(xc) else {
        return;
    };
    if hold <= 0 {
        hold = -1;
        decay = -1;
    }
    ce.hold_active = (hold > 0) as i32;
    ce.hold_count = hold;
    ce.decay_value = decay;
}

/* OctaMED 3.00 onward: each individual retrigger increases the hold counter
 * by the CURRENT TICK NUMBER if decay hasn't started, allowing very long holds.
 * Presumably this was intended to be the retrigger delay instead.
 *
 * Example: Aftab Hussain/deep cover.mmd3 pos 36/block 30 ch.2 rows 60-64
 * uses retrigger hold extension for a snare roll, but it doesn't rely
 * on the extra long hold length bug.
 */
/// Translation of `libxmp_med_hold_retrigger()`.
pub(crate) fn libxmp_med_hold_retrigger(ctx: &mut ContextData, chn: usize) {
    let frame = ctx.p.frame;
    let tracker_version = med_module_extras(&ctx.m).map(|me| me.tracker_version);
    let Some(ce) = med_channel_extras(&mut ctx.p.xc_data[chn]) else {
        return;
    };
    if tracker_version.unwrap() >= MED_VER_OCTAMED_300 && ce.hold_active != 0 {
        ce.hold_count += frame;
    }
}

/**
 * Hack to get the next event in the pattern (for implementing hold symbols).
 * This may not work as intended for injected events.
 */
/// Translation of `get_next_event()`.
fn get_next_event(ctx: &ContextData, chn: i32) -> Option<XmpEvent> {
    let p = &ctx.p;
    let mod_ = &ctx.m.mod_;
    let pat = mod_.xxo[p.ord as usize] as i32;
    let num_rows = mod_.track(mod_.track_num(pat, chn)).rows;

    if p.row + 1 >= num_rows {
        return None;
    }
    Some(*mod_.event(pat, chn, p.row + 1))
}

/* Should be called from read_event_med only. */
/// Translation of `libxmp_med_check_hold_symbol()`.
pub(crate) fn libxmp_med_check_hold_symbol(ctx: &mut ContextData, chn: usize) {
    ce_of(ctx, chn).hold_sustained = 0;

    /* Hold/decay added in MED 3.00 / OctaMED 1.00 */
    if med_module_extras(&ctx.m).unwrap().tracker_version < MED_VER_300 {
        return;
    }
    if ce_of(ctx, chn).hold_count <= 0 {
        return;
    }
    let Some(e) = get_next_event(ctx, chn as i32) else {
        return;
    };

    /* No note + ins -> sustain hold. */
    if !is_valid_note(e.note as i32 - 1) && e.ins != 0 {
        ce_of(ctx, chn).hold_sustained = 1;
        return;
    }

    /* Note + toneporta (3xx only, NOT 5xy) -> sustain hold. */
    if is_valid_note(e.note as i32 - 1) && (e.fxt == FX_TONEPORTA || e.f2t == FX_TONEPORTA) {
        ce_of(ctx, chn).hold_sustained = 1;
    }
}

/// Translation of `med_tick_hold_decay()`.
fn med_tick_hold_decay(ctx: &mut ContextData, chn: usize) {
    let volbase = ctx.m.volbase;
    let tracker_version = med_module_extras(&ctx.m).unwrap().tracker_version;
    let frame = ctx.p.frame;
    let speed = ctx.p.speed;
    let xc = &mut ctx.p.xc_data[chn];
    let mut volume = xc.volume;
    let ce = med_channel_extras(xc).unwrap();

    if ce.hold_count == 0 {
        /* Decay 0 = instant decay. */
        let dec = if ce.decay_value != 0 {
            ce.decay_value
        } else {
            volume
        };

        volume -= dec;
        clamp(&mut volume, 0, volbase);

        if volume == 0 {
            /* End hold/decay once volume 0 is reached. */
            ce.hold_count = -1;
            ce.decay_value = -1;
        }
        /* Hold can't be sustained/increased after decay starts(?). */
        ce.hold_active = 0;
    }

    /* From OctaMED 3.00 through Soundstudio v1, pattern delay (1Exx) on
     * a line with hold sustained will only sustain the hold for the first
     * execution of the row. In Soundstudio v2, the whole row sustains. */
    if ce.hold_sustained != 0 && tracker_version < MED_VER_OCTAMED_SS_2 && frame >= speed {
        ce.hold_sustained = 0;
    }

    /* Hold is sustained if the NEXT row contains a sustain event.
     * MED probably actually decrements hold after effects processing;
     * using ce->hold_active as a hack to work around this. */
    if ce.hold_count > 0 && ce.hold_sustained == 0 {
        ce.hold_count -= 1;
    }
    xc.volume = volume;
}

/// Translation of `libxmp_med_play_extras()`.
pub(crate) fn libxmp_med_play_extras(ctx: &mut ContextData, chn: usize) {
    let mut jws = 0;
    let mut jvs = 0;

    if !matches!(ctx.m.extra, ModuleExtras::Med(_)) {
        return;
    }

    let ins = ctx.p.xc_data[chn].ins as usize;

    /* Handle hold/decay */

    med_tick_hold_decay(ctx, chn);

    /* Handle synth */

    let has_tables = {
        let me = med_module_extras(&ctx.m).unwrap();
        me.vol_table[ins].is_some() && me.wav_table[ins].is_some()
    };
    if !has_tables {
        ce_of(ctx, chn).volume = 64; /* we need this in extras_get_volume() */
        return;
    }

    let ie = med_instrument_extras(&ctx.m.mod_.xxi[ins]).unwrap().clone();
    let frame = ctx.p.frame;
    {
        let xc = &mut ctx.p.xc_data[chn];
        if frame == 0 && xc.test(NEW_NOTE) {
            let period = xc.period;
            let new_ins = xc.test(NEW_INS);
            let ce = med_channel_extras(xc).unwrap();
            ce.period = period as i32;
            if new_ins {
                ce.arp = 0;
                ce.aidx = 0;
                ce.vp = 0;
                ce.vc = 0;
                ce.vw = 0;
                ce.wp = 0;
                ce.wc = 0;
                ce.ww = 0;
                ce.env_wav = -1;
                ce.env_idx = 0;
                ce.flags &= !MED_SYNTH_ENV_LOOP;
                ce.vv = 0;
                ce.wv = 0;
                ce.vs = ie.vts;
                ce.ws = ie.wts;
            }
        }
    }

    let vol_table: Vec<u8> = med_module_extras(&ctx.m).unwrap().vol_table[ins]
        .clone()
        .unwrap();
    let wav_table: Vec<u8> = med_module_extras(&ctx.m).unwrap().wav_table[ins]
        .clone()
        .unwrap();

    // VT, WT
    fn vt(ce: &mut MedChannelExtras, ie: &MedInstrumentExtras, t: &[u8]) -> i32 {
        if ce.vp >= 0 && ce.vp < ie.vtlen {
            let v = t[ce.vp as usize] as i32;
            ce.vp += 1;
            v
        } else {
            0xff
        }
    }
    fn wt(ce: &mut MedChannelExtras, ie: &MedInstrumentExtras, t: &[u8]) -> i32 {
        if ce.wp >= 0 && ce.wp < ie.wtlen {
            let v = t[ce.wp as usize] as i32;
            ce.wp += 1;
            v
        } else {
            0xff
        }
    }

    let run_vol = {
        let ce = ce_of(ctx, chn);
        let r = ce.vs > 0 && ce.vc == 0;
        if ce.vs > 0 {
            ce.vc = ce.vc.wrapping_sub(1);
        }
        r
    };
    if run_vol {
        let ce = ce_of(ctx, chn);
        ce.vc = ce.vs - 1;

        let mut skip_vol = false;
        if ce.vw > 0 {
            ce.vw -= 1;
            skip_vol = true;
        }

        if !skip_vol {
            let mut loop_ = 0;
            jws = 0;

            /* Volume commands */

            loop {
                // next_vt:
                let b = vt(ce, &ie, &vol_table);
                match b {
                    0xff | 0xfb => {
                        /* END */
                        /* HLT */
                        ce.vp -= 1;
                    }
                    0xfe => {
                        /* JMP */
                        if loop_ != 0 {
                            /* avoid infinite loop */
                            break;
                        }
                        let temp = vt(ce, &ie, &vol_table);
                        ce.vp = temp;
                        loop_ = 1;
                        continue;
                    }
                    0xfa => {
                        /* JWS */
                        jws = vt(ce, &ie, &vol_table);
                    }
                    0xf5 => {
                        /* EN2 */
                        ce.env_wav = vt(ce, &ie, &vol_table);
                        ce.flags |= MED_SYNTH_ENV_LOOP;
                    }
                    0xf4 => {
                        /* EN1 */
                        ce.env_wav = vt(ce, &ie, &vol_table);
                    }
                    0xf3 => {
                        /* CHU */
                        ce.vv = vt(ce, &ie, &vol_table);
                    }
                    0xf2 => {
                        /* CHD */
                        ce.vv = -vt(ce, &ie, &vol_table);
                    }
                    0xf1 => {
                        /* WAI */
                        ce.vw = vt(ce, &ie, &vol_table);
                    }
                    0xf0 => {
                        /* SPD */
                        ce.vs = vt(ce, &ie, &vol_table);
                    }
                    _ => {
                        if (0x00..=0x40).contains(&b) {
                            ce.volume = b;
                        }
                    }
                }
                break;
            }
        }

        // skip_vol:
        /* volume envelope */
        let (env_wav, env_idx) = {
            let ce = ce_of(ctx, chn);
            (ce.env_wav, ce.env_idx)
        };
        let xxi = &ctx.m.mod_.xxi[ins];
        if env_wav >= 0 && env_wav < xxi.nsm {
            let sid = xxi.sub[env_wav as usize].sid;
            let xxs = &ctx.m.mod_.xxs[sid as usize];
            if xxs.len == 0x80 {
                /* sanity check */
                let d = xxs.data.as_ref().unwrap()[SAMPLE_GUARD + env_idx as usize] as i8 as i32;
                let ce = ce_of(ctx, chn);
                ce.volume = (d + 0x80) >> 2;
                ce.env_idx += 1;

                if ce.env_idx >= 0x80 {
                    if !ce.flags & MED_SYNTH_ENV_LOOP != 0 {
                        ce.env_wav = -1;
                    }
                    ce.env_idx = 0;
                }
            }
        }

        let ce = ce_of(ctx, chn);
        ce.volume += ce.vv;
        clamp(&mut ce.volume, 0, 64);
    }

    let run_wav = {
        let ce = ce_of(ctx, chn);
        let r = ce.ws > 0 && ce.wc == 0;
        if ce.ws > 0 {
            ce.wc = ce.wc.wrapping_sub(1);
        }
        r
    };
    if run_wav {
        let ce = ce_of(ctx, chn);
        ce.wc = ce.ws - 1;

        let mut skip_wav = false;
        if ce.ww > 0 {
            ce.ww -= 1;
            skip_wav = true;
        }

        if !skip_wav {
            let mut loop_ = 0;
            jvs = 0;

            /* Waveform commands */

            loop {
                // next_wt:
                let ce = ce_of(ctx, chn);
                let mut b = wt(ce, &ie, &wav_table);
                match b {
                    0xff | 0xfb => {
                        /* END */
                        /* HLT */
                        ce.wp -= 1;
                    }
                    0xfe => {
                        /* JMP */
                        if loop_ != 0 {
                            /* avoid infinite loop */
                            break;
                        }
                        let temp = wt(ce, &ie, &wav_table);
                        if temp == 0xff {
                            /* handle JMP END case */
                            ce.wp -= 1; /* see lepeltheme ins 0x02 */
                            break;
                        }
                        ce.wp = temp;
                        loop_ = 1;
                        continue;
                    }
                    0xfd => { /* ARE */ }
                    0xfc => {
                        /* ARP */
                        ce.arp = ce.wp;
                        ce.aidx = ce.wp;
                        ce.wp += 1;
                        while b != 0xfd && b != 0xff {
                            b = wt(ce, &ie, &wav_table);
                        }
                    }
                    0xfa => {
                        /* JVS */
                        jvs = wt(ce, &ie, &wav_table);
                    }
                    0xf7 => {
                        /* VWF */
                        ce.vwf = wt(ce, &ie, &wav_table);
                    }
                    0xf6 => {
                        /* RES */
                        let period = ce.period;
                        ctx.p.xc_data[chn].period = period as f64;
                    }
                    0xf5 => {
                        /* VBS */
                        ce.vib_speed = wt(ce, &ie, &wav_table);
                    }
                    0xf4 => {
                        /* VBD */
                        ce.vib_depth = wt(ce, &ie, &wav_table);
                    }
                    0xf3 => {
                        /* CHU */
                        ce.wv = -wt(ce, &ie, &wav_table);
                    }
                    0xf2 => {
                        /* CHD */
                        ce.wv = wt(ce, &ie, &wav_table);
                    }
                    0xf1 => {
                        /* WAI */
                        ce.ww = wt(ce, &ie, &wav_table);
                    }
                    0xf0 => {
                        /* SPD */
                        ce.ws = wt(ce, &ie, &wav_table);
                    }
                    _ => {
                        let xxi = &ctx.m.mod_.xxi[ins];
                        if b < xxi.nsm && xxi.sub[b as usize].sid != ctx.p.xc_data[chn].smp {
                            let smp = xxi.sub[b as usize].sid;
                            ctx.p.xc_data[chn].smp = smp;
                            libxmp_virt_setsmp(ctx, chn as i32, smp);
                        }
                    }
                }
                break;
            }
        }

        // skip_wav:
        let wv = ce_of(ctx, chn).wv;
        ctx.p.xc_data[chn].period += wv as f64;
    }

    if jws != 0 {
        ce_of(ctx, chn).wp = jws;
        /* jws = 0; */
    }

    if jvs != 0 {
        ce_of(ctx, chn).vp = jvs;
        /* jvs = 0; */
    }
}

/// Translation of `libxmp_med_new_instrument_extras()`.
pub(crate) fn libxmp_med_new_instrument_extras(xxi: &mut XmpInstrument) -> i32 {
    xxi.extra = InstrumentExtras::Med(Box::default());

    0
}

/// Translation of `libxmp_med_new_channel_extras()`.
pub(crate) fn libxmp_med_new_channel_extras(xc: &mut ChannelData) -> i32 {
    xc.extra = ChannelExtras::Med(Box::new(MedChannelExtras {
        hold_count: -1,
        ..Default::default()
    }));

    0
}

/// Translation of `libxmp_med_reset_channel_extras()`.
pub(crate) fn libxmp_med_reset_channel_extras(xc: &mut ChannelData) {
    if let Some(ce) = med_channel_extras(xc) {
        *ce = MedChannelExtras::default();
    }
}

/// Translation of `libxmp_med_new_module_extras()`.
pub(crate) fn libxmp_med_new_module_extras(m: &mut ModuleData) -> i32 {
    let n = m.mod_.ins.max(0) as usize;
    let Ok(vol_table) = try_vec(n, None) else {
        return -1;
    };
    let Ok(wav_table) = try_vec(n, None) else {
        return -1;
    };
    m.extra = ModuleExtras::Med(Box::new(MedModuleExtras {
        vol_table,
        wav_table,
        tracker_version: 0,
    }));

    0
}

/// Translation of `libxmp_med_release_module_extras()`.
pub(crate) fn libxmp_med_release_module_extras(m: &mut ModuleData) {
    m.extra = ModuleExtras::None;
}

/// Translation of `libxmp_med_extras_process_fx()`.
pub(crate) fn libxmp_med_extras_process_fx(
    ctx: &mut ContextData,
    chn: usize,
    note: u8,
    ins: u8,
    fxt: u8,
    fxp: u8,
    _fnum: i32,
) {
    if fxt == FX_MED_HOLD {
        /* Command 08 is only valid beside a note+ins. */
        if is_valid_note(note as i32 - 1) && is_valid_instrument(&ctx.m.mod_, ins as i32 - 1) {
            libxmp_med_set_hold_decay(&mut ctx.p.xc_data[chn], lsn(fxp), msn(fxp));
        }
    }
}
