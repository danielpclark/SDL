// Game_Music_Emu https://bitbucket.org/mpyne/game-music-emu/

/* Copyright (C) 2004-2006 Shay Green. This module is free software; you
can redistribute it and/or modify it under the terms of the GNU Lesser
General Public License as published by the Free Software Foundation; either
version 2.1 of the License, or (at your option) any later version. This
module is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License for more
details. You should have received a copy of the GNU Lesser General Public
License along with this module; if not, write to the Free Software Foundation,
Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA */
// Modified 2026-10-17: translated into Rust from SDL_mixer 3.3.0's Game_Music_Emu 0.6.5 (see NOTICE).

//! Super Nintendo SPC music file emulator (`Spc_Emu.h`, `Spc_Emu.cpp`).

use crate::blargg_common::{get_le16, get_le32, try_vec, BlarggErr};
use crate::data_reader::DataReader;
use crate::fir_resampler::FirResampler;
use crate::gme_file::{copy_field_n, gme_copy_field, GmeTypeT, TrackInfo, GME_WRONG_FILE_TYPE};
use crate::music_emu::{gme_info_overrides, MusicEmu, MusicEmuState, Sample};
use crate::snes_spc::{self, SnesSpc};
use crate::spc_filter::{self, SpcFilter};

/// The Super Nintendo hardware samples at 32kHz. Other sample rates are
/// handled by resampling the 32kHz output; emulation accuracy is not affected.
pub const NATIVE_SAMPLE_RATE: i64 = 32000;

/// SPC file header
pub const HEADER_SIZE: usize = 0x100;

/// The fields of the SPC `header_t`, as offsets into its bytes.
pub mod header {
    pub const TAG: usize = 0; // [35]
    pub const FORMAT: usize = 35;
    pub const VERSION: usize = 36;
    pub const PC: usize = 37; // [2]
    pub const A: usize = 39;
    pub const X: usize = 40;
    pub const Y: usize = 41;
    pub const PSW: usize = 42;
    pub const SP: usize = 43;
    pub const UNUSED: usize = 44; // [2]
    pub const SONG: usize = 46; // [32]
    pub const GAME: usize = 78; // [32]
    pub const DUMPER: usize = 110; // [16]
    pub const COMMENT: usize = 126; // [32]
    pub const DATE: usize = 158; // [11]
    pub const LEN_SECS: usize = 169; // [3]
    pub const FADE_MSEC: usize = 172; // [4]
    pub const AUTHOR: usize = 176; // [32] sometimes first char should be skipped (see official SPC spec)
    pub const MUTE_MASK: usize = 208;
    pub const EMULATOR: usize = 209;
    pub const UNUSED2: usize = 210; // [46]
}

static VOICE_NAMES: [&str; snes_spc::VOICE_COUNT] = [
    "DSP 1", "DSP 2", "DSP 3", "DSP 4", "DSP 5", "DSP 6", "DSP 7", "DSP 8",
];

// TODO: support Spc_Filter's bass

pub struct SpcEmu {
    pub(crate) state: MusicEmuState,
    /// (the file data's index in the file data)
    file_data: usize,
    file_size: i64,
    resampler: FirResampler,
    filter: SpcFilter,
    apu: Box<SnesSpc>,
}

impl std::fmt::Debug for SpcEmu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpcEmu").finish_non_exhaustive()
    }
}

impl Default for SpcEmu {
    fn default() -> Self {
        Self::new()
    }
}

impl SpcEmu {
    pub fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_SPC_TYPE_);
        state.set_voice_names(&VOICE_NAMES);
        state.set_gain(1.4);
        SpcEmu {
            state,
            file_data: 0,
            file_size: 0,
            resampler: FirResampler::new(24),
            filter: SpcFilter::new(),
            apu: Box::new(SnesSpc::new()),
        }
    }

    /// Prevents channels and global volumes from being phase-negated
    pub fn disable_surround(&mut self, disable: bool) {
        self.apu.disable_surround(disable);
    }

    /// Header for currently loaded file
    pub fn header(&self) -> &[u8] {
        &self.state.file.file_data[self.file_data..self.file_data + HEADER_SIZE]
    }

    // Track info

    pub fn trailer(&self) -> &[u8] {
        let at = self.file_data + self.file_size.min(SPC_SIZE) as usize;
        let n = self.trailer_size() as usize;
        &self.state.file.file_data[at..at + n]
    }

    pub fn trailer_size(&self) -> i64 {
        0i64.max(self.file_size - SPC_SIZE)
    }
}

const SPC_SIZE: i64 = snes_spc::SPC_FILE_SIZE;
const HEAD_SIZE: i64 = HEADER_SIZE as i64;

fn get_spc_xid6(begin: &[u8], out: &mut TrackInfo) {
    let size = begin.len();
    // header
    let mut end = size;
    if size < 8 || &begin[..4] != b"xid6" {
        // check( false );
        return;
    }
    let info_size = get_le32(&begin[4..]) as i64;
    let mut input = 8usize;
    if (end - input) as i64 > info_size {
        // debug_printf( "Extra data after SPC xid6 info\n" );
        end = input + info_size as usize;
    }

    let mut year: i32 = 0;
    let mut copyright = [0u8; 256 + 5];
    let mut copyright_len: i32 = 0;
    const YEAR_LEN: usize = 5;

    while end as i64 - input as i64 >= 4 {
        // header
        let id = begin[input] as i32;
        let data = begin[input + 3] as i32 * 0x100 + begin[input + 2] as i32;
        let type_ = begin[input + 1] as i32;
        let len = if type_ != 0 { data } else { 0 };
        input += 4;
        if len as i64 > end as i64 - input as i64 {
            // check( false );
            break; // block goes past end of data
        }

        // handle specific block types
        let field: Option<&mut [u8]> = match id {
            0x01 => Some(&mut out.song),
            0x02 => Some(&mut out.game),
            0x03 => Some(&mut out.author),
            0x04 => Some(&mut out.dumper),
            0x07 => Some(&mut out.comment),
            0x14 => {
                year = data;
                None
            }

            //case 0x30: // intro length
            // Many SPCs have intro length set wrong for looped tracks, making it useless
            // (the intro length, commented out)
            0x13 => {
                copyright_len = len.min((copyright.len() - YEAR_LEN) as i32);
                let n = copyright_len as usize;
                copyright[YEAR_LEN..YEAR_LEN + n].copy_from_slice(&begin[input..input + n]);
                None
            }

            _ => {
                // if ( id < 0x01 || (id > 0x07 && id < 0x10) ||
                //      (id > 0x14 && id < 0x30) || id > 0x36 )
                //  debug_printf( "Unknown SPC xid6 block: %X\n", (int) id );
                None
            }
        };
        if let Some(field) = field {
            // check( type == 1 );
            // (FIXME (upstream): with an empty block at the very end of
            // the data, upstream reads the byte past it; this reads
            // nothing, and leaves the field as it was)
            copy_field_n(field, &begin[input..], len);
        }

        // skip to next block
        input += len as usize;

        // blocks are supposed to be 4-byte aligned with zero-padding...
        let unaligned = input;
        while input & 3 != 0 && input < end {
            let b = begin[input];
            input += 1;
            if b != 0 {
                // ...but some files have no padding
                input = unaligned;
                // debug_printf( "SPC info tag wasn't properly padded to align\n" );
                break;
            }
        }
    }

    let mut p = YEAR_LEN;
    if year != 0 {
        p -= 1;
        copyright[p] = b' ';
        for _ in 0..4 {
            p -= 1;
            copyright[p] = (year % 10) as u8 + b'0';
            year /= 10;
        }
        copyright_len += YEAR_LEN as i32;
    }
    if copyright_len != 0 {
        let n = copyright_len as usize;
        copy_field_n(&mut out.copyright, &copyright[p..p + n], copyright_len);
    }

    // check( in == end );
}

fn decode_length(h: &[u8], tag: usize, size: usize) -> i64 {
    // decode length (can be in text or binary format, sometimes ambiguous ugh)
    let mut len_secs: i64 = 0;
    for i in 0..size {
        let n = (h[tag + i] as u32).wrapping_sub(b'0' as u32);
        if n > 9 {
            // ignore single-digit text lengths
            // (except if author field is present and begins at offset 1, ugh)
            if i == 1 && (h[header::AUTHOR] != 0 || h[header::AUTHOR + 1] == 0) {
                len_secs = 0;
            }
            break;
        }
        len_secs *= 10;
        len_secs += n as i64;
    }
    len_secs
}

fn get_spc_info(h: &[u8], xid6: &[u8], out: &mut TrackInfo) {
    let mut len_secs = decode_length(h, header::LEN_SECS, 3);
    if len_secs == 0 || len_secs > 0x1FFF {
        len_secs = get_le16(&h[header::LEN_SECS..]) as i64;
    }
    if len_secs < 0x1FFF {
        out.length = len_secs * 1000;
    }

    out.fade_length = decode_length(h, header::FADE_MSEC, 4);

    let a0 = h[header::AUTHOR] as i8; // (char)
    let offset = (a0 < b' ' as i8 || (a0 as i32 - b'0' as i32) as u32 <= 9) as usize;
    copy_field_n(
        &mut out.author,
        &h[header::AUTHOR + offset..header::AUTHOR + 32],
        32 - offset as i32,
    );

    gme_copy_field(&mut out.song, &h[header::SONG..header::SONG + 32]);
    gme_copy_field(&mut out.game, &h[header::GAME..header::GAME + 32]);
    gme_copy_field(&mut out.dumper, &h[header::DUMPER..header::DUMPER + 16]);
    gme_copy_field(&mut out.comment, &h[header::COMMENT..header::COMMENT + 32]);

    if !xid6.is_empty() {
        get_spc_xid6(xid6, out);
    }
}

fn check_spc_header(header: &[u8]) -> BlarggErr {
    if &header[..27] != b"SNES-SPC700 Sound File Data" {
        return Err(GME_WRONG_FILE_TYPE);
    }
    Ok(())
}

/// `Spc_File`: the info-only SPC reader
pub(crate) struct SpcFile {
    state: MusicEmuState,
    header: [u8; HEADER_SIZE],
    xid6: Vec<u8>,
}

impl SpcFile {
    fn new() -> Self {
        let mut state = MusicEmuState::new();
        state.set_type(&GME_SPC_TYPE_);
        SpcFile {
            state,
            header: [0; HEADER_SIZE],
            xid6: Vec::new(),
        }
    }
}

impl MusicEmu for SpcFile {
    gme_info_overrides!();

    fn load_(&mut self, input: &mut dyn DataReader) -> BlarggErr {
        let file_size = input.remain();
        if file_size < snes_spc::SPC_MIN_FILE_SIZE {
            return Err(GME_WRONG_FILE_TYPE);
        }
        input.read(&mut self.header[..HEAD_SIZE as usize])?;
        check_spc_header(&self.header)?;
        let xid6_size = file_size - SPC_SIZE;
        if xid6_size > 0 {
            self.xid6 = try_vec(xid6_size as usize)?;
            input.skip(SPC_SIZE - HEAD_SIZE)?;
            input.read(&mut self.xid6)?;
        }
        Ok(())
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        get_spc_info(&self.header, &self.xid6, out);
        Ok(())
    }
}

fn new_spc_emu() -> Box<dyn MusicEmu> {
    Box::new(SpcEmu::new())
}
fn new_spc_file() -> Box<dyn MusicEmu> {
    Box::new(SpcFile::new())
}

pub(crate) static GME_SPC_TYPE_: GmeTypeT = GmeTypeT {
    system: "Super Nintendo",
    track_count: 1,
    new_emu: new_spc_emu,
    new_info: new_spc_file,
    extension_: "SPC",
    flags_: 0,
};

impl SpcEmu {
    // Emulation

    fn play_and_filter(&mut self, count: i64, out: Option<&mut [Sample]>) -> BlarggErr {
        // (out: None for the resampler's buffer)
        match out {
            Some(out) => {
                self.apu
                    .play(count as i32, Some(&mut out[..count as usize]))?;
                self.filter.run(out, count as i32);
            }
            None => {
                let buf = self.resampler.buffer();
                self.apu
                    .play(count as i32, Some(&mut buf[..count as usize]))?;
                self.filter.run(buf, count as i32);
            }
        }
        Ok(())
    }
}

impl MusicEmu for SpcEmu {
    fn emu(&self) -> &MusicEmuState {
        &self.state
    }
    fn emu_mut(&mut self) -> &mut MusicEmuState {
        &mut self.state
    }

    fn track_info_(&self, out: &mut TrackInfo, _track: i32) -> BlarggErr {
        get_spc_info(self.header(), self.trailer(), out);
        Ok(())
    }

    // Setup

    fn set_sample_rate_(&mut self, sample_rate: i64) -> BlarggErr {
        self.apu.init()?;
        self.enable_accuracy(false);
        if sample_rate != NATIVE_SAMPLE_RATE {
            self.resampler
                .buffer_size((NATIVE_SAMPLE_RATE / 20 * 2) as i32)?;
            self.resampler
                .time_ratio(NATIVE_SAMPLE_RATE as f64 / sample_rate as f64, 0.9965, 1.0);
        }
        Ok(())
    }

    fn enable_accuracy_(&mut self, b: bool) {
        // Music_Emu::enable_accuracy_( b ); (nothing)
        self.filter.enable(b);
    }

    fn mute_voices_(&mut self, m: i32) {
        // Music_Emu::mute_voices_( m ); (nothing)
        self.apu.mute_voices(m);
    }

    fn disable_echo_(&mut self, disable: bool) {
        self.apu.disable_echo(disable);
    }

    fn load_mem_(&mut self, input: usize, size: usize) -> BlarggErr {
        // blaarg_static_assert( offsetof (header_t,unused2 [46]) == header_size, ... );
        self.file_data = input;
        self.file_size = size as i64;
        self.state.set_voice_count(snes_spc::VOICE_COUNT as i32);
        if (size as i64) < snes_spc::SPC_MIN_FILE_SIZE {
            return Err(GME_WRONG_FILE_TYPE);
        }
        check_spc_header(&self.state.file.file_data[input..])
    }

    // Emulation

    fn set_tempo_(&mut self, t: f64) {
        self.apu.set_tempo((t * snes_spc::TEMPO_UNIT as f64) as i32);
    }

    fn start_track_(&mut self, track: i32) -> BlarggErr {
        crate::music_emu::music_emu_start_track_(self, track)?;
        self.resampler.clear();
        self.filter.clear();
        {
            let (at, n) = (self.file_data, self.file_size as usize);
            let data = &self.state.file.file_data[at..at + n];
            self.apu.load_spc(data)?;
        }
        self.filter
            .set_gain((self.state.gain() * spc_filter::GAIN_UNIT as f64) as i32);
        self.apu.clear_echo();
        // FIXME (upstream): left uninitialized, and its length is read when
        // the header's length is too long to set it; it starts zeroed here,
        // and upstream's build for the tests is patched the same.
        let mut spc_info = TrackInfo::default();
        self.track_info_(&mut spc_info, track)?;

        // Set a default track length, need a non-zero fadeout
        if self.autoload_playback_limit() && spc_info.length > 0 {
            self.set_fade(spc_info.length, 50);
        }
        Ok(())
    }

    fn skip_(&mut self, mut count: i64) -> BlarggErr {
        if self.state.sample_rate() != NATIVE_SAMPLE_RATE {
            count = (count as f64 * self.resampler.ratio()) as i64 & !1;
            count -= self.resampler.skip_input(count) as i64;
        }

        // TODO: shouldn't skip be adjusted for the 64 samples read afterwards?

        if count > 0 {
            self.apu.skip(count as i32)?;
            self.filter.clear();
        }

        // eliminate pop due to resampler
        const RESAMPLER_LATENCY: usize = 64;
        let mut buf = [0 as Sample; RESAMPLER_LATENCY];
        self.play_(&mut buf)
    }

    fn play_(&mut self, out: &mut [Sample]) -> BlarggErr {
        let count = out.len() as i64;
        if self.state.sample_rate() == NATIVE_SAMPLE_RATE {
            return self.play_and_filter(count, Some(out));
        }

        let mut remain = count;
        while remain > 0 {
            let at = (count - remain) as usize;
            remain -= self.resampler.read(&mut out[at..], remain as i32) as i64;
            if remain > 0 {
                let n = self.resampler.max_write() as i64;
                self.play_and_filter(n, None)?;
                self.resampler.write(n);
            }
        }
        // check( remain == 0 );
        Ok(())
    }
}
