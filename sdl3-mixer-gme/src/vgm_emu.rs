// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2003-2006 Shay Green. This module is free software; you
can redistribute it and/or modify it under the terms of the GNU Lesser
General Public License as published by the Free Software Foundation; either
version 2.1 of the License, or (at your option) any later version. This
module is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
details. You should have received a copy of the GNU Lesser General Public
License along with this module; if not, write to the Free Software Foundation,
Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA */
// Modified 2026-10-18: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! Sega Master System/Mark III, Sega Genesis/Mega Drive, BBC Micro VGM music file emulator
//! (`Vgm_Emu.h`, `Vgm_Emu.cpp`).

use sdl3::stdlib::math::floor;

use crate::blargg_common::{get_le16, get_le32, try_resize, BlarggErr};
use crate::blip_buffer::{BlipEq, BlipTime, BufId};
use crate::classic_emu::{
    classic_mute_voices_, classic_play_, classic_set_equalizer_, classic_set_sample_rate_,
    classic_start_track_, setup_buffer, ClassicEmu, ClassicEmuState, NOISE_TYPE, WAVE_TYPE,
};
use crate::data_reader::DataReader;
use crate::dual_resampler::DualResampler;
use crate::gme_file::{GmeTypeT, TrackInfo, GME_MAX_FIELD, GME_WRONG_FILE_TYPE};
use crate::music_emu::{
    gme_info_overrides, make_equalizer, Equalizer, MusicEmu, MusicEmuState, Sample,
};
use crate::sms_apu;
use crate::vgm_emu_impl::{psg_bufs, VgmEmuImpl, BLIP_BUF, BLIP_TIME_BITS, FM_TIME_BITS};

const FM_GAIN: f64 = 3.0; // FM emulators are internally quieter to avoid 16-bit overflow
const ROLLOFF: f64 = 0.990;
const OVERSAMPLE_FACTOR: f64 = 1.0;

/// VGM header format
pub const HEADER_SIZE: usize = 0x40;

/// The fields of the VGM `header_t`, as offsets into its bytes.
pub mod header {
    pub const TAG: usize = 0x00; // [4]
    pub const DATA_SIZE: usize = 0x04; // [4]
    pub const VERSION: usize = 0x08; // [4]
    pub const PSG_RATE: usize = 0x0C; // [4]
    pub const YM2413_RATE: usize = 0x10; // [4]
    pub const GD3_OFFSET: usize = 0x14; // [4]
    pub const TRACK_DURATION: usize = 0x18; // [4]
    pub const LOOP_OFFSET: usize = 0x1C; // [4]
    pub const LOOP_DURATION: usize = 0x20; // [4]
    pub const FRAME_RATE: usize = 0x24; // [4]
    pub const NOISE_FEEDBACK: usize = 0x28; // [2]
    pub const NOISE_WIDTH: usize = 0x2A;
    pub const UNUSED1: usize = 0x2B;
    pub const YM2612_RATE: usize = 0x2C; // [4]
    pub const YM2151_RATE: usize = 0x30; // [4]
    pub const DATA_OFFSET: usize = 0x34; // [4]
    pub const UNUSED2: usize = 0x38; // [8]
}

static VOICE_TYPES: [i32; 8] = [
    WAVE_TYPE | 1,
    WAVE_TYPE,
    WAVE_TYPE | 2,
    NOISE_TYPE,
    0,
    0,
    0,
    0,
];

static FM_NAMES: [&str; 8] = ["FM 1", "FM 2", "FM 3", "FM 4", "FM 5", "FM 6", "PCM", "PSG"];
static PSG_NAMES: [&str; 4] = ["Square 1", "Square 2", "Square 3", "Noise"];

/// Emulates VGM music using SN76489/SN76496 PSG, YM2612, and YM2413 FM sound chips.
/// Supports custom sound buffer and frequency equalization when VGM uses just the PSG.
/// FM sound chips can be run at their proper rates, or slightly higher to reduce
/// aliasing on high notes. Currently YM2413 support requires that you supply a
/// YM2413 sound chip emulator. I can provide one I've modified to work with the library.
///
/// (`Vgm_Emu`: its fields, `Vgm_Emu_Impl`'s and `Classic_Emu`'s are in
/// `imp`, and `dual` is `Vgm_Emu_Impl`'s `Dual_Resampler` base.)
#[derive(Debug)]
pub struct VgmEmu {
    imp: VgmEmuImpl,
    dual: DualResampler,
}

impl Default for VgmEmu {
    fn default() -> Self {
        Self::new()
    }
}

// Track info

fn skip_gd3_str(data: &[u8], mut i: usize) -> usize {
    while data.len() - i >= 2 {
        i += 2;
        if (data[i - 2] | data[i - 1]) == 0 {
            break;
        }
    }
    i
}

fn get_gd3_str(data: &[u8], i: usize, field: &mut [u8]) -> usize {
    let mid = skip_gd3_str(data, i);
    let mut len: i32 = ((mid - i) / 2) as i32 - 1;
    if len > 0 {
        len = len.min(GME_MAX_FIELD as i32);
        let len = len as usize;
        field[len] = 0;
        for k in 0..len {
            field[k] = if data[i + k * 2 + 1] != 0 {
                b'?'
            } else {
                data[i + k * 2]
            }; // TODO: convert to utf-8
        }
    }
    mid
}

fn get_gd3_pair(data: &[u8], i: usize, field: &mut [u8]) -> usize {
    let i = get_gd3_str(data, i, field);
    skip_gd3_str(data, i)
}

/// `parse_gd3( in, end, out )`, on the bytes from `in` to `end`
fn parse_gd3(data: &[u8], out: &mut TrackInfo) {
    let mut i = 0;
    i = get_gd3_pair(data, i, &mut out.song);
    i = get_gd3_pair(data, i, &mut out.game);
    i = get_gd3_pair(data, i, &mut out.system);
    i = get_gd3_pair(data, i, &mut out.author);
    i = get_gd3_str(data, i, &mut out.copyright);
    i = get_gd3_pair(data, i, &mut out.dumper);
    /*in =*/
    get_gd3_str(data, i, &mut out.comment);
}

const GD3_HEADER_SIZE: i64 = 12;

/// `check_gd3_header( h, remain )` (`h` being read only when `remain` is
/// enough for the header)
fn check_gd3_header(h: &[u8], remain: i64) -> i64 {
    if remain < GD3_HEADER_SIZE {
        return 0;
    }
    if &h[..4] != b"Gd3 " {
        return 0;
    }
    if get_le32(&h[4..]) >= 0x200 {
        return 0;
    }

    let gd3_size = get_le32(&h[8..]) as i64;
    if gd3_size > remain - GD3_HEADER_SIZE {
        return 0;
    }

    gd3_size
}

/// `get_vgm_length()`
fn get_vgm_length(h: &[u8], out: &mut TrackInfo) {
    let length = (get_le32(&h[header::TRACK_DURATION..]).wrapping_mul(10) / 441) as i64;
    if length > 0 {
        let loop_ = get_le32(&h[header::LOOP_DURATION..]) as i64;
        if loop_ > 0 && get_le32(&h[header::LOOP_OFFSET..]) != 0 {
            out.loop_length = loop_ * 10 / 441;
            out.intro_length = length - out.loop_length;
        } else {
            out.length = length; // 1000 / 44100 (VGM files used 44100 as timebase)
            out.intro_length = length; // make it clear that track is no longer than length
            out.loop_length = 0;
        }
    }
}

fn check_vgm_header(h: &[u8]) -> BlarggErr {
    if &h[header::TAG..header::TAG + 4] != b"Vgm " {
        return Err(GME_WRONG_FILE_TYPE);
    }
    Ok(())
}

/// `Vgm_File`: the info-only VGM reader
pub(crate) struct VgmFile {
    state: MusicEmuState,
    h: [u8; HEADER_SIZE],
    gd3: Vec<u8>,
}

impl VgmFile {
    fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_VGM_TYPE_);
        VgmFile {
            state,
            h: [0; HEADER_SIZE],
            gd3: Vec::new(),
        }
    }
}

impl MusicEmu for VgmFile {
    gme_info_overrides!();

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        let file_size = input.remain();
        if file_size <= HEADER_SIZE as i64 {
            return Err(GME_WRONG_FILE_TYPE);
        }

        input.read(&mut self.h)?;
        check_vgm_header(&self.h)?;

        let gd3_offset = get_le32(&self.h[header::GD3_OFFSET..]).wrapping_sub(0x2C) as i64;
        let remain = file_size - HEADER_SIZE as i64 - gd3_offset;
        let mut gd3_h = [0u8; GD3_HEADER_SIZE as usize];
        if gd3_offset > 0 && remain >= GD3_HEADER_SIZE {
            input.skip(gd3_offset)?;
            input.read(&mut gd3_h)?;
            let gd3_size = check_gd3_header(&gd3_h, remain);
            if gd3_size != 0 {
                try_resize(&mut self.gd3, gd3_size as usize)?;
                input.read(&mut self.gd3)?;
            }
        }
        Ok(())
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        get_vgm_length(&self.h, out);
        if !self.gd3.is_empty() {
            parse_gd3(&self.gd3, out);
        }
        Ok(())
    }
}

impl std::fmt::Debug for VgmFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VgmFile").finish_non_exhaustive()
    }
}

fn new_vgm_emu() -> Box<dyn MusicEmu> {
    Box::new(VgmEmu::new())
}
fn new_vgm_file() -> Box<dyn MusicEmu> {
    Box::new(VgmFile::new())
}

pub(crate) static GME_VGM_TYPE_: GmeTypeT = GmeTypeT {
    system: "Sega SMS/Genesis",
    track_count: 1,
    new_emu: new_vgm_emu,
    new_info: new_vgm_file,
    extension_: "VGM",
    flags_: 1,
};

pub(crate) static GME_VGZ_TYPE_: GmeTypeT = GmeTypeT {
    system: "Sega SMS/Genesis",
    track_count: 1,
    new_emu: new_vgm_emu,
    new_info: new_vgm_file,
    extension_: "VGZ",
    flags_: 1,
};

impl VgmEmu {
    pub fn new() -> Self {
        let mut state = MusicEmuState::new();
        let mut classic = ClassicEmuState::new();
        state.set_type(&GME_VGM_TYPE_);
        classic.set_voice_types(&VOICE_TYPES);
        state.set_silence_lookahead(1); // tracks should already be trimmed

        let mut e = VgmEmu {
            imp: VgmEmuImpl::new(state, classic),
            dual: DualResampler::new(),
        };
        // (disable_oversampling_, psg_dual, psg_t6w28 and psg_rate start
        // false and 0)

        e.set_equalizer(&make_equalizer(-14.0, 80.0));
        e
    }

    /// True if custom buffer and custom equalization are supported
    /// TODO: move into Music_Emu and rename to something like supports_custom_buffer()
    pub fn is_classic_emu(&self) -> bool {
        !self.imp.uses_fm
    }

    /// Disable running FM chips at higher than normal rate. Will result in slightly
    /// more aliasing of high notes.
    pub fn disable_oversampling(&mut self, disable: bool) {
        self.imp.disable_oversampling_ = disable;
    }

    /// Header for currently loaded file
    pub fn header(&self) -> &[u8] {
        &self.imp.data()[..HEADER_SIZE]
    }

    /// `gd3_data()`: the GD3 tag (its header and data) of the loaded file
    pub fn gd3_data(&self) -> Option<&[u8]> {
        let e = &self.imp;
        let gd3_offset = get_le32(&self.header()[header::GD3_OFFSET..]).wrapping_sub(0x2C) as i64;
        if gd3_offset < 0 {
            return None;
        }

        let gd3 = HEADER_SIZE as i64 + gd3_offset;
        let remain = e.data_size as i64 - gd3;
        let h = if remain >= GD3_HEADER_SIZE {
            &e.data()[gd3 as usize..]
        } else {
            &[][..]
        };
        let gd3_size = check_gd3_header(h, remain);
        if gd3_size == 0 {
            return None;
        }

        Some(&h[..(gd3_size + GD3_HEADER_SIZE) as usize])
    }

    fn setup_fm(&mut self) -> BlarggErr {
        let e = &mut self.imp;
        let h = &e.state.file.file_data[e.data..e.data + HEADER_SIZE];
        let mut ym2612_rate = get_le32(&h[header::YM2612_RATE..]) as i64;
        let ym2612_dual = (ym2612_rate & 0x40000000) != 0;
        let mut ym2413_rate = get_le32(&h[header::YM2413_RATE..]) as i64;
        let ym2413_dual = (ym2413_rate & 0x40000000) != 0;
        if ym2413_rate != 0 && get_le32(&h[header::VERSION..]) < 0x110 {
            e.update_fm_rates(&mut ym2413_rate, &mut ym2612_rate);
        }

        e.uses_fm = false;

        e.fm_rate = e.blip_buf.sample_rate() as f64 * OVERSAMPLE_FACTOR;

        if ym2612_rate != 0 {
            ym2612_rate &= !0xC0000000;
            e.uses_fm = true;
            if e.disable_oversampling_ {
                e.fm_rate = ym2612_rate as f64 / 144.0;
            }
            self.dual.setup(
                e.fm_rate / e.blip_buf.sample_rate() as f64,
                ROLLOFF,
                FM_GAIN * e.state.gain(),
            );
            e.ym2612[0].set_rate(e.fm_rate, ym2612_rate as f64)?;
            e.ym2612[0].enable(true);
            if ym2612_dual {
                e.ym2612[1].set_rate(e.fm_rate, ym2612_rate as f64)?;
                e.ym2612[1].enable(true);
            }
            e.state.set_voice_count(8);
        }

        if !e.uses_fm && ym2413_rate != 0 {
            ym2413_rate &= !0xC0000000;
            e.uses_fm = true;
            if e.disable_oversampling_ {
                e.fm_rate = ym2413_rate as f64 / 72.0;
            }
            self.dual.setup(
                e.fm_rate / e.blip_buf.sample_rate() as f64,
                ROLLOFF,
                FM_GAIN * e.state.gain(),
            );
            let result = e.ym2413[0].set_rate(e.fm_rate, ym2413_rate as f64);
            if result == 2 {
                return Err("YM2413 FM sound isn't supported");
            }
            if result != 0 {
                return Err("Out of memory");
            }
            e.ym2413[0].enable(true);
            if ym2413_dual {
                e.ym2413[1].enable(true);
                let result = e.ym2413[1].set_rate(e.fm_rate, ym2413_rate as f64);
                if result == 2 {
                    return Err("YM2413 FM sound isn't supported");
                }
                if result != 0 {
                    return Err("Out of memory");
                }
            }
            e.state.set_voice_count(8);
        }

        if e.uses_fm {
            self.dual
                .reset((e.blip_buf.length() as i64 * e.blip_buf.sample_rate() / 1000) as i32)?;
            e.psg[0].volume(0.135 * FM_GAIN * e.state.gain());
            if e.psg_dual {
                e.psg[1].volume(0.135 * FM_GAIN * e.state.gain());
            }
        } else {
            e.ym2612[0].enable(false);
            e.ym2612[1].enable(false);
            e.ym2413[0].enable(false);
            e.ym2413[1].enable(false);
            e.psg[0].volume(e.state.gain());
            e.psg[1].volume(e.state.gain());
        }

        Ok(())
    }
}

impl ClassicEmu for VgmEmu {
    fn classic(&self) -> &ClassicEmuState {
        &self.imp.classic
    }
    fn classic_mut(&mut self) -> &mut ClassicEmuState {
        &mut self.imp.classic
    }

    fn update_eq(&mut self, eq: &BlipEq) {
        let e = &mut self.imp;
        e.psg[0].treble_eq(eq);
        if e.psg_dual {
            e.psg[1].treble_eq(eq);
        }
        e.dac_synth.treble_eq(eq);
    }

    fn set_voice(&mut self, i: i32, c: Option<BufId>, l: Option<BufId>, r: Option<BufId>) {
        let e = &mut self.imp;
        if e.psg_dual {
            if e.psg_t6w28 {
                // TODO: Make proper output of each PSG chip: 0 - all right, 1 - all left
                if i < sms_apu::OSC_COUNT {
                    e.psg[0].osc_output(i, c, r, r);
                }
                if i < sms_apu::OSC_COUNT {
                    e.psg[1].osc_output(i, c, l, l);
                }
            } else {
                if i < sms_apu::OSC_COUNT {
                    e.psg[0].osc_output(i, c, l, r);
                }
                if i < sms_apu::OSC_COUNT {
                    e.psg[1].osc_output(i, c, l, r);
                }
            }
        } else {
            if i < sms_apu::OSC_COUNT {
                e.psg[0].osc_output(i, c, l, r);
            }
        }
    }

    fn run_clocks(&mut self, time_io: &mut BlipTime, msec: i32) -> BlarggErr {
        let e = &mut self.imp;
        *time_io = e.run_commands(
            ((msec as u32).wrapping_mul(e.vgm_rate) / 1000) as i32,
            &mut [],
        );
        let t = *time_io;
        e.psg[0].end_frame(psg_bufs!(e), t);
        if e.psg_dual {
            e.psg[1].end_frame(psg_bufs!(e), t);
        }
        Ok(())
    }
}

impl MusicEmu for VgmEmu {
    fn emu(&self) -> &MusicEmuState {
        &self.imp.state
    }
    fn emu_mut(&mut self) -> &mut MusicEmuState {
        &mut self.imp.state
    }
    fn set_buffer(&mut self, buf: Box<dyn crate::multi_buffer::MultiBuffer>) {
        self.imp.classic.set_buffer(buf);
    }
    fn effects_buffer(&mut self) -> Option<&mut crate::effects_buffer::EffectsBuffer> {
        self.imp.classic.effects_buffer()
    }
    fn set_equalizer_(&mut self, eq: &Equalizer) {
        classic_set_equalizer_(self, eq)
    }

    fn set_multi_channel(&mut self, is_enabled: bool) -> BlarggErr {
        // we acutally should check here whether this is classic emu or not
        // however set_multi_channel() is called before setup_fm() resulting in uninited is_classic_emu()
        // hard code it to unsupported
        // (#if 0: if ( is_classic_emu() ) { Music_Emu::set_multi_channel_( is_enabled ) })
        let _ = is_enabled;
        Err("multichannel rendering not supported for YM2*** FM sound chip emulators")
    }

    // Track info

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        get_vgm_length(self.header(), out);

        if let Some(gd3) = self.gd3_data() {
            parse_gd3(&gd3[GD3_HEADER_SIZE as usize..], out);
        }

        Ok(())
    }

    // Setup

    fn set_tempo_(&mut self, t: f64) {
        let e = &mut self.imp;
        if e.psg_rate != 0 {
            e.vgm_rate = (44100.0 * t + 0.5) as u32;
            e.blip_time_factor = floor(
                (1i64 << BLIP_TIME_BITS) as f64 / e.vgm_rate as f64 * e.psg_rate as f64 + 0.5,
            ) as u64;
            //debug_printf( "blip_time_factor: %ld\n", blip_time_factor );
            //debug_printf( "vgm_rate: %ld\n", vgm_rate );
            // TODO: remove? calculates vgm_rate more accurately (above differs at most by one Hz only)
            //blip_time_factor = (long) floor( double (1L << blip_time_bits) * psg_rate / 44100 / t + 0.5 );
            //vgm_rate = (long) floor( double (1L << blip_time_bits) * psg_rate / blip_time_factor + 0.5 );

            e.fm_time_factor =
                2 + floor(e.fm_rate * (1i64 << FM_TIME_BITS) as f64 / e.vgm_rate as f64 + 0.5)
                    as u64;
        }
    }

    fn set_sample_rate_(&mut self, sample_rate: i64) -> BlarggErr {
        self.imp.blip_buf.set_sample_rate(sample_rate, 1000 / 30)?;
        classic_set_sample_rate_(self, sample_rate)
    }

    fn mute_voices_(&mut self, mask: i32) {
        classic_mute_voices_(self, mask);
        let e = &mut self.imp;
        e.dac_synth.set_output(Some(BLIP_BUF));
        if e.uses_fm {
            let psg_out = if mask & 0x80 != 0 {
                None
            } else {
                Some(BLIP_BUF)
            };
            e.psg[0].output(psg_out, psg_out, psg_out);
            if e.psg_dual {
                e.psg[1].output(psg_out, psg_out, psg_out);
            }
            if e.ym2612[0].enabled() {
                e.dac_synth.volume(if mask & 0x40 != 0 {
                    0.0
                } else {
                    0.1115 / 256.0 * FM_GAIN * e.state.gain()
                });
                e.ym2612[0].mute_voices(mask);
                if e.ym2612[1].enabled() {
                    e.ym2612[1].mute_voices(mask);
                }
            }
            if e.ym2413[0].enabled() {
                let mut m = mask & 0x3F;
                if mask & 0x20 != 0 {
                    m |= 0x01E0; // channels 5-8
                }
                if mask & 0x40 != 0 {
                    m |= 0x3E00;
                }
                e.ym2413[0].mute_voices(m);
                if e.ym2413[1].enabled() {
                    e.ym2413[1].mute_voices(m);
                }
            }
        }
    }

    fn load_mem_(&mut self, new_data: usize, new_size: usize) -> BlarggErr {
        // blaarg_static_assert( offsetof (header_t,unused2 [8]) == header_size, "VGM Header layout incorrect!" );

        if new_size <= HEADER_SIZE {
            return Err(GME_WRONG_FILE_TYPE);
        }

        let e = &mut self.imp;
        let h = &e.state.file.file_data[new_data..new_data + HEADER_SIZE];

        check_vgm_header(h)?;

        // check( get_le32( h.version ) <= 0x150 );

        // psg rate
        let mut psg_rate = get_le32(&h[header::PSG_RATE..]);
        if psg_rate == 0 {
            psg_rate = 3579545;
        } else if psg_rate > 100000000 {
            psg_rate = 100000000;
        }
        // FIXME (upstream): the flags are read from the rate after it is
        // limited to 100 MHz, which has neither, so the dual and T6W28
        // PSGs are never enabled.
        e.psg_dual = (psg_rate & 0x40000000) != 0;
        e.psg_t6w28 = (psg_rate & 0x80000000) != 0;
        psg_rate &= 0x0FFFFFFF;
        e.psg_rate = psg_rate;
        e.blip_buf.set_clock_rate(psg_rate);

        let loop_offset = get_le32(&h[header::LOOP_OFFSET..]);
        e.data = new_data;
        e.data_size = new_size as u64;

        // get loop
        e.loop_begin = e.data_size;
        if loop_offset != 0 {
            e.loop_begin = loop_offset as u64 + header::LOOP_OFFSET as u64;
        }

        e.state.set_voice_count(sms_apu::OSC_COUNT);

        self.setup_fm()?;

        let names: &[&'static str] = if self.imp.uses_fm {
            &FM_NAMES
        } else {
            &PSG_NAMES
        };
        self.imp.state.set_voice_names(names);

        // do after FM in case output buffer is changed
        setup_buffer(self, psg_rate)
    }

    // Emulation

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        classic_start_track_(self, track)?;
        let e = &mut self.imp;
        let h = &e.state.file.file_data[e.data..e.data + HEADER_SIZE];
        let (feedback, width) = (
            get_le16(&h[header::NOISE_FEEDBACK..]),
            h[header::NOISE_WIDTH],
        );
        let version = get_le32(&h[header::VERSION..]);
        let data_offset = get_le32(&h[header::DATA_OFFSET..]) as u64;
        e.psg[0].reset(feedback, width as i32);
        if e.psg_dual {
            e.psg[1].reset(feedback, width as i32);
        }

        e.dac_disabled = -1;
        e.pos = HEADER_SIZE as u64;
        e.pcm_data = e.pos;
        e.pcm_pos = e.pos;
        e.dac_amp = -1;
        e.vgm_time = 0;
        if version >= 0x150 {
            // check( data_offset );
            if data_offset != 0 {
                e.pos = e
                    .pos
                    .wrapping_add(data_offset + header::DATA_OFFSET as u64)
                    .wrapping_sub(0x40);
            }
        }

        if e.uses_fm {
            if e.ym2413[0].enabled() {
                e.ym2413[0].reset();
            }

            if e.ym2413[1].enabled() {
                e.ym2413[1].reset();
            }

            if e.ym2612[0].enabled() {
                e.ym2612[0].reset();
            }

            if e.ym2612[1].enabled() {
                e.ym2612[1].reset();
            }

            e.fm_time_offset = 0;
            e.blip_buf.clear(true);
            self.dual.clear();
        }
        Ok(())
    }

    fn play_(&mut self, out: &mut [Sample]) -> BlarggErr {
        if !self.imp.uses_fm {
            return classic_play_(self, out);
        }

        self.dual.dual_play(out.len() as i64, out, &mut self.imp);
        Ok(())
    }
}
