// Rust translation of src/dr_libs/dr_mp3.h (dr_mp3 v0.7.4) from SDL_mixer.
// dr_mp3 is by David Reid, based on minimp3 by lieff; it is available under
// a choice of the public domain (Unlicense) or MIT No Attribution, and
// minimp3 is CC0 (public domain). See LICENSE.txt.
// This is an altered (translated) version of the original software.

//! MP3 audio decoder. Translation of dr_mp3, "based on minimp3
//! (<https://github.com/lieff/minimp3>) which is where the real work was
//! done", in the configuration SDL_mixer builds it with
//! (`DR_MP3_FLOAT_OUTPUT`, `DR_MP3_NO_STDIO`).
//!
//! minimp3 has SSE and NEON versions of some of its loops; on x86 they are
//! always used, and only the polyphase DCT's final sums are added in a
//! different order than the scalar code does. This translation adds them
//! in the SIMD order, so its output matches upstream's x86 builds bit for
//! bit everywhere.
//!
//! Only what SDL_mixer uses is translated: the low level decoder and the
//! pull API over callbacks (here an [`IoStream`]), with seek tables and
//! frame counts. Not translated: the memory and stdio initializers, the
//! s16 output, the metadata callback (SDL_mixer passes none), the
//! `drmp3_open_*_and_read_*` helpers and the custom allocators.

#![allow(clippy::excessive_precision)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_range_loop)]

use sdl3::io::{IoStream, IoWhence};

pub(crate) const DRMP3_MAX_PCM_FRAMES_PER_MP3_FRAME: usize = 1152;
pub(crate) const DRMP3_MAX_SAMPLES_PER_FRAME: usize = DRMP3_MAX_PCM_FRAMES_PER_MP3_FRAME * 2;

/*
Low Level Push API
==================
*/
const DRMP3_MAX_BITRESERVOIR_BYTES: usize = 511;
const DRMP3_MAX_FREE_FORMAT_FRAME_SIZE: i32 = 2304; /* more than ISO spec's */
const DRMP3_MAX_L3_FRAME_PAYLOAD_BYTES: usize = DRMP3_MAX_FREE_FORMAT_FRAME_SIZE as usize; /* MUST be >= 320000/8/32000*1152 = 1440 */

/// Translation of `drmp3dec_frame_info`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Drmp3decFrameInfo {
    pub frame_bytes: i32,
    pub channels: i32,
    pub sample_rate: i32,
    pub layer: i32,
    pub bitrate_kbps: i32,
}

/// Translation of `drmp3_bs`: a bit reader over `buf`.
struct Drmp3Bs<'a> {
    buf: &'a [u8],
    pos: i32,
    limit: i32,
}

/// Translation of `drmp3_L3_gr_info`.
#[derive(Clone, Copy, Default)]
struct Drmp3L3GrInfo {
    sfbtab: &'static [u8],
    part_23_length: u16,
    big_values: u16,
    scalefac_compress: u16,
    global_gain: u8,
    block_type: u8,
    mixed_block_flag: u8,
    n_long_sfb: u8,
    n_short_sfb: u8,
    table_select: [u8; 3],
    region_count: [u8; 3],
    subblock_gain: [u8; 3],
    preflag: u8,
    scalefac_scale: u8,
    count1_table: u8,
    scfsi: u8,
}

const MAINDATA_SIZE: usize = DRMP3_MAX_BITRESERVOIR_BYTES + DRMP3_MAX_L3_FRAME_PAYLOAD_BYTES;

/// Translation of `drmp3dec_scratch` (`grbuf` and `syn` flattened, as the
/// code walks across their rows).
struct Drmp3decScratch {
    bs_pos: i32,
    bs_limit: i32,
    maindata: [u8; MAINDATA_SIZE],
    gr_info: [Drmp3L3GrInfo; 4],
    grbuf: [f32; 2 * 576],
    scf: [f32; 40],
    syn: [f32; (18 + 15) * (2 * 32)],
    ist_pos: [[u8; 39]; 2],
}

/// Translation of `drmp3dec`.
pub(crate) struct Drmp3dec {
    mdct_overlap: [[f32; 9 * 32]; 2],
    qmf_state: [f32; 15 * 2 * 32],
    reserv: i32,
    free_format_bytes: i32,
    header: [u8; 4],
    reserv_buf: [u8; 511],
    scratch: Drmp3decScratch,
}

impl Drmp3dec {
    fn zeroed() -> Box<Drmp3dec> {
        Box::new(Drmp3dec {
            mdct_overlap: [[0.0; 288]; 2],
            qmf_state: [0.0; 960],
            reserv: 0,
            free_format_bytes: 0,
            header: [0; 4],
            reserv_buf: [0; 511],
            scratch: Drmp3decScratch {
                bs_pos: 0,
                bs_limit: 0,
                maindata: [0; MAINDATA_SIZE],
                gr_info: [Drmp3L3GrInfo::default(); 4],
                grbuf: [0.0; 1152],
                scf: [0.0; 40],
                syn: [0.0; 33 * 64],
                ist_pos: [[0; 39]; 2],
            },
        })
    }

    /// `DRMP3_ZERO_MEMORY(dec, sizeof(drmp3dec))`
    fn zero(&mut self) {
        self.mdct_overlap = [[0.0; 288]; 2];
        self.qmf_state = [0.0; 960];
        self.reserv = 0;
        self.free_format_bytes = 0;
        self.header = [0; 4];
        self.reserv_buf = [0; 511];
        let s = &mut self.scratch;
        s.bs_pos = 0;
        s.bs_limit = 0;
        s.maindata = [0; MAINDATA_SIZE];
        s.gr_info = [Drmp3L3GrInfo::default(); 4];
        s.grbuf = [0.0; 1152];
        s.scf = [0.0; 40];
        s.syn = [0.0; 33 * 64];
        s.ist_pos = [[0; 39]; 2];
    }
}

/*
Main API (Pull API)
===================
*/
/// Translation of `drmp3_seek_origin`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Drmp3SeekOrigin {
    Set,
    Cur,
    End,
}

/// Translation of `drmp3_seek_point`.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Drmp3SeekPoint {
    pub seek_pos_in_bytes: u64, /* Points to the first byte of an MP3 frame. */
    pub pcm_frame_index: u64,   /* The index of the PCM frame this seek point targets. */
    pub mp3_frames_to_discard: u16, /* The number of whole MP3 frames to be discarded before pcmFramesToDiscard. */
    pub pcm_frames_to_discard: u16, /* The number of leading samples to read and discard. These are discarded after mp3FramesToDiscard. */
}

/// Translation of `drmp3`, over an [`IoStream`] (SDL_mixer's
/// `DRMP3_IoRead`, `DRMP3_IoSeek` and `DRMP3_IoTell` callbacks).
pub(crate) struct Drmp3<'a> {
    decoder: Box<Drmp3dec>,
    pub channels: u32,
    pub sample_rate: u32,
    io: IoStream<'a>,
    mp3_frame_channels: u32, /* The number of channels in the currently loaded MP3 frame. Internal use only. */
    mp3_frame_sample_rate: u32, /* The sample rate of the currently loaded MP3 frame. Internal use only. */
    pcm_frames_consumed_in_mp3_frame: u32,
    pcm_frames_remaining_in_mp3_frame: u32,
    pcm_frames: Box<[f32; DRMP3_MAX_SAMPLES_PER_FRAME]>,
    current_pcm_frame: u64,   /* The current PCM frame, globally. */
    stream_cursor: u64,       /* The current byte the decoder is sitting on in the raw stream. */
    stream_length: u64, /* The length of the stream in bytes. dr_mp3 will not read beyond this. If a ID3v1 or APE tag is present, this will be set to the first byte of the tag. */
    stream_start_offset: u64, /* The offset of the start of the MP3 data. This is used for skipping ID3v2 and VBR tags. */
    seek_points: std::sync::Arc<[Drmp3SeekPoint]>, /* Empty by default. Set with drmp3_bind_seek_table(). */
    delay_in_pcm_frames: u32,
    padding_in_pcm_frames: u32,
    total_pcm_frame_count: u64, /* Set to DRMP3_UINT64_MAX if the length is unknown. Includes delay and padding. */
    is_vbr: bool,
    is_cbr: bool,
    data_size: usize,
    data_capacity: usize,
    data_consumed: usize,
    data: Vec<u8>,
    at_end: bool,
}

const DRMP3_MAX_FRAME_SYNC_MATCHES: i32 = 10;

const DRMP3_SHORT_BLOCK_TYPE: u8 = 2;
const DRMP3_STOP_BLOCK_TYPE: u8 = 3;
const DRMP3_MODE_MONO: i32 = 3;
const DRMP3_MODE_JOINT_STEREO: i32 = 1;
const DRMP3_HDR_SIZE: i32 = 4;

fn hdr_is_mono(h: &[u8]) -> bool {
    (h[3] & 0xC0) == 0xC0
}
fn hdr_is_ms_stereo(h: &[u8]) -> bool {
    (h[3] & 0xE0) == 0x60
}
fn hdr_is_free_format(h: &[u8]) -> bool {
    (h[2] & 0xF0) == 0
}
fn hdr_is_crc(h: &[u8]) -> bool {
    (h[1] & 1) == 0
}
fn hdr_test_padding(h: &[u8]) -> bool {
    (h[2] & 0x2) != 0
}
fn hdr_test_mpeg1(h: &[u8]) -> bool {
    (h[1] & 0x8) != 0
}
fn hdr_test_not_mpeg25(h: &[u8]) -> bool {
    (h[1] & 0x10) != 0
}
fn hdr_test_i_stereo(h: &[u8]) -> bool {
    (h[3] & 0x10) != 0
}
fn hdr_test_ms_stereo(h: &[u8]) -> bool {
    (h[3] & 0x20) != 0
}
fn hdr_get_stereo_mode(h: &[u8]) -> i32 {
    ((h[3] >> 6) & 3) as i32
}
fn hdr_get_stereo_mode_ext(h: &[u8]) -> i32 {
    ((h[3] >> 4) & 3) as i32
}
fn hdr_get_layer(h: &[u8]) -> i32 {
    ((h[1] >> 1) & 3) as i32
}
fn hdr_get_bitrate(h: &[u8]) -> i32 {
    (h[2] >> 4) as i32
}
fn hdr_get_sample_rate(h: &[u8]) -> i32 {
    ((h[2] >> 2) & 3) as i32
}
fn hdr_get_my_sample_rate(h: &[u8]) -> i32 {
    hdr_get_sample_rate(h) + ((((h[1] >> 3) & 1) + ((h[1] >> 4) & 1)) as i32) * 3
}
fn hdr_is_frame_576(h: &[u8]) -> bool {
    (h[1] & 14) == 2
}
fn hdr_is_layer_1(h: &[u8]) -> bool {
    (h[1] & 6) == 6
}

const DRMP3_BITS_DEQUANTIZER_OUT: i32 = -1;
const DRMP3_MAX_SCF: i32 = 255 + DRMP3_BITS_DEQUANTIZER_OUT * 4 - 210;
const DRMP3_MAX_SCFI: i32 = (DRMP3_MAX_SCF + 3) & !3;

/// Translation of `drmp3_L12_scale_info`.
struct Drmp3L12ScaleInfo {
    scf: [f32; 3 * 64],
    total_bands: u8,
    stereo_bands: u8,
    bitalloc: [u8; 64],
    scfcod: [u8; 64],
}

/// Translation of `drmp3_L12_subband_alloc`.
#[derive(Clone, Copy)]
struct Drmp3L12SubbandAlloc {
    tab_offset: u8,
    code_tab_width: u8,
    band_count: u8,
}

const fn sa(tab_offset: u8, code_tab_width: u8, band_count: u8) -> Drmp3L12SubbandAlloc {
    Drmp3L12SubbandAlloc {
        tab_offset,
        code_tab_width,
        band_count,
    }
}

impl<'a> Drmp3Bs<'a> {
    /// Translation of `drmp3_bs_init()`.
    fn new(data: &'a [u8], bytes: i32) -> Drmp3Bs<'a> {
        Drmp3Bs {
            buf: data,
            pos: 0,
            limit: bytes.wrapping_mul(8),
        }
    }

    fn byte(&self, i: usize) -> u32 {
        self.buf.get(i).copied().unwrap_or(0) as u32
    }

    /// Translation of `drmp3_bs_get_bits()`.
    fn get_bits(&mut self, n: i32) -> u32 {
        let s = (self.pos & 7) as u32;
        let mut shl = n + s as i32;
        let mut p = (self.pos >> 3) as usize;
        self.pos += n;
        if self.pos > self.limit {
            return 0;
        }
        let mut next = self.byte(p) & (255 >> s);
        p += 1;
        let mut cache = 0u32;
        loop {
            shl -= 8;
            if shl <= 0 {
                break;
            }
            cache |= next << shl;
            next = self.byte(p);
            p += 1;
        }
        cache | (next >> -shl)
    }
}

/// Translation of `drmp3_hdr_valid()`.
fn hdr_valid(h: &[u8]) -> bool {
    h[0] == 0xff
        && ((h[1] & 0xF0) == 0xf0 || (h[1] & 0xFE) == 0xe2)
        && (hdr_get_layer(h) != 0)
        && (hdr_get_bitrate(h) != 15)
        && (hdr_get_sample_rate(h) != 3)
}

/// Translation of `drmp3_hdr_compare()`.
fn hdr_compare(h1: &[u8], h2: &[u8]) -> bool {
    hdr_valid(h2)
        && ((h1[1] ^ h2[1]) & 0xFE) == 0
        && ((h1[2] ^ h2[2]) & 0x0C) == 0
        && (hdr_is_free_format(h1) == hdr_is_free_format(h2))
}

/// Translation of `drmp3_hdr_bitrate_kbps()`.
fn hdr_bitrate_kbps(h: &[u8]) -> u32 {
    static HALFRATE: [[[u8; 15]; 3]; 2] = [
        [
            [0, 4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 72, 80],
            [0, 4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 72, 80],
            [0, 16, 24, 28, 32, 40, 48, 56, 64, 72, 80, 88, 96, 112, 128],
        ],
        [
            [0, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160],
            [
                0, 16, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192,
            ],
            [
                0, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224,
            ],
        ],
    ];
    2 * HALFRATE[hdr_test_mpeg1(h) as usize][(hdr_get_layer(h) - 1) as usize]
        [hdr_get_bitrate(h) as usize] as u32
}

/// Translation of `drmp3_hdr_sample_rate_hz()`.
fn hdr_sample_rate_hz(h: &[u8]) -> u32 {
    static G_HZ: [u32; 3] = [44100, 48000, 32000];
    G_HZ[hdr_get_sample_rate(h) as usize]
        >> (!hdr_test_mpeg1(h) as u32)
        >> (!hdr_test_not_mpeg25(h) as u32)
}

/// Translation of `drmp3_hdr_frame_samples()`.
fn hdr_frame_samples(h: &[u8]) -> u32 {
    if hdr_is_layer_1(h) {
        384
    } else {
        1152 >> (hdr_is_frame_576(h) as u32)
    }
}

/// Translation of `drmp3_hdr_frame_bytes()`.
fn hdr_frame_bytes(h: &[u8], free_format_size: i32) -> i32 {
    let mut frame_bytes =
        (hdr_frame_samples(h) * hdr_bitrate_kbps(h) * 125 / hdr_sample_rate_hz(h)) as i32;
    if hdr_is_layer_1(h) {
        frame_bytes &= !3; /* slot align */
    }
    if frame_bytes != 0 {
        frame_bytes
    } else {
        free_format_size
    }
}

/// Translation of `drmp3_hdr_padding()`.
fn hdr_padding(h: &[u8]) -> i32 {
    if hdr_test_padding(h) {
        if hdr_is_layer_1(h) {
            4
        } else {
            1
        }
    } else {
        0
    }
}

/// Translation of `drmp3_L12_subband_alloc_table()`.
fn l12_subband_alloc_table(
    hdr: &[u8],
    sci: &mut Drmp3L12ScaleInfo,
) -> &'static [Drmp3L12SubbandAlloc] {
    let mut alloc: &'static [Drmp3L12SubbandAlloc];
    let mode = hdr_get_stereo_mode(hdr);
    let mut nbands;
    let stereo_bands = if mode == DRMP3_MODE_MONO {
        0
    } else if mode == DRMP3_MODE_JOINT_STEREO {
        (hdr_get_stereo_mode_ext(hdr) << 2) + 4
    } else {
        32
    };

    if hdr_is_layer_1(hdr) {
        static G_ALLOC_L1: [Drmp3L12SubbandAlloc; 1] = [sa(76, 4, 32)];
        alloc = &G_ALLOC_L1;
        nbands = 32;
    } else if !hdr_test_mpeg1(hdr) {
        static G_ALLOC_L2M2: [Drmp3L12SubbandAlloc; 3] =
            [sa(60, 4, 4), sa(44, 3, 7), sa(44, 2, 19)];
        alloc = &G_ALLOC_L2M2;
        nbands = 30;
    } else {
        static G_ALLOC_L2M1: [Drmp3L12SubbandAlloc; 4] =
            [sa(0, 4, 3), sa(16, 4, 8), sa(32, 3, 12), sa(40, 2, 7)];
        let sample_rate_idx = hdr_get_sample_rate(hdr);
        let mut kbps = hdr_bitrate_kbps(hdr) >> ((mode != DRMP3_MODE_MONO) as u32);
        if kbps == 0 {
            /* free-format */
            kbps = 192;
        }

        alloc = &G_ALLOC_L2M1;
        nbands = 27;
        if kbps < 56 {
            static G_ALLOC_L2M1_LOWRATE: [Drmp3L12SubbandAlloc; 2] = [sa(44, 4, 2), sa(44, 3, 10)];
            alloc = &G_ALLOC_L2M1_LOWRATE;
            nbands = if sample_rate_idx == 2 { 12 } else { 8 };
        } else if kbps >= 96 && sample_rate_idx != 1 {
            nbands = 30;
        }
    }

    sci.total_bands = nbands as u8;
    sci.stereo_bands = stereo_bands.min(nbands) as u8;

    alloc
}

macro_rules! dq {
    ($($x:expr),*) => {
        [$([9.53674316e-07f32 / $x as f32, 7.56931807e-07f32 / $x as f32, 6.00777173e-07f32 / $x as f32]),*]
    };
}

/// Translation of `drmp3_L12_read_scalefactors()`.
fn l12_read_scalefactors(
    bs: &mut Drmp3Bs<'_>,
    pba: &[u8],
    scfcod: &[u8],
    bands: usize,
    scf: &mut [f32],
) {
    static G_DEQ_L12: [[f32; 3]; 18] =
        dq!(3, 7, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 16383, 32767, 65535, 3, 5, 9);
    let mut o = 0;
    for i in 0..bands {
        let mut s = 0.0f32;
        let ba = pba[i] as i32;
        let mask = if ba != 0 {
            4 + ((19 >> scfcod[i]) & 3)
        } else {
            0
        };
        let mut m = 4;
        while m != 0 {
            if mask & m != 0 {
                let b = bs.get_bits(6) as i32;
                let idx = (ba * 3 - 6 + b % 3) as usize;
                s = G_DEQ_L12[idx / 3][idx % 3] * ((1i32 << 21) >> (b / 3)) as f32;
            }
            scf[o] = s;
            o += 1;
            m >>= 1;
        }
    }
}

/// Translation of `drmp3_L12_read_scale_info()`.
fn l12_read_scale_info(hdr: &[u8], bs: &mut Drmp3Bs<'_>, sci: &mut Drmp3L12ScaleInfo) {
    static G_BITALLOC_CODE_TAB: [u8; 92] = [
        0, 17, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 0, 17, 18, 3, 19, 4, 5, 6, 7, 8, 9,
        10, 11, 12, 13, 16, 0, 17, 18, 3, 19, 4, 5, 16, 0, 17, 18, 16, 0, 17, 18, 19, 4, 5, 6, 7,
        8, 9, 10, 11, 12, 13, 14, 15, 0, 17, 18, 3, 19, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 0, 2,
        3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
    ];
    let subband_alloc = l12_subband_alloc_table(hdr, sci);
    let mut sai = 0;

    let mut k = 0i32;
    let mut ba_bits = 0i32;
    let mut ba_code_tab = 0usize;

    for i in 0..sci.total_bands as i32 {
        if i == k {
            let a = subband_alloc[sai];
            k += a.band_count as i32;
            ba_bits = a.code_tab_width as i32;
            ba_code_tab = a.tab_offset as usize;
            sai += 1;
        }
        let mut ba = G_BITALLOC_CODE_TAB[ba_code_tab + bs.get_bits(ba_bits) as usize];
        sci.bitalloc[2 * i as usize] = ba;
        if i < sci.stereo_bands as i32 {
            ba = G_BITALLOC_CODE_TAB[ba_code_tab + bs.get_bits(ba_bits) as usize];
        }
        sci.bitalloc[2 * i as usize + 1] = if sci.stereo_bands != 0 { ba } else { 0 };
    }

    for i in 0..2 * sci.total_bands as usize {
        sci.scfcod[i] = if sci.bitalloc[i] != 0 {
            if hdr_is_layer_1(hdr) {
                2
            } else {
                bs.get_bits(2) as u8
            }
        } else {
            6
        };
    }

    let bands = sci.total_bands as usize * 2;
    l12_read_scalefactors(bs, &sci.bitalloc, &sci.scfcod, bands, &mut sci.scf);

    for i in sci.stereo_bands as usize..sci.total_bands as usize {
        sci.bitalloc[2 * i + 1] = 0;
    }
}

/// Translation of `drmp3_L12_dequantize_granule()`: `grbuf` from `base`.
fn l12_dequantize_granule(
    grbuf: &mut [f32],
    base: usize,
    bs: &mut Drmp3Bs<'_>,
    sci: &Drmp3L12ScaleInfo,
    group_size: i32,
) -> i32 {
    let mut choff: isize = 576;
    for j in 0..4 {
        let mut dst = base as isize + (group_size * j) as isize;
        for i in 0..2 * sci.total_bands as usize {
            let ba = sci.bitalloc[i] as i32;
            if ba != 0 {
                if ba < 17 {
                    let half = (1i32 << (ba - 1)) - 1;
                    for k in 0..group_size as isize {
                        let v = (bs.get_bits(ba) as i32 - half) as f32;
                        if let Some(d) = grbuf.get_mut((dst + k) as usize) {
                            *d = v;
                        }
                    }
                } else {
                    let m = (2u32 << (ba - 17)) + 1; /* 3, 5, 9 */
                    let mut code = bs.get_bits((m + 2 - (m >> 3)) as i32); /* 5, 7, 10 */
                    for k in 0..group_size as isize {
                        let v = ((code % m).wrapping_sub(m / 2) as i32) as f32;
                        if let Some(d) = grbuf.get_mut((dst + k) as usize) {
                            *d = v;
                        }
                        code /= m;
                    }
                }
            }
            dst += choff;
            choff = 18 - choff;
        }
    }
    group_size * 4
}

/// Translation of `drmp3_L12_apply_scf_384()`: `scf` from `scf_off`.
fn l12_apply_scf_384(sci: &Drmp3L12ScaleInfo, scf_off: usize, dst: &mut [f32]) {
    let sb = sci.stereo_bands as usize;
    let tb = sci.total_bands as usize;
    dst.copy_within(sb * 18..sb * 18 + (tb - sb) * 18, 576 + sb * 18);
    for i in 0..tb {
        let d = i * 18;
        let s = scf_off + i * 6;
        for k in 0..12 {
            dst[d + k] *= sci.scf[s];
            dst[d + k + 576] *= sci.scf[s + 3];
        }
    }
}

static G_SCF_LONG: [[u8; 23]; 8] = [
    [
        6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 38, 46, 52, 60, 68, 58, 54, 0,
    ],
    [
        12, 12, 12, 12, 12, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 76, 90, 2, 2, 2, 2, 2, 0,
    ],
    [
        6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 38, 46, 52, 60, 68, 58, 54, 0,
    ],
    [
        6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 18, 22, 26, 32, 38, 46, 54, 62, 70, 76, 36, 0,
    ],
    [
        6, 6, 6, 6, 6, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 38, 46, 52, 60, 68, 58, 54, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 6, 6, 8, 8, 10, 12, 16, 20, 24, 28, 34, 42, 50, 54, 76, 158, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 6, 6, 6, 8, 10, 12, 16, 18, 22, 28, 34, 40, 46, 54, 54, 192, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 6, 6, 8, 10, 12, 16, 20, 24, 30, 38, 46, 56, 68, 84, 102, 26, 0,
    ],
];
static G_SCF_SHORT: [[u8; 40]; 8] = [
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 6, 6, 6, 8, 8, 8, 10, 10, 10, 12, 12, 12, 14, 14, 14, 18, 18,
        18, 24, 24, 24, 30, 30, 30, 40, 40, 40, 18, 18, 18, 0,
    ],
    [
        8, 8, 8, 8, 8, 8, 8, 8, 8, 12, 12, 12, 16, 16, 16, 20, 20, 20, 24, 24, 24, 28, 28, 28, 36,
        36, 36, 2, 2, 2, 2, 2, 2, 2, 2, 2, 26, 26, 26, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 6, 6, 6, 6, 6, 6, 8, 8, 8, 10, 10, 10, 14, 14, 14, 18, 18, 18,
        26, 26, 26, 32, 32, 32, 42, 42, 42, 18, 18, 18, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 6, 6, 6, 8, 8, 8, 10, 10, 10, 12, 12, 12, 14, 14, 14, 18, 18,
        18, 24, 24, 24, 32, 32, 32, 44, 44, 44, 12, 12, 12, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 6, 6, 6, 8, 8, 8, 10, 10, 10, 12, 12, 12, 14, 14, 14, 18, 18,
        18, 24, 24, 24, 30, 30, 30, 40, 40, 40, 18, 18, 18, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 6, 6, 6, 8, 8, 8, 10, 10, 10, 12, 12, 12, 14, 14, 14,
        18, 18, 18, 22, 22, 22, 30, 30, 30, 56, 56, 56, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 6, 6, 6, 6, 6, 6, 10, 10, 10, 12, 12, 12, 14, 14, 14,
        16, 16, 16, 20, 20, 20, 26, 26, 26, 66, 66, 66, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 6, 6, 6, 8, 8, 8, 12, 12, 12, 16, 16, 16, 20, 20, 20,
        26, 26, 26, 34, 34, 34, 42, 42, 42, 12, 12, 12, 0,
    ],
];
static G_SCF_MIXED: [[u8; 40]; 8] = [
    [
        6, 6, 6, 6, 6, 6, 6, 6, 6, 8, 8, 8, 10, 10, 10, 12, 12, 12, 14, 14, 14, 18, 18, 18, 24, 24,
        24, 30, 30, 30, 40, 40, 40, 18, 18, 18, 0, 0, 0, 0,
    ],
    [
        12, 12, 12, 4, 4, 4, 8, 8, 8, 12, 12, 12, 16, 16, 16, 20, 20, 20, 24, 24, 24, 28, 28, 28,
        36, 36, 36, 2, 2, 2, 2, 2, 2, 2, 2, 2, 26, 26, 26, 0,
    ],
    [
        6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 8, 8, 8, 10, 10, 10, 14, 14, 14, 18, 18, 18, 26, 26,
        26, 32, 32, 32, 42, 42, 42, 18, 18, 18, 0, 0, 0, 0,
    ],
    [
        6, 6, 6, 6, 6, 6, 6, 6, 6, 8, 8, 8, 10, 10, 10, 12, 12, 12, 14, 14, 14, 18, 18, 18, 24, 24,
        24, 32, 32, 32, 44, 44, 44, 12, 12, 12, 0, 0, 0, 0,
    ],
    [
        6, 6, 6, 6, 6, 6, 6, 6, 6, 8, 8, 8, 10, 10, 10, 12, 12, 12, 14, 14, 14, 18, 18, 18, 24, 24,
        24, 30, 30, 30, 40, 40, 40, 18, 18, 18, 0, 0, 0, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 6, 6, 4, 4, 4, 6, 6, 6, 8, 8, 8, 10, 10, 10, 12, 12, 12, 14, 14, 14, 18,
        18, 18, 22, 22, 22, 30, 30, 30, 56, 56, 56, 0, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 6, 6, 4, 4, 4, 6, 6, 6, 6, 6, 6, 10, 10, 10, 12, 12, 12, 14, 14, 14, 16,
        16, 16, 20, 20, 20, 26, 26, 26, 66, 66, 66, 0, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 6, 6, 4, 4, 4, 6, 6, 6, 8, 8, 8, 12, 12, 12, 16, 16, 16, 20, 20, 20, 26,
        26, 26, 34, 34, 34, 42, 42, 42, 12, 12, 12, 0, 0,
    ],
];

/// Translation of `drmp3_L3_read_side_info()`.
fn l3_read_side_info(bs: &mut Drmp3Bs<'_>, gr: &mut [Drmp3L3GrInfo], hdr: &[u8]) -> i32 {
    let mut tables: u32;
    let mut scfsi: u32 = 0;
    let main_data_begin: i32;
    let mut part_23_sum: i32 = 0;
    let mut gr_count: i32 = if hdr_is_mono(hdr) { 1 } else { 2 };
    let mut sr_idx = hdr_get_my_sample_rate(hdr);
    sr_idx -= (sr_idx != 0) as i32;
    // Note (upstream): drmp3_init_internal() runs this over whatever bytes
    // precede its first frame, where the sample rate field can be the
    // reserved 3 and `sr_idx` one past the tables; upstream then points
    // `sfbtab` past them, but nobody reads it from there.
    let sr_idx = (sr_idx as usize).min(G_SCF_LONG.len() - 1);

    if hdr_test_mpeg1(hdr) {
        gr_count *= 2;
        main_data_begin = bs.get_bits(9) as i32;
        scfsi = bs.get_bits(7 + gr_count);
    } else {
        main_data_begin = (bs.get_bits(8 + gr_count) >> gr_count) as i32;
    }

    let mut g = 0;
    loop {
        let gi = &mut gr[g];
        if hdr_is_mono(hdr) {
            scfsi <<= 4;
        }
        gi.part_23_length = bs.get_bits(12) as u16;
        part_23_sum += gi.part_23_length as i32;
        gi.big_values = bs.get_bits(9) as u16;
        if gi.big_values > 288 {
            return -1;
        }
        gi.global_gain = bs.get_bits(8) as u8;
        gi.scalefac_compress = bs.get_bits(if hdr_test_mpeg1(hdr) { 4 } else { 9 }) as u16;
        gi.sfbtab = &G_SCF_LONG[sr_idx];
        gi.n_long_sfb = 22;
        gi.n_short_sfb = 0;
        if bs.get_bits(1) != 0 {
            gi.block_type = bs.get_bits(2) as u8;
            if gi.block_type == 0 {
                return -1;
            }
            gi.mixed_block_flag = bs.get_bits(1) as u8;
            gi.region_count[0] = 7;
            gi.region_count[1] = 255;
            if gi.block_type == DRMP3_SHORT_BLOCK_TYPE {
                scfsi &= 0x0F0F;
                if gi.mixed_block_flag == 0 {
                    gi.region_count[0] = 8;
                    gi.sfbtab = &G_SCF_SHORT[sr_idx];
                    gi.n_long_sfb = 0;
                    gi.n_short_sfb = 39;
                } else {
                    gi.sfbtab = &G_SCF_MIXED[sr_idx];
                    gi.n_long_sfb = if hdr_test_mpeg1(hdr) { 8 } else { 6 };
                    gi.n_short_sfb = 30;
                }
            }
            tables = bs.get_bits(10);
            tables <<= 5;
            gi.subblock_gain[0] = bs.get_bits(3) as u8;
            gi.subblock_gain[1] = bs.get_bits(3) as u8;
            gi.subblock_gain[2] = bs.get_bits(3) as u8;
        } else {
            gi.block_type = 0;
            gi.mixed_block_flag = 0;
            tables = bs.get_bits(15);
            gi.region_count[0] = bs.get_bits(4) as u8;
            gi.region_count[1] = bs.get_bits(3) as u8;
            gi.region_count[2] = 255;
        }
        gi.table_select[0] = (tables >> 10) as u8;
        gi.table_select[1] = ((tables >> 5) & 31) as u8;
        gi.table_select[2] = (tables & 31) as u8;
        gi.preflag = if hdr_test_mpeg1(hdr) {
            bs.get_bits(1) as u8
        } else {
            (gi.scalefac_compress >= 500) as u8
        };
        gi.scalefac_scale = bs.get_bits(1) as u8;
        gi.count1_table = bs.get_bits(1) as u8;
        gi.scfsi = ((scfsi >> 12) & 15) as u8;
        scfsi <<= 4;
        g += 1;
        gr_count -= 1;
        if gr_count == 0 {
            break;
        }
    }

    if part_23_sum + bs.pos > bs.limit + main_data_begin * 8 {
        return -1;
    }

    main_data_begin
}

/// Translation of `drmp3_L3_read_scalefactors()`.
fn l3_read_scalefactors(
    scf: &mut [u8],
    ist_pos: &mut [u8],
    scf_size: &[u8; 4],
    scf_count: &[u8],
    bitbuf: &mut Drmp3Bs<'_>,
    mut scfsi: i32,
) {
    let mut o = 0usize;
    let mut i = 0;
    while i < 4 && scf_count[i] != 0 {
        let cnt = scf_count[i] as usize;
        if scfsi & 8 != 0 {
            scf[o..o + cnt].copy_from_slice(&ist_pos[o..o + cnt]);
        } else {
            let bits = scf_size[i] as i32;
            if bits == 0 {
                scf[o..o + cnt].fill(0);
                ist_pos[o..o + cnt].fill(0);
            } else {
                let max_scf = if scfsi < 0 { (1i32 << bits) - 1 } else { -1 };
                for k in 0..cnt {
                    let s = bitbuf.get_bits(bits) as i32;
                    ist_pos[o + k] = if s == max_scf { -1i32 as u8 } else { s as u8 };
                    scf[o + k] = s as u8;
                }
            }
        }
        o += cnt;
        i += 1;
        scfsi = scfsi.wrapping_mul(2);
    }
    scf[o] = 0;
    scf[o + 1] = 0;
    scf[o + 2] = 0;
}

/// Translation of `drmp3_L3_ldexp_q2()`.
fn l3_ldexp_q2(mut y: f32, mut exp_q2: i32) -> f32 {
    static G_EXPFRAC: [f32; 4] = [
        9.31322575e-10,
        7.83145814e-10,
        6.58544508e-10,
        5.53767716e-10,
    ];
    loop {
        let e = (30 * 4).min(exp_q2);
        y *= G_EXPFRAC[(e & 3) as usize] * ((1i32 << 30) >> (e >> 2)) as f32;
        exp_q2 -= e;
        if exp_q2 <= 0 {
            break;
        }
    }
    y
}

/// Translation of `drmp3_L3_decode_scalefactors()`.
fn l3_decode_scalefactors(
    hdr: &[u8],
    ist_pos: &mut [u8],
    bs: &mut Drmp3Bs<'_>,
    gr: &Drmp3L3GrInfo,
    scf: &mut [f32],
    ch: i32,
) {
    static G_SCF_PARTITIONS: [[u8; 28]; 3] = [
        [
            6, 5, 5, 5, 6, 5, 5, 5, 6, 5, 7, 3, 11, 10, 0, 0, 7, 7, 7, 0, 6, 6, 6, 3, 8, 8, 5, 0,
        ],
        [
            8, 9, 6, 12, 6, 9, 9, 9, 6, 9, 12, 6, 15, 18, 0, 0, 6, 15, 12, 0, 6, 12, 9, 6, 6, 18,
            9, 0,
        ],
        [
            9, 9, 6, 12, 9, 9, 9, 9, 9, 9, 12, 6, 18, 18, 0, 0, 12, 12, 12, 0, 12, 9, 9, 6, 15, 12,
            9, 0,
        ],
    ];
    let scf_partition =
        &G_SCF_PARTITIONS[(gr.n_short_sfb != 0) as usize + (gr.n_long_sfb == 0) as usize];
    let mut scf_partition_off = 0usize;
    let mut scf_size = [0u8; 4];
    let mut iscf = [0u8; 40];
    let scf_shift = gr.scalefac_scale as i32 + 1;
    let mut scfsi = gr.scfsi as i32;

    if hdr_test_mpeg1(hdr) {
        static G_SCFC_DECODE: [u8; 16] = [0, 1, 2, 3, 12, 5, 6, 7, 9, 10, 11, 13, 14, 15, 18, 19];
        let part = G_SCFC_DECODE[gr.scalefac_compress as usize] as i32;
        scf_size[0] = (part >> 2) as u8;
        scf_size[1] = scf_size[0];
        scf_size[2] = (part & 3) as u8;
        scf_size[3] = scf_size[2];
    } else {
        static G_MOD: [u8; 6 * 4] = [
            5, 5, 4, 4, 5, 5, 4, 1, 4, 3, 1, 1, 5, 6, 6, 1, 4, 4, 4, 1, 4, 3, 1, 1,
        ];
        let ist = (hdr_test_i_stereo(hdr) && ch != 0) as i32;
        let mut sfc = gr.scalefac_compress as i32 >> ist;
        let mut k = (ist * 3 * 4) as usize;
        while sfc >= 0 {
            let mut modprod = 1i32;
            for i in (0..4).rev() {
                scf_size[i] = (sfc / modprod % G_MOD[k + i] as i32) as u8;
                modprod *= G_MOD[k + i] as i32;
            }
            sfc -= modprod;
            k += 4;
        }
        scf_partition_off += k;
        scfsi = -16;
    }
    l3_read_scalefactors(
        &mut iscf,
        ist_pos,
        &scf_size,
        &scf_partition[scf_partition_off..],
        bs,
        scfsi,
    );

    if gr.n_short_sfb != 0 {
        let sh = 3 - scf_shift;
        let nl = gr.n_long_sfb as usize;
        let mut i = 0usize;
        while i < gr.n_short_sfb as usize {
            iscf[nl + i] = iscf[nl + i].wrapping_add(((gr.subblock_gain[0] as i32) << sh) as u8);
            iscf[nl + i + 1] =
                iscf[nl + i + 1].wrapping_add(((gr.subblock_gain[1] as i32) << sh) as u8);
            iscf[nl + i + 2] =
                iscf[nl + i + 2].wrapping_add(((gr.subblock_gain[2] as i32) << sh) as u8);
            i += 3;
        }
    } else if gr.preflag != 0 {
        static G_PREAMP: [u8; 10] = [1, 1, 1, 1, 2, 2, 3, 3, 3, 2];
        for i in 0..10 {
            iscf[11 + i] = iscf[11 + i].wrapping_add(G_PREAMP[i]);
        }
    }

    let gain_exp = gr.global_gain as i32 + DRMP3_BITS_DEQUANTIZER_OUT * 4
        - 210
        - if hdr_is_ms_stereo(hdr) { 2 } else { 0 };
    let gain = l3_ldexp_q2(
        (1i32 << (DRMP3_MAX_SCFI / 4)) as f32,
        DRMP3_MAX_SCFI - gain_exp,
    );
    for i in 0..(gr.n_long_sfb as usize + gr.n_short_sfb as usize) {
        scf[i] = l3_ldexp_q2(gain, (iscf[i] as i32) << scf_shift);
    }
}

static G_DRMP3_POW43: [f32; 129 + 16] = [
    0.0, -1.0, -2.519842, -4.326749, -6.349604, -8.549880, -10.902724, -13.390518, -16.000000,
    -18.720754, -21.544347, -24.463781, -27.473142, -30.567351, -33.741992, -36.993181, 0.0, 1.0,
    2.519842, 4.326749, 6.349604, 8.549880, 10.902724, 13.390518, 16.000000, 18.720754, 21.544347,
    24.463781, 27.473142, 30.567351, 33.741992, 36.993181, 40.317474, 43.711787, 47.173345,
    50.699631, 54.288352, 57.937408, 61.644865, 65.408941, 69.227979, 73.100443, 77.024898,
    81.000000, 85.024491, 89.097188, 93.216975, 97.382800, 101.593667, 105.848633, 110.146801,
    114.487321, 118.869381, 123.292209, 127.755065, 132.257246, 136.798076, 141.376907, 145.993119,
    150.646117, 155.335327, 160.060199, 164.820202, 169.614826, 174.443577, 179.305980, 184.201575,
    189.129918, 194.090580, 199.083145, 204.107210, 209.162385, 214.248292, 219.364564, 224.510845,
    229.686789, 234.892058, 240.126328, 245.389280, 250.680604, 256.000000, 261.347174, 266.721841,
    272.123723, 277.552547, 283.008049, 288.489971, 293.998060, 299.532071, 305.091761, 310.676898,
    316.287249, 321.922592, 327.582707, 333.267377, 338.976394, 344.709550, 350.466646, 356.247482,
    362.051866, 367.879608, 373.730522, 379.604427, 385.501143, 391.420496, 397.362314, 403.326427,
    409.312672, 415.320884, 421.350905, 427.402579, 433.475750, 439.570269, 445.685987, 451.822757,
    457.980436, 464.158883, 470.357960, 476.577530, 482.817459, 489.077615, 495.357868, 501.658090,
    507.978156, 514.317941, 520.677324, 527.056184, 533.454404, 539.871867, 546.308458, 552.764065,
    559.238575, 565.731879, 572.243870, 578.774440, 585.323483, 591.890898, 598.476581, 605.080431,
    611.702349, 618.342238, 625.000000, 631.675540, 638.368763, 645.079578,
];

/// Translation of `drmp3_L3_pow_43()`.
fn l3_pow_43(mut x: i32) -> f32 {
    let mut mult = 256;

    if x < 129 {
        return G_DRMP3_POW43[(16 + x) as usize];
    }

    if x < 1024 {
        mult = 16;
        x <<= 3;
    }

    let sign = 2 * x & 64;
    let frac = ((x & 63) - sign) as f32 / ((x & !63) + sign) as f32;
    G_DRMP3_POW43[(16 + ((x + sign) >> 6)) as usize]
        * (1.0 + frac * ((4.0f32 / 3.0) + frac * (2.0f32 / 9.0)))
        * mult as f32
}

static TABS: [i16; 2164] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    785, 785, 785, 785, 784, 784, 784, 784, 513, 513, 513, 513, 513, 513, 513, 513, 256, 256, 256,
    256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, -255, 1313, 1298, 1282, 785,
    785, 785, 785, 784, 784, 784, 784, 769, 769, 769, 769, 256, 256, 256, 256, 256, 256, 256, 256,
    256, 256, 256, 256, 256, 256, 256, 256, 290, 288, -255, 1313, 1298, 1282, 769, 769, 769, 769,
    529, 529, 529, 529, 529, 529, 529, 529, 528, 528, 528, 528, 528, 528, 528, 528, 512, 512, 512,
    512, 512, 512, 512, 512, 290, 288, -253, -318, -351, -367, 785, 785, 785, 785, 784, 784, 784,
    784, 769, 769, 769, 769, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256,
    256, 256, 819, 818, 547, 547, 275, 275, 275, 275, 561, 560, 515, 546, 289, 274, 288, 258, -254,
    -287, 1329, 1299, 1314, 1312, 1057, 1057, 1042, 1042, 1026, 1026, 784, 784, 784, 784, 529, 529,
    529, 529, 529, 529, 529, 529, 769, 769, 769, 769, 768, 768, 768, 768, 563, 560, 306, 306, 291,
    259, -252, -413, -477, -542, 1298, -575, 1041, 1041, 784, 784, 784, 784, 769, 769, 769, 769,
    256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, -383, -399,
    1107, 1092, 1106, 1061, 849, 849, 789, 789, 1104, 1091, 773, 773, 1076, 1075, 341, 340, 325,
    309, 834, 804, 577, 577, 532, 532, 516, 516, 832, 818, 803, 816, 561, 561, 531, 531, 515, 546,
    289, 289, 288, 258, -252, -429, -493, -559, 1057, 1057, 1042, 1042, 529, 529, 529, 529, 529,
    529, 529, 529, 784, 784, 784, 784, 769, 769, 769, 769, 512, 512, 512, 512, 512, 512, 512, 512,
    -382, 1077, -415, 1106, 1061, 1104, 849, 849, 789, 789, 1091, 1076, 1029, 1075, 834, 834, 597,
    581, 340, 340, 339, 324, 804, 833, 532, 532, 832, 772, 818, 803, 817, 787, 816, 771, 290, 290,
    290, 290, 288, 258, -253, -349, -414, -447, -463, 1329, 1299, -479, 1314, 1312, 1057, 1057,
    1042, 1042, 1026, 1026, 785, 785, 785, 785, 784, 784, 784, 784, 769, 769, 769, 769, 768, 768,
    768, 768, -319, 851, 821, -335, 836, 850, 805, 849, 341, 340, 325, 336, 533, 533, 579, 579,
    564, 564, 773, 832, 578, 548, 563, 516, 321, 276, 306, 291, 304, 259, -251, -572, -733, -830,
    -863, -879, 1041, 1041, 784, 784, 784, 784, 769, 769, 769, 769, 256, 256, 256, 256, 256, 256,
    256, 256, 256, 256, 256, 256, 256, 256, 256, 256, -511, -527, -543, 1396, 1351, 1381, 1366,
    1395, 1335, 1380, -559, 1334, 1138, 1138, 1063, 1063, 1350, 1392, 1031, 1031, 1062, 1062, 1364,
    1363, 1120, 1120, 1333, 1348, 881, 881, 881, 881, 375, 374, 359, 373, 343, 358, 341, 325, 791,
    791, 1123, 1122, -703, 1105, 1045, -719, 865, 865, 790, 790, 774, 774, 1104, 1029, 338, 293,
    323, 308, -799, -815, 833, 788, 772, 818, 803, 816, 322, 292, 307, 320, 561, 531, 515, 546,
    289, 274, 288, 258, -251, -525, -605, -685, -765, -831, -846, 1298, 1057, 1057, 1312, 1282,
    785, 785, 785, 785, 784, 784, 784, 784, 769, 769, 769, 769, 512, 512, 512, 512, 512, 512, 512,
    512, 1399, 1398, 1383, 1367, 1382, 1396, 1351, -511, 1381, 1366, 1139, 1139, 1079, 1079, 1124,
    1124, 1364, 1349, 1363, 1333, 882, 882, 882, 882, 807, 807, 807, 807, 1094, 1094, 1136, 1136,
    373, 341, 535, 535, 881, 775, 867, 822, 774, -591, 324, 338, -671, 849, 550, 550, 866, 864,
    609, 609, 293, 336, 534, 534, 789, 835, 773, -751, 834, 804, 308, 307, 833, 788, 832, 772, 562,
    562, 547, 547, 305, 275, 560, 515, 290, 290, -252, -397, -477, -557, -622, -653, -719, -735,
    -750, 1329, 1299, 1314, 1057, 1057, 1042, 1042, 1312, 1282, 1024, 1024, 785, 785, 785, 785,
    784, 784, 784, 784, 769, 769, 769, 769, -383, 1127, 1141, 1111, 1126, 1140, 1095, 1110, 869,
    869, 883, 883, 1079, 1109, 882, 882, 375, 374, 807, 868, 838, 881, 791, -463, 867, 822, 368,
    263, 852, 837, 836, -543, 610, 610, 550, 550, 352, 336, 534, 534, 865, 774, 851, 821, 850, 805,
    593, 533, 579, 564, 773, 832, 578, 578, 548, 548, 577, 577, 307, 276, 306, 291, 516, 560, 259,
    259, -250, -2107, -2507, -2764, -2909, -2974, -3007, -3023, 1041, 1041, 1040, 1040, 769, 769,
    769, 769, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, -767,
    -1052, -1213, -1277, -1358, -1405, -1469, -1535, -1550, -1582, -1614, -1647, -1662, -1694,
    -1726, -1759, -1774, -1807, -1822, -1854, -1886, 1565, -1919, -1935, -1951, -1967, 1731, 1730,
    1580, 1717, -1983, 1729, 1564, -1999, 1548, -2015, -2031, 1715, 1595, -2047, 1714, -2063, 1610,
    -2079, 1609, -2095, 1323, 1323, 1457, 1457, 1307, 1307, 1712, 1547, 1641, 1700, 1699, 1594,
    1685, 1625, 1442, 1442, 1322, 1322, -780, -973, -910, 1279, 1278, 1277, 1262, 1276, 1261, 1275,
    1215, 1260, 1229, -959, 974, 974, 989, 989, -943, 735, 478, 478, 495, 463, 506, 414, -1039,
    1003, 958, 1017, 927, 942, 987, 957, 431, 476, 1272, 1167, 1228, -1183, 1256, -1199, 895, 895,
    941, 941, 1242, 1227, 1212, 1135, 1014, 1014, 490, 489, 503, 487, 910, 1013, 985, 925, 863,
    894, 970, 955, 1012, 847, -1343, 831, 755, 755, 984, 909, 428, 366, 754, 559, -1391, 752, 486,
    457, 924, 997, 698, 698, 983, 893, 740, 740, 908, 877, 739, 739, 667, 667, 953, 938, 497, 287,
    271, 271, 683, 606, 590, 712, 726, 574, 302, 302, 738, 736, 481, 286, 526, 725, 605, 711, 636,
    724, 696, 651, 589, 681, 666, 710, 364, 467, 573, 695, 466, 466, 301, 465, 379, 379, 709, 604,
    665, 679, 316, 316, 634, 633, 436, 436, 464, 269, 424, 394, 452, 332, 438, 363, 347, 408, 393,
    448, 331, 422, 362, 407, 392, 421, 346, 406, 391, 376, 375, 359, 1441, 1306, -2367, 1290,
    -2383, 1337, -2399, -2415, 1426, 1321, -2431, 1411, 1336, -2447, -2463, -2479, 1169, 1169,
    1049, 1049, 1424, 1289, 1412, 1352, 1319, -2495, 1154, 1154, 1064, 1064, 1153, 1153, 416, 390,
    360, 404, 403, 389, 344, 374, 373, 343, 358, 372, 327, 357, 342, 311, 356, 326, 1395, 1394,
    1137, 1137, 1047, 1047, 1365, 1392, 1287, 1379, 1334, 1364, 1349, 1378, 1318, 1363, 792, 792,
    792, 792, 1152, 1152, 1032, 1032, 1121, 1121, 1046, 1046, 1120, 1120, 1030, 1030, -2895, 1106,
    1061, 1104, 849, 849, 789, 789, 1091, 1076, 1029, 1090, 1060, 1075, 833, 833, 309, 324, 532,
    532, 832, 772, 818, 803, 561, 561, 531, 560, 515, 546, 289, 274, 288, 258, -250, -1179, -1579,
    -1836, -1996, -2124, -2253, -2333, -2413, -2477, -2542, -2574, -2607, -2622, -2655, 1314, 1313,
    1298, 1312, 1282, 785, 785, 785, 785, 1040, 1040, 1025, 1025, 768, 768, 768, 768, -766, -798,
    -830, -862, -895, -911, -927, -943, -959, -975, -991, -1007, -1023, -1039, -1055, -1070, 1724,
    1647, -1103, -1119, 1631, 1767, 1662, 1738, 1708, 1723, -1135, 1780, 1615, 1779, 1599, 1677,
    1646, 1778, 1583, -1151, 1777, 1567, 1737, 1692, 1765, 1722, 1707, 1630, 1751, 1661, 1764,
    1614, 1736, 1676, 1763, 1750, 1645, 1598, 1721, 1691, 1762, 1706, 1582, 1761, 1566, -1167,
    1749, 1629, 767, 766, 751, 765, 494, 494, 735, 764, 719, 749, 734, 763, 447, 447, 748, 718,
    477, 506, 431, 491, 446, 476, 461, 505, 415, 430, 475, 445, 504, 399, 460, 489, 414, 503, 383,
    474, 429, 459, 502, 502, 746, 752, 488, 398, 501, 473, 413, 472, 486, 271, 480, 270, -1439,
    -1455, 1357, -1471, -1487, -1503, 1341, 1325, -1519, 1489, 1463, 1403, 1309, -1535, 1372, 1448,
    1418, 1476, 1356, 1462, 1387, -1551, 1475, 1340, 1447, 1402, 1386, -1567, 1068, 1068, 1474,
    1461, 455, 380, 468, 440, 395, 425, 410, 454, 364, 467, 466, 464, 453, 269, 409, 448, 268, 432,
    1371, 1473, 1432, 1417, 1308, 1460, 1355, 1446, 1459, 1431, 1083, 1083, 1401, 1416, 1458, 1445,
    1067, 1067, 1370, 1457, 1051, 1051, 1291, 1430, 1385, 1444, 1354, 1415, 1400, 1443, 1082, 1082,
    1173, 1113, 1186, 1066, 1185, 1050, -1967, 1158, 1128, 1172, 1097, 1171, 1081, -1983, 1157,
    1112, 416, 266, 375, 400, 1170, 1142, 1127, 1065, 793, 793, 1169, 1033, 1156, 1096, 1141, 1111,
    1155, 1080, 1126, 1140, 898, 898, 808, 808, 897, 897, 792, 792, 1095, 1152, 1032, 1125, 1110,
    1139, 1079, 1124, 882, 807, 838, 881, 853, 791, -2319, 867, 368, 263, 822, 852, 837, 866, 806,
    865, -2399, 851, 352, 262, 534, 534, 821, 836, 594, 594, 549, 549, 593, 593, 533, 533, 848,
    773, 579, 579, 564, 578, 548, 563, 276, 276, 577, 576, 306, 291, 516, 560, 305, 305, 275, 259,
    -251, -892, -2058, -2620, -2828, -2957, -3023, -3039, 1041, 1041, 1040, 1040, 769, 769, 769,
    769, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, 256, -511,
    -527, -543, -559, 1530, -575, -591, 1528, 1527, 1407, 1526, 1391, 1023, 1023, 1023, 1023, 1525,
    1375, 1268, 1268, 1103, 1103, 1087, 1087, 1039, 1039, 1523, -604, 815, 815, 815, 815, 510, 495,
    509, 479, 508, 463, 507, 447, 431, 505, 415, 399, -734, -782, 1262, -815, 1259, 1244, -831,
    1258, 1228, -847, -863, 1196, -879, 1253, 987, 987, 748, -767, 493, 493, 462, 477, 414, 414,
    686, 669, 478, 446, 461, 445, 474, 429, 487, 458, 412, 471, 1266, 1264, 1009, 1009, 799, 799,
    -1019, -1276, -1452, -1581, -1677, -1757, -1821, -1886, -1933, -1997, 1257, 1257, 1483, 1468,
    1512, 1422, 1497, 1406, 1467, 1496, 1421, 1510, 1134, 1134, 1225, 1225, 1466, 1451, 1374, 1405,
    1252, 1252, 1358, 1480, 1164, 1164, 1251, 1251, 1238, 1238, 1389, 1465, -1407, 1054, 1101,
    -1423, 1207, -1439, 830, 830, 1248, 1038, 1237, 1117, 1223, 1148, 1236, 1208, 411, 426, 395,
    410, 379, 269, 1193, 1222, 1132, 1235, 1221, 1116, 976, 976, 1192, 1162, 1177, 1220, 1131,
    1191, 963, 963, -1647, 961, 780, -1663, 558, 558, 994, 993, 437, 408, 393, 407, 829, 978, 813,
    797, 947, -1743, 721, 721, 377, 392, 844, 950, 828, 890, 706, 706, 812, 859, 796, 960, 948,
    843, 934, 874, 571, 571, -1919, 690, 555, 689, 421, 346, 539, 539, 944, 779, 918, 873, 932,
    842, 903, 888, 570, 570, 931, 917, 674, 674, -2575, 1562, -2591, 1609, -2607, 1654, 1322, 1322,
    1441, 1441, 1696, 1546, 1683, 1593, 1669, 1624, 1426, 1426, 1321, 1321, 1639, 1680, 1425, 1425,
    1305, 1305, 1545, 1668, 1608, 1623, 1667, 1592, 1638, 1666, 1320, 1320, 1652, 1607, 1409, 1409,
    1304, 1304, 1288, 1288, 1664, 1637, 1395, 1395, 1335, 1335, 1622, 1636, 1394, 1394, 1319, 1319,
    1606, 1621, 1392, 1392, 1137, 1137, 1137, 1137, 345, 390, 360, 375, 404, 373, 1047, -2751,
    -2767, -2783, 1062, 1121, 1046, -2799, 1077, -2815, 1106, 1061, 789, 789, 1105, 1104, 263, 355,
    310, 340, 325, 354, 352, 262, 339, 324, 1091, 1076, 1029, 1090, 1060, 1075, 833, 833, 788, 788,
    1088, 1028, 818, 818, 803, 803, 561, 561, 531, 531, 816, 771, 546, 546, 289, 274, 288, 258,
    -253, -317, -381, -446, -478, -509, 1279, 1279, -811, -1179, -1451, -1756, -1900, -2028, -2189,
    -2253, -2333, -2414, -2445, -2511, -2526, 1313, 1298, -2559, 1041, 1041, 1040, 1040, 1025,
    1025, 1024, 1024, 1022, 1007, 1021, 991, 1020, 975, 1019, 959, 687, 687, 1018, 1017, 671, 671,
    655, 655, 1016, 1015, 639, 639, 758, 758, 623, 623, 757, 607, 756, 591, 755, 575, 754, 559,
    543, 543, 1009, 783, -575, -621, -685, -749, 496, -590, 750, 749, 734, 748, 974, 989, 1003,
    958, 988, 973, 1002, 942, 987, 957, 972, 1001, 926, 986, 941, 971, 956, 1000, 910, 985, 925,
    999, 894, 970, -1071, -1087, -1102, 1390, -1135, 1436, 1509, 1451, 1374, -1151, 1405, 1358,
    1480, 1420, -1167, 1507, 1494, 1389, 1342, 1465, 1435, 1450, 1326, 1505, 1310, 1493, 1373,
    1479, 1404, 1492, 1464, 1419, 428, 443, 472, 397, 736, 526, 464, 464, 486, 457, 442, 471, 484,
    482, 1357, 1449, 1434, 1478, 1388, 1491, 1341, 1490, 1325, 1489, 1463, 1403, 1309, 1477, 1372,
    1448, 1418, 1433, 1476, 1356, 1462, 1387, -1439, 1475, 1340, 1447, 1402, 1474, 1324, 1461,
    1371, 1473, 269, 448, 1432, 1417, 1308, 1460, -1711, 1459, -1727, 1441, 1099, 1099, 1446, 1386,
    1431, 1401, -1743, 1289, 1083, 1083, 1160, 1160, 1458, 1445, 1067, 1067, 1370, 1457, 1307,
    1430, 1129, 1129, 1098, 1098, 268, 432, 267, 416, 266, 400, -1887, 1144, 1187, 1082, 1173,
    1113, 1186, 1066, 1050, 1158, 1128, 1143, 1172, 1097, 1171, 1081, 420, 391, 1157, 1112, 1170,
    1142, 1127, 1065, 1169, 1049, 1156, 1096, 1141, 1111, 1155, 1080, 1126, 1154, 1064, 1153, 1140,
    1095, 1048, -2159, 1125, 1110, 1137, -2175, 823, 823, 1139, 1138, 807, 807, 384, 264, 368, 263,
    868, 838, 853, 791, 867, 822, 852, 837, 866, 806, 865, 790, -2319, 851, 821, 836, 352, 262,
    850, 805, 849, -2399, 533, 533, 835, 820, 336, 261, 578, 548, 563, 577, 532, 532, 832, 772,
    562, 562, 547, 547, 305, 275, 560, 515, 290, 290, 288, 258,
];

/// Translation of `drmp3_L3_huffman()`: `dst` is a channel's 576 values.
fn l3_huffman(
    dst: &mut [f32],
    bs: &mut Drmp3Bs<'_>,
    gr_info: &Drmp3L3GrInfo,
    scf: &[f32],
    layer3gr_limit: i32,
) {
    static TAB32: [u8; 28] = [
        130, 162, 193, 209, 44, 28, 76, 140, 9, 9, 9, 9, 9, 9, 9, 9, 190, 254, 222, 238, 126, 94,
        157, 157, 109, 61, 173, 205,
    ];
    static TAB33: [u8; 16] = [
        252, 236, 220, 204, 188, 172, 156, 140, 124, 108, 92, 76, 60, 44, 28, 12,
    ];
    static TABINDEX: [i16; 2 * 16] = [
        0, 32, 64, 98, 0, 132, 180, 218, 292, 364, 426, 538, 648, 746, 0, 1126, 1460, 1460, 1460,
        1460, 1460, 1460, 1460, 1460, 1842, 1842, 1842, 1842, 1842, 1842, 1842, 1842,
    ];
    static G_LINBITS: [u8; 32] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 6, 8, 10, 13, 4, 5, 6, 7, 8, 9,
        11, 13,
    ];

    let buf = bs.buf;
    let byte = |i: usize| buf.get(i).copied().unwrap_or(0) as u32;
    let tab = |i: isize| TABS.get(i as usize).copied().unwrap_or(0) as i32;
    fn put(d: &mut [f32], i: usize, v: f32) {
        if let Some(x) = d.get_mut(i) {
            *x = v;
        }
    }

    let mut one: f32 = 0.0;
    let mut ireg = 0usize;
    let mut big_val_cnt = gr_info.big_values as i32;
    let sfb = gr_info.sfbtab;
    let mut sfbi = 0usize;
    let mut scfi = 0usize;
    let mut d = 0usize;
    let mut bs_next_ptr = (bs.pos / 8) as usize;
    let mut bs_cache: u32 = ((((byte(bs_next_ptr) * 256 + byte(bs_next_ptr + 1)) * 256
        + byte(bs_next_ptr + 2))
    .wrapping_mul(256))
    .wrapping_add(byte(bs_next_ptr + 3)))
    .wrapping_shl((bs.pos & 7) as u32);
    let mut pairs_to_decode: i32;
    let mut np: i32;
    let mut bs_sh: i32 = (bs.pos & 7) - 8;
    bs_next_ptr += 4;

    macro_rules! peek_bits {
        ($n:expr) => {
            bs_cache.wrapping_shr((32 - ($n)) as u32)
        };
    }
    macro_rules! flush_bits {
        ($n:expr) => {{
            bs_cache = bs_cache.wrapping_shl(($n) as u32);
            bs_sh += $n;
        }};
    }
    macro_rules! check_bits {
        () => {
            while bs_sh >= 0 {
                bs_cache |= byte(bs_next_ptr).wrapping_shl(bs_sh as u32);
                bs_next_ptr += 1;
                bs_sh -= 8;
            }
        };
    }
    macro_rules! bspos {
        () => {
            (bs_next_ptr as i32) * 8 - 24 + bs_sh
        };
    }

    while big_val_cnt > 0 {
        // (a fourth region would read past the three tables; upstream
        // reads the next field.)
        let tab_num = gr_info.table_select.get(ireg).copied().unwrap_or(0) as usize;
        let mut sfb_cnt = gr_info.region_count.get(ireg).copied().unwrap_or(0) as i32;
        ireg += 1;
        let codebook = TABINDEX[tab_num] as isize;
        let linbits = G_LINBITS[tab_num] as i32;
        if linbits != 0 {
            loop {
                np = sfb.get(sfbi).copied().unwrap_or(0) as i32 / 2;
                sfbi += 1;
                pairs_to_decode = big_val_cnt.min(np);
                one = scf.get(scfi).copied().unwrap_or(0.0);
                scfi += 1;
                loop {
                    let mut w = 5;
                    let mut leaf = tab(codebook + peek_bits!(w) as isize);
                    while leaf < 0 {
                        flush_bits!(w);
                        w = leaf & 7;
                        leaf = tab(codebook + peek_bits!(w) as isize - (leaf >> 3) as isize);
                    }
                    flush_bits!(leaf >> 8);

                    for _ in 0..2 {
                        let mut lsb = leaf & 0x0F;
                        if lsb == 15 {
                            lsb += peek_bits!(linbits) as i32;
                            flush_bits!(linbits);
                            check_bits!();
                            put(
                                dst,
                                d,
                                one * l3_pow_43(lsb)
                                    * if (bs_cache as i32) < 0 { -1.0 } else { 1.0 },
                            );
                        } else {
                            put(
                                dst,
                                d,
                                G_DRMP3_POW43[(16 + lsb - 16 * (bs_cache >> 31) as i32) as usize]
                                    * one,
                            );
                        }
                        flush_bits!(if lsb != 0 { 1 } else { 0 });
                        d += 1;
                        leaf >>= 4;
                    }
                    check_bits!();
                    pairs_to_decode -= 1;
                    if pairs_to_decode == 0 {
                        break;
                    }
                }
                big_val_cnt -= np;
                if big_val_cnt <= 0 {
                    break;
                }
                sfb_cnt -= 1;
                if sfb_cnt < 0 {
                    break;
                }
            }
        } else {
            loop {
                np = sfb.get(sfbi).copied().unwrap_or(0) as i32 / 2;
                sfbi += 1;
                pairs_to_decode = big_val_cnt.min(np);
                one = scf.get(scfi).copied().unwrap_or(0.0);
                scfi += 1;
                loop {
                    let mut w = 5;
                    let mut leaf = tab(codebook + peek_bits!(w) as isize);
                    while leaf < 0 {
                        flush_bits!(w);
                        w = leaf & 7;
                        leaf = tab(codebook + peek_bits!(w) as isize - (leaf >> 3) as isize);
                    }
                    flush_bits!(leaf >> 8);

                    for _ in 0..2 {
                        let lsb = leaf & 0x0F;
                        put(
                            dst,
                            d,
                            G_DRMP3_POW43[(16 + lsb - 16 * (bs_cache >> 31) as i32) as usize] * one,
                        );
                        flush_bits!(if lsb != 0 { 1 } else { 0 });
                        d += 1;
                        leaf >>= 4;
                    }
                    check_bits!();
                    pairs_to_decode -= 1;
                    if pairs_to_decode == 0 {
                        break;
                    }
                }
                big_val_cnt -= np;
                if big_val_cnt <= 0 {
                    break;
                }
                sfb_cnt -= 1;
                if sfb_cnt < 0 {
                    break;
                }
            }
        }
    }

    np = 1 - big_val_cnt;
    'count1: loop {
        let codebook_count1: &[u8] = if gr_info.count1_table != 0 {
            &TAB33
        } else {
            &TAB32
        };
        let mut leaf = codebook_count1[peek_bits!(4) as usize] as i32;
        if leaf & 8 == 0 {
            leaf = codebook_count1
                .get(
                    ((leaf >> 3) as u32)
                        .wrapping_add((bs_cache << 4).wrapping_shr((32 - (leaf & 3)) as u32))
                        as usize,
                )
                .copied()
                .unwrap_or(0) as i32;
        }
        flush_bits!(leaf & 7);
        if bspos!() > layer3gr_limit {
            break;
        }

        macro_rules! reload_scalefactor {
            () => {
                np -= 1;
                if np == 0 {
                    np = sfb.get(sfbi).copied().unwrap_or(0) as i32 / 2;
                    sfbi += 1;
                    if np == 0 {
                        break 'count1;
                    }
                    one = scf.get(scfi).copied().unwrap_or(0.0);
                    scfi += 1;
                }
            };
        }
        macro_rules! deq_count1 {
            ($s:expr) => {
                if leaf & (128 >> $s) != 0 {
                    put(dst, d + $s, if (bs_cache as i32) < 0 { -one } else { one });
                    flush_bits!(1);
                }
            };
        }
        reload_scalefactor!();
        deq_count1!(0);
        deq_count1!(1);
        reload_scalefactor!();
        deq_count1!(2);
        deq_count1!(3);
        check_bits!();
        d += 4;
    }

    bs.pos = layer3gr_limit;
}

/// Translation of `drmp3_L3_midside_stereo()`: left at `left`, right 576
/// later. (The SSE version does the same arithmetic.)
fn l3_midside_stereo(buf: &mut [f32], left: usize, n: i32) {
    let right = left + 576;
    for i in 0..n.max(0) as usize {
        let a = buf[left + i];
        let b = buf[right + i];
        buf[left + i] = a + b;
        buf[right + i] = a - b;
    }
}

/// Translation of `drmp3_L3_intensity_stereo_band()`.
fn l3_intensity_stereo_band(buf: &mut [f32], left: usize, n: i32, kl: f32, kr: f32) {
    for i in 0..n.max(0) as usize {
        buf[left + i + 576] = buf[left + i] * kr;
        buf[left + i] *= kl;
    }
}

/// Translation of `drmp3_L3_stereo_top_band()`.
fn l3_stereo_top_band(
    buf: &[f32],
    mut right: usize,
    sfb: &[u8],
    nbands: i32,
    max_band: &mut [i32; 3],
) {
    max_band[0] = -1;
    max_band[1] = -1;
    max_band[2] = -1;

    for i in 0..nbands.max(0) as usize {
        let mut k = 0;
        while k < sfb[i] as usize {
            if buf[right + k] != 0.0 || buf[right + k + 1] != 0.0 {
                max_band[i % 3] = i as i32;
                break;
            }
            k += 2;
        }
        right += sfb[i] as usize;
    }
}

/// Translation of `drmp3_L3_stereo_process()`.
fn l3_stereo_process(
    buf: &mut [f32],
    mut left: usize,
    ist_pos: &[u8],
    sfb: &[u8],
    hdr: &[u8],
    max_band: &[i32; 3],
    mpeg2_sh: i32,
) {
    static G_PAN: [f32; 7 * 2] = [
        0.0, 1.0, 0.21132487, 0.78867513, 0.36602540, 0.63397460, 0.5, 0.5, 0.63397460, 0.36602540,
        0.78867513, 0.21132487, 1.0, 0.0,
    ];
    let max_pos: u32 = if hdr_test_mpeg1(hdr) { 7 } else { 64 };

    let mut i = 0usize;
    while sfb[i] != 0 {
        let ipos = ist_pos[i] as u32;
        if (i as i32) > max_band[i % 3] && ipos < max_pos {
            let mut kl: f32;
            let mut kr: f32;
            let s: f32 = if hdr_test_ms_stereo(hdr) {
                1.41421356
            } else {
                1.0
            };
            if hdr_test_mpeg1(hdr) {
                kl = G_PAN[2 * ipos as usize];
                kr = G_PAN[2 * ipos as usize + 1];
            } else {
                kl = 1.0;
                kr = l3_ldexp_q2(1.0, (((ipos + 1) >> 1) << mpeg2_sh) as i32);
                if ipos & 1 != 0 {
                    kl = kr;
                    kr = 1.0;
                }
            }
            l3_intensity_stereo_band(buf, left, sfb[i] as i32, kl * s, kr * s);
        } else if hdr_test_ms_stereo(hdr) {
            l3_midside_stereo(buf, left, sfb[i] as i32);
        }
        left += sfb[i] as usize;
        i += 1;
    }
}

/// Translation of `drmp3_L3_intensity_stereo()`.
fn l3_intensity_stereo(buf: &mut [f32], ist_pos: &mut [u8], gr: &[Drmp3L3GrInfo], hdr: &[u8]) {
    let mut max_band = [0i32; 3];
    let n_sfb = gr[0].n_long_sfb as i32 + gr[0].n_short_sfb as i32;
    let max_blocks = if gr[0].n_short_sfb != 0 { 3 } else { 1 };

    l3_stereo_top_band(buf, 576, gr[0].sfbtab, n_sfb, &mut max_band);
    if gr[0].n_long_sfb != 0 {
        let m = max_band[0].max(max_band[1]).max(max_band[2]);
        max_band = [m; 3];
    }
    for i in 0..max_blocks {
        let default_pos = if hdr_test_mpeg1(hdr) { 3 } else { 0 };
        let itop = n_sfb - max_blocks + i;
        let prev = itop - max_blocks;
        ist_pos[itop as usize] = if max_band[i as usize] >= prev {
            default_pos
        } else {
            ist_pos[prev as usize]
        };
    }
    let mpeg2_sh = (gr[1].scalefac_compress & 1) as i32;
    l3_stereo_process(buf, 0, ist_pos, gr[0].sfbtab, hdr, &max_band, mpeg2_sh);
}

/// Translation of `drmp3_L3_reorder()`.
fn l3_reorder(grbuf: &mut [f32], scratch: &mut [f32], sfb: &[u8]) {
    let mut src = 0usize;
    let mut dst = 0usize;
    let mut si = 0usize;

    loop {
        let len = sfb[si] as usize;
        if len == 0 {
            break;
        }
        // FIXME (upstream): the 8 kHz mixed-block table's short bands cover
        // 528 samples, but only 504 follow the four long bands, so upstream
        // reads and writes past the channel's buffer (into the other
        // channel's, or uninitialized memory). Stop at its end instead.
        if src + 3 * len > grbuf.len() {
            break;
        }
        for _ in 0..len {
            scratch[dst] = grbuf[src];
            scratch[dst + 1] = grbuf[src + len];
            scratch[dst + 2] = grbuf[src + 2 * len];
            dst += 3;
            src += 1;
        }
        si += 3;
        src += 2 * len;
    }
    grbuf[..dst].copy_from_slice(&scratch[..dst]);
}

/// Translation of `drmp3_L3_antialias()`. (The SSE version does the same
/// arithmetic.)
fn l3_antialias(grbuf: &mut [f32], nbands: i32) {
    static G_AA: [[f32; 8]; 2] = [
        [
            0.85749293, 0.88174200, 0.94962865, 0.98331459, 0.99551782, 0.99916056, 0.99989920,
            0.99999316,
        ],
        [
            0.51449576, 0.47173197, 0.31337745, 0.18191320, 0.09457419, 0.04096558, 0.01419856,
            0.00369997,
        ],
    ];

    let mut g = 0usize;
    let mut nbands = nbands;
    while nbands > 0 {
        for i in 0..8 {
            let u = grbuf[g + 18 + i];
            let d = grbuf[g + 17 - i];
            grbuf[g + 18 + i] = u * G_AA[0][i] - d * G_AA[1][i];
            grbuf[g + 17 - i] = u * G_AA[1][i] + d * G_AA[0][i];
        }
        nbands -= 1;
        g += 18;
    }
}

/// Translation of `drmp3_L3_dct3_9()`.
fn l3_dct3_9(y: &mut [f32; 9]) {
    let (mut s0, mut s1, mut s2, mut s3, mut s4, mut s5, mut s6, mut s7, mut s8);
    let (mut t0, mut t2, mut t4);

    s0 = y[0];
    s2 = y[2];
    s4 = y[4];
    s6 = y[6];
    s8 = y[8];
    t0 = s0 + s6 * 0.5;
    s0 -= s6;
    t4 = (s4 + s2) * 0.93969262;
    t2 = (s8 + s2) * 0.76604444;
    s6 = (s4 - s8) * 0.17364818;
    s4 += s8 - s2;

    s2 = s0 - s4 * 0.5;
    y[4] = s4 + s0;
    s8 = t0 - t2 + s6;
    s0 = t0 - t4 + t2;
    s4 = t0 + t4 - s6;

    s1 = y[1];
    s3 = y[3];
    s5 = y[5];
    s7 = y[7];

    s3 *= 0.86602540;
    t0 = (s5 + s1) * 0.98480775;
    t4 = (s5 - s7) * 0.34202014;
    t2 = (s1 + s7) * 0.64278761;
    s1 = (s1 - s5 - s7) * 0.86602540;

    s5 = t0 - s3 - t2;
    s7 = t4 - s3 - t0;
    s3 = t4 + s3 - t2;

    y[0] = s4 - s7;
    y[1] = s2 + s1;
    y[2] = s0 - s3;
    y[3] = s8 + s5;
    y[5] = s8 - s5;
    y[6] = s0 + s3;
    y[7] = s2 - s1;
    y[8] = s4 + s7;
}

/// Translation of `drmp3_L3_imdct36()`. (The SSE version does the same
/// arithmetic.)
fn l3_imdct36(grbuf: &mut [f32], overlap: &mut [f32], window: &[f32; 18], nbands: i32) {
    static G_TWID9: [f32; 18] = [
        0.73727734, 0.79335334, 0.84339145, 0.88701083, 0.92387953, 0.95371695, 0.97629601,
        0.99144486, 0.99904822, 0.67559021, 0.60876143, 0.53729961, 0.46174861, 0.38268343,
        0.30070580, 0.21643961, 0.13052619, 0.04361938,
    ];

    for j in 0..nbands.max(0) as usize {
        let g = &mut grbuf[j * 18..j * 18 + 18];
        let ov = &mut overlap[j * 9..j * 9 + 9];
        let mut co = [0.0f32; 9];
        let mut si = [0.0f32; 9];
        co[0] = -g[0];
        si[0] = g[17];
        for i in 0..4 {
            si[8 - 2 * i] = g[4 * i + 1] - g[4 * i + 2];
            co[1 + 2 * i] = g[4 * i + 1] + g[4 * i + 2];
            si[7 - 2 * i] = g[4 * i + 4] - g[4 * i + 3];
            co[2 + 2 * i] = -(g[4 * i + 3] + g[4 * i + 4]);
        }
        l3_dct3_9(&mut co);
        l3_dct3_9(&mut si);

        si[1] = -si[1];
        si[3] = -si[3];
        si[5] = -si[5];
        si[7] = -si[7];

        for i in 0..9 {
            let ovl = ov[i];
            let sum = co[i] * G_TWID9[9 + i] + si[i] * G_TWID9[i];
            ov[i] = co[i] * G_TWID9[i] - si[i] * G_TWID9[9 + i];
            g[i] = ovl * window[i] - sum * window[9 + i];
            g[17 - i] = ovl * window[9 + i] + sum * window[i];
        }
    }
}

/// Translation of `drmp3_L3_idct3()`.
fn l3_idct3(x0: f32, x1: f32, x2: f32) -> [f32; 3] {
    let m1 = x1 * 0.86602540;
    let a1 = x0 - x2 * 0.5;
    [a1 + m1, x0 + x2, a1 - m1]
}

/// Translation of `drmp3_L3_imdct12()`.
fn l3_imdct12(x: &[f32], dst: &mut [f32], overlap: &mut [f32]) {
    static G_TWID3: [f32; 6] = [
        0.79335334, 0.92387953, 0.99144486, 0.60876143, 0.38268343, 0.13052619,
    ];

    let co = l3_idct3(-x[0], x[6] + x[3], x[12] + x[9]);
    let mut si = l3_idct3(x[15], x[12] - x[9], x[6] - x[3]);
    si[1] = -si[1];

    for i in 0..3 {
        let ovl = overlap[i];
        let sum = co[i] * G_TWID3[3 + i] + si[i] * G_TWID3[i];
        overlap[i] = co[i] * G_TWID3[i] - si[i] * G_TWID3[3 + i];
        dst[i] = ovl * G_TWID3[2 - i] - sum * G_TWID3[5 - i];
        dst[5 - i] = ovl * G_TWID3[5 - i] + sum * G_TWID3[2 - i];
    }
}

/// Translation of `drmp3_L3_imdct_short()`.
fn l3_imdct_short(grbuf: &mut [f32], overlap: &mut [f32], nbands: i32) {
    for b in 0..nbands.max(0) as usize {
        let g = &mut grbuf[b * 18..b * 18 + 18];
        let ov = &mut overlap[b * 9..b * 9 + 9];
        let mut tmp = [0.0f32; 18];
        tmp.copy_from_slice(g);
        g[..6].copy_from_slice(&ov[..6]);
        let (o0, o6) = ov.split_at_mut(6);
        l3_imdct12(&tmp, &mut g[6..12], o6);
        l3_imdct12(&tmp[1..], &mut g[12..18], o6);
        l3_imdct12(&tmp[2..], o0, o6);
    }
}

/// Translation of `drmp3_L3_change_sign()`.
fn l3_change_sign(grbuf: &mut [f32]) {
    let mut g = 18usize;
    let mut b = 0;
    while b < 32 {
        let mut i = 1;
        while i < 18 {
            grbuf[g + i] = -grbuf[g + i];
            i += 2;
        }
        b += 2;
        g += 36;
    }
}

/// Translation of `drmp3_L3_imdct_gr()`.
fn l3_imdct_gr(grbuf: &mut [f32], overlap: &mut [f32], block_type: u8, n_long_bands: u32) {
    static G_MDCT_WINDOW: [[f32; 18]; 2] = [
        [
            0.99904822, 0.99144486, 0.97629601, 0.95371695, 0.92387953, 0.88701083, 0.84339145,
            0.79335334, 0.73727734, 0.04361938, 0.13052619, 0.21643961, 0.30070580, 0.38268343,
            0.46174861, 0.53729961, 0.60876143, 0.67559021,
        ],
        [
            1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.99144486, 0.92387953, 0.79335334, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.13052619, 0.38268343, 0.60876143,
        ],
    ];
    let mut g = 0usize;
    let mut o = 0usize;
    if n_long_bands != 0 {
        l3_imdct36(grbuf, overlap, &G_MDCT_WINDOW[0], n_long_bands as i32);
        g += 18 * n_long_bands as usize;
        o += 9 * n_long_bands as usize;
    }
    if block_type == DRMP3_SHORT_BLOCK_TYPE {
        l3_imdct_short(&mut grbuf[g..], &mut overlap[o..], 32 - n_long_bands as i32);
    } else {
        l3_imdct36(
            &mut grbuf[g..],
            &mut overlap[o..],
            &G_MDCT_WINDOW[(block_type == DRMP3_STOP_BLOCK_TYPE) as usize],
            32 - n_long_bands as i32,
        );
    }
}

/// Translation of `drmp3_L3_save_reservoir()`.
fn l3_save_reservoir(h: &mut Drmp3dec) {
    let s = &h.scratch;
    let mut pos = ((s.bs_pos + 7) as u32 / 8) as i32;
    let mut remains = ((s.bs_limit as u32 / 8).wrapping_sub(pos as u32)) as i32;
    if remains > DRMP3_MAX_BITRESERVOIR_BYTES as i32 {
        pos += remains - DRMP3_MAX_BITRESERVOIR_BYTES as i32;
        remains = DRMP3_MAX_BITRESERVOIR_BYTES as i32;
    }
    if remains > 0 {
        let (p, r) = (pos as usize, remains as usize);
        h.reserv_buf[..r].copy_from_slice(&h.scratch.maindata[p..p + r]);
    }
    h.reserv = remains;
}

/// Translation of `drmp3_L3_restore_reservoir()`: the frame's bit reader is
/// over `frame`, at `pos` of `limit`.
fn l3_restore_reservoir(
    h: &mut Drmp3dec,
    frame: &[u8],
    pos: i32,
    limit: i32,
    main_data_begin: i32,
) -> bool {
    let frame_bytes = ((limit - pos) / 8).max(0) as usize;
    let bytes_have = h.reserv.min(main_data_begin).max(0) as usize;
    let from = (h.reserv - main_data_begin).max(0) as usize;
    h.scratch.maindata[..bytes_have].copy_from_slice(&h.reserv_buf[from..from + bytes_have]);
    let start = (pos / 8) as usize;
    let n = frame_bytes
        .min(MAINDATA_SIZE - bytes_have)
        .min(frame.len().saturating_sub(start));
    h.scratch.maindata[bytes_have..bytes_have + n].copy_from_slice(&frame[start..start + n]);
    h.scratch.bs_pos = 0;
    h.scratch.bs_limit = ((bytes_have + frame_bytes) * 8) as i32;
    h.reserv >= main_data_begin
}

/// Translation of `drmp3_L3_decode()`: the granule's info is `gr_info[gr..]`.
fn l3_decode(h: &mut Drmp3dec, gr: usize, nch: i32) {
    let header = h.header;
    let s = &mut h.scratch;
    {
        let mut bs = Drmp3Bs {
            buf: &s.maindata,
            pos: s.bs_pos,
            limit: s.bs_limit,
        };

        for ch in 0..nch as usize {
            let layer3gr_limit = bs.pos + s.gr_info[gr + ch].part_23_length as i32;
            l3_decode_scalefactors(
                &header,
                &mut s.ist_pos[ch],
                &mut bs,
                &s.gr_info[gr + ch],
                &mut s.scf,
                ch as i32,
            );
            l3_huffman(
                &mut s.grbuf[ch * 576..ch * 576 + 576],
                &mut bs,
                &s.gr_info[gr + ch],
                &s.scf,
                layer3gr_limit,
            );
        }
        s.bs_pos = bs.pos;
    }

    if hdr_test_i_stereo(&header) {
        l3_intensity_stereo(&mut s.grbuf, &mut s.ist_pos[1], &s.gr_info[gr..], &header);
    } else if hdr_is_ms_stereo(&header) {
        l3_midside_stereo(&mut s.grbuf, 0, 576);
    }

    for ch in 0..nch as usize {
        let gi = s.gr_info[gr + ch];
        let mut aa_bands = 31;
        let n_long_bands = (if gi.mixed_block_flag != 0 { 2 } else { 0 })
            << ((hdr_get_my_sample_rate(&header) == 2) as i32);

        let g = &mut s.grbuf[ch * 576..ch * 576 + 576];
        if gi.n_short_sfb != 0 {
            aa_bands = n_long_bands - 1;
            l3_reorder(
                &mut g[n_long_bands as usize * 18..],
                &mut s.syn,
                &gi.sfbtab[gi.n_long_sfb as usize..],
            );
        }

        l3_antialias(g, aa_bands);
        l3_imdct_gr(
            g,
            &mut h.mdct_overlap[ch],
            gi.block_type,
            n_long_bands as u32,
        );
        l3_change_sign(g);
    }
}

/// Translation of `drmp3d_DCT_II()`, with the SSE version's sums (see the
/// module docs).
fn d_dct_ii(grbuf: &mut [f32], n: i32) {
    static G_SEC: [f32; 24] = [
        10.19000816,
        0.50060302,
        0.50241929,
        3.40760851,
        0.50547093,
        0.52249861,
        2.05778098,
        0.51544732,
        0.56694406,
        1.48416460,
        0.53104258,
        0.64682180,
        1.16943991,
        0.55310392,
        0.78815460,
        0.97256821,
        0.58293498,
        1.06067765,
        0.83934963,
        0.62250412,
        1.72244716,
        0.74453628,
        0.67480832,
        5.10114861,
    ];
    for k in 0..n.max(0) as usize {
        let mut t = [[0.0f32; 8]; 4];
        let y = k;

        for i in 0..8 {
            let x0 = grbuf[y + i * 18];
            let x1 = grbuf[y + (15 - i) * 18];
            let x2 = grbuf[y + (16 + i) * 18];
            let x3 = grbuf[y + (31 - i) * 18];
            let t0 = x0 + x3;
            let t1 = x1 + x2;
            let t2 = (x1 - x2) * G_SEC[3 * i];
            let t3 = (x0 - x3) * G_SEC[3 * i + 1];
            t[0][i] = t0 + t1;
            t[1][i] = (t0 - t1) * G_SEC[3 * i + 2];
            t[2][i] = t3 + t2;
            t[3][i] = (t3 - t2) * G_SEC[3 * i + 2];
        }
        for x in t.iter_mut() {
            let (mut x0, mut x1, mut x2, mut x3, mut x4, mut x5, mut x6, mut x7) =
                (x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]);
            let mut xt;
            xt = x0 - x7;
            x0 += x7;
            x7 = x1 - x6;
            x1 += x6;
            x6 = x2 - x5;
            x2 += x5;
            x5 = x3 - x4;
            x3 += x4;
            x4 = x0 - x3;
            x0 += x3;
            x3 = x1 - x2;
            x1 += x2;
            x[0] = x0 + x1;
            x[4] = (x0 - x1) * 0.70710677;
            x5 += x6;
            x6 = (x6 + x7) * 0.70710677;
            x7 += xt;
            x3 = (x3 + x4) * 0.70710677;
            x5 -= x7 * 0.198912367; /* rotate by PI/8 */
            x7 += x5 * 0.382683432;
            x5 -= x7 * 0.198912367;
            x0 = xt - x6;
            xt += x6;
            x[1] = (xt + x7) * 0.50979561;
            x[2] = (x4 + x3) * 0.54119611;
            x[3] = (x0 - x5) * 0.60134488;
            x[5] = (x0 + x5) * 0.89997619;
            x[6] = (x4 - x3) * 1.30656302;
            x[7] = (xt - x7) * 2.56291556;
        }
        let mut yy = y;
        for i in 0..7 {
            // (the SSE version adds `t[3][i] + t[3][i + 1]` first; the
            // scalar one adds them to `t[2]` in turn.)
            let s = t[3][i] + t[3][i + 1];
            grbuf[yy] = t[0][i];
            grbuf[yy + 18] = t[2][i] + s;
            grbuf[yy + 2 * 18] = t[1][i] + t[1][i + 1];
            grbuf[yy + 3 * 18] = t[2][1 + i] + s;
            yy += 4 * 18;
        }
        grbuf[yy] = t[0][7];
        grbuf[yy + 18] = t[2][7] + t[3][7];
        grbuf[yy + 2 * 18] = t[1][7];
        grbuf[yy + 3 * 18] = t[3][7];
    }
}

/// Translation of `drmp3d_scale_pcm()` (float output).
fn d_scale_pcm(sample: f32) -> f32 {
    sample * (1.0f32 / 32768.0)
}

/// Translation of `drmp3d_synth_pair()`.
fn d_synth_pair(pcm: &mut [f32], p: usize, nch: usize, z: &[f32], mut zo: usize) {
    let mut a;
    a = (z[zo + 14 * 64] - z[zo]) * 29.0;
    a += (z[zo + 64] + z[zo + 13 * 64]) * 213.0;
    a += (z[zo + 12 * 64] - z[zo + 2 * 64]) * 459.0;
    a += (z[zo + 3 * 64] + z[zo + 11 * 64]) * 2037.0;
    a += (z[zo + 10 * 64] - z[zo + 4 * 64]) * 5153.0;
    a += (z[zo + 5 * 64] + z[zo + 9 * 64]) * 6574.0;
    a += (z[zo + 8 * 64] - z[zo + 6 * 64]) * 37489.0;
    a += z[zo + 7 * 64] * 75038.0;
    pcm[p] = d_scale_pcm(a);

    zo += 2;
    a = z[zo + 14 * 64] * 104.0;
    a += z[zo + 12 * 64] * 1567.0;
    a += z[zo + 10 * 64] * 9727.0;
    a += z[zo + 8 * 64] * 64019.0;
    a += z[zo + 6 * 64] * -9975.0;
    a += z[zo + 4 * 64] * -45.0;
    a += z[zo + 2 * 64] * 146.0;
    a += z[zo] * -5.0;
    pcm[p + 16 * nch] = d_scale_pcm(a);
}

static G_WIN: [f32; 240] = [
    -1.0, 26.0, -31.0, 208.0, 218.0, 401.0, -519.0, 2063.0, 2000.0, 4788.0, -5517.0, 7134.0,
    5959.0, 35640.0, -39336.0, 74992.0, -1.0, 24.0, -35.0, 202.0, 222.0, 347.0, -581.0, 2080.0,
    1952.0, 4425.0, -5879.0, 7640.0, 5288.0, 33791.0, -41176.0, 74856.0, -1.0, 21.0, -38.0, 196.0,
    225.0, 294.0, -645.0, 2087.0, 1893.0, 4063.0, -6237.0, 8092.0, 4561.0, 31947.0, -43006.0,
    74630.0, -1.0, 19.0, -41.0, 190.0, 227.0, 244.0, -711.0, 2085.0, 1822.0, 3705.0, -6589.0,
    8492.0, 3776.0, 30112.0, -44821.0, 74313.0, -1.0, 17.0, -45.0, 183.0, 228.0, 197.0, -779.0,
    2075.0, 1739.0, 3351.0, -6935.0, 8840.0, 2935.0, 28289.0, -46617.0, 73908.0, -1.0, 16.0, -49.0,
    176.0, 228.0, 153.0, -848.0, 2057.0, 1644.0, 3004.0, -7271.0, 9139.0, 2037.0, 26482.0,
    -48390.0, 73415.0, -2.0, 14.0, -53.0, 169.0, 227.0, 111.0, -919.0, 2032.0, 1535.0, 2663.0,
    -7597.0, 9389.0, 1082.0, 24694.0, -50137.0, 72835.0, -2.0, 13.0, -58.0, 161.0, 224.0, 72.0,
    -991.0, 2001.0, 1414.0, 2330.0, -7910.0, 9592.0, 70.0, 22929.0, -51853.0, 72169.0, -2.0, 11.0,
    -63.0, 154.0, 221.0, 36.0, -1064.0, 1962.0, 1280.0, 2006.0, -8209.0, 9750.0, -998.0, 21189.0,
    -53534.0, 71420.0, -2.0, 10.0, -68.0, 147.0, 215.0, 2.0, -1137.0, 1919.0, 1131.0, 1692.0,
    -8491.0, 9863.0, -2122.0, 19478.0, -55178.0, 70590.0, -3.0, 9.0, -73.0, 139.0, 208.0, -29.0,
    -1210.0, 1870.0, 970.0, 1388.0, -8755.0, 9935.0, -3300.0, 17799.0, -56778.0, 69679.0, -3.0,
    8.0, -79.0, 132.0, 200.0, -57.0, -1283.0, 1817.0, 794.0, 1095.0, -8998.0, 9966.0, -4533.0,
    16155.0, -58333.0, 68692.0, -4.0, 7.0, -85.0, 125.0, 189.0, -83.0, -1356.0, 1759.0, 605.0,
    814.0, -9219.0, 9959.0, -5818.0, 14548.0, -59838.0, 67629.0, -4.0, 7.0, -91.0, 117.0, 177.0,
    -106.0, -1428.0, 1698.0, 402.0, 545.0, -9416.0, 9916.0, -7154.0, 12980.0, -61289.0, 66494.0,
    -5.0, 6.0, -97.0, 111.0, 163.0, -127.0, -1498.0, 1634.0, 185.0, 288.0, -9585.0, 9838.0,
    -8540.0, 11455.0, -62684.0, 65290.0,
];

/// Translation of `drmp3d_synth()`: `xl` in `grbuf`, `dstl` in `pcm`, and
/// `lins` in `syn`, as offsets. (The SSE version does the same arithmetic.)
fn d_synth(
    grbuf: &[f32],
    xl: usize,
    pcm: &mut [f32],
    dstl: usize,
    nch: usize,
    syn: &mut [f32],
    lins: usize,
) {
    let xr = xl + 576 * (nch - 1);
    let dstr = dstl + (nch - 1);

    let zlin = lins + 15 * 64;
    let mut w = 0usize;

    syn[zlin + 4 * 15] = grbuf[xl + 18 * 16];
    syn[zlin + 4 * 15 + 1] = grbuf[xr + 18 * 16];
    syn[zlin + 4 * 15 + 2] = grbuf[xl];
    syn[zlin + 4 * 15 + 3] = grbuf[xr];

    syn[zlin + 4 * 31] = grbuf[xl + 1 + 18 * 16];
    syn[zlin + 4 * 31 + 1] = grbuf[xr + 1 + 18 * 16];
    syn[zlin + 4 * 31 + 2] = grbuf[xl + 1];
    syn[zlin + 4 * 31 + 3] = grbuf[xr + 1];

    d_synth_pair(pcm, dstr, nch, syn, lins + 4 * 15 + 1);
    d_synth_pair(pcm, dstr + 32 * nch, nch, syn, lins + 4 * 15 + 64 + 1);
    d_synth_pair(pcm, dstl, nch, syn, lins + 4 * 15);
    d_synth_pair(pcm, dstl + 32 * nch, nch, syn, lins + 4 * 15 + 64);

    for i in (0..15usize).rev() {
        let mut a = [0.0f32; 4];
        let mut b = [0.0f32; 4];

        syn[zlin + 4 * i] = grbuf[xl + 18 * (31 - i)];
        syn[zlin + 4 * i + 1] = grbuf[xr + 18 * (31 - i)];
        syn[zlin + 4 * i + 2] = grbuf[xl + 1 + 18 * (31 - i)];
        syn[zlin + 4 * i + 3] = grbuf[xr + 1 + 18 * (31 - i)];
        syn[zlin + 4 * (i + 16)] = grbuf[xl + 1 + 18 * (1 + i)];
        syn[zlin + 4 * (i + 16) + 1] = grbuf[xr + 1 + 18 * (1 + i)];
        syn[zlin + 4 * i - 64 + 2] = grbuf[xl + 18 * (1 + i)];
        syn[zlin + 4 * i - 64 + 3] = grbuf[xr + 18 * (1 + i)];

        // DRMP3_S0(0) DRMP3_S2(1) DRMP3_S1(2) DRMP3_S2(3) DRMP3_S1(4) DRMP3_S2(5) DRMP3_S1(6) DRMP3_S2(7)
        for k in 0..8usize {
            let w0 = G_WIN[w];
            let w1 = G_WIN[w + 1];
            w += 2;
            let vz = zlin + 4 * i - k * 64;
            let vy = zlin + 4 * i - (15 - k) * 64;
            for j in 0..4 {
                let (z, y) = (syn[vz + j], syn[vy + j]);
                if k == 0 {
                    b[j] = z * w1 + y * w0;
                    a[j] = z * w0 - y * w1;
                } else if k & 1 == 0 {
                    b[j] += z * w1 + y * w0;
                    a[j] += z * w0 - y * w1;
                } else {
                    b[j] += z * w1 + y * w0;
                    a[j] += y * w1 - z * w0;
                }
            }
        }

        pcm[dstr + (15 - i) * nch] = d_scale_pcm(a[1]);
        pcm[dstr + (17 + i) * nch] = d_scale_pcm(b[1]);
        pcm[dstl + (15 - i) * nch] = d_scale_pcm(a[0]);
        pcm[dstl + (17 + i) * nch] = d_scale_pcm(b[0]);
        pcm[dstr + (47 - i) * nch] = d_scale_pcm(a[3]);
        pcm[dstr + (49 + i) * nch] = d_scale_pcm(b[3]);
        pcm[dstl + (47 - i) * nch] = d_scale_pcm(a[2]);
        pcm[dstl + (49 + i) * nch] = d_scale_pcm(b[2]);
    }
}

/// Translation of `drmp3d_synth_granule()`: the output is `pcm` from
/// `pcm_off`.
fn d_synth_granule(h: &mut Drmp3dec, nbands: usize, nch: usize, pcm: &mut [f32], pcm_off: usize) {
    let s = &mut h.scratch;
    for i in 0..nch {
        d_dct_ii(&mut s.grbuf[576 * i..], nbands as i32);
    }

    s.syn[..15 * 64].copy_from_slice(&h.qmf_state);

    let mut i = 0;
    while i < nbands {
        d_synth(
            &s.grbuf,
            i,
            pcm,
            pcm_off + 32 * nch * i,
            nch,
            &mut s.syn,
            i * 64,
        );
        i += 2;
    }
    if nch == 1 {
        let mut i = 0;
        while i < 15 * 64 {
            h.qmf_state[i] = s.syn[nbands * 64 + i];
            i += 2;
        }
    } else {
        h.qmf_state
            .copy_from_slice(&s.syn[nbands * 64..nbands * 64 + 15 * 64]);
    }
}

/// Translation of `drmp3d_match_frame()`.
fn d_match_frame(hdr: &[u8], mp3_bytes: i32, frame_bytes: i32) -> bool {
    let mut i: i32 = 0;
    for nmatch in 0..DRMP3_MAX_FRAME_SYNC_MATCHES {
        let h = &hdr[i as usize..];
        i += hdr_frame_bytes(h, frame_bytes) + hdr_padding(h);
        if i + DRMP3_HDR_SIZE > mp3_bytes {
            return nmatch > 0;
        }
        if !hdr_compare(hdr, &hdr[i as usize..]) {
            return false;
        }
    }
    true
}

/// Translation of `drmp3d_find_frame()`.
fn d_find_frame(
    mp3: &[u8],
    mp3_bytes: i32,
    free_format_bytes: &mut i32,
    ptr_frame_bytes: &mut i32,
) -> i32 {
    let mut i = 0i32;
    while i < mp3_bytes - DRMP3_HDR_SIZE {
        let m = &mp3[i as usize..];
        if hdr_valid(m) {
            let mut frame_bytes = hdr_frame_bytes(m, *free_format_bytes);
            let mut frame_and_padding = frame_bytes + hdr_padding(m);

            let mut k = DRMP3_HDR_SIZE;
            while frame_bytes == 0
                && k < DRMP3_MAX_FREE_FORMAT_FRAME_SIZE
                && i + 2 * k < mp3_bytes - DRMP3_HDR_SIZE
            {
                if hdr_compare(m, &m[k as usize..]) {
                    let fb = k - hdr_padding(m);
                    let nextfb = fb + hdr_padding(&m[k as usize..]);
                    if !(i + k + nextfb + DRMP3_HDR_SIZE > mp3_bytes
                        || !hdr_compare(m, &m[(k + nextfb) as usize..]))
                    {
                        frame_and_padding = k;
                        frame_bytes = fb;
                        *free_format_bytes = fb;
                    }
                }
                k += 1;
            }

            if (frame_bytes != 0
                && i + frame_and_padding <= mp3_bytes
                && d_match_frame(m, mp3_bytes - i, frame_bytes))
                || (i == 0 && frame_and_padding == mp3_bytes)
            {
                *ptr_frame_bytes = frame_and_padding;
                return i;
            }
            *free_format_bytes = 0;
        }
        i += 1;
    }
    *ptr_frame_bytes = 0;
    mp3_bytes
}

/// Translation of `drmp3dec_init()`.
pub(crate) fn drmp3dec_init(dec: &mut Drmp3dec) {
    dec.header[0] = 0;
}

/// Translation of `drmp3dec_decode_frame()`: decode a frame from
/// `mp3[..mp3_bytes]` into `pcm` (if given); the number of samples per
/// channel, or zero (and `info.frame_bytes` bytes to skip).
pub(crate) fn drmp3dec_decode_frame(
    dec: &mut Drmp3dec,
    mp3: &[u8],
    mp3_bytes: i32,
    mut pcm: Option<&mut [f32]>,
    info: &mut Drmp3decFrameInfo,
) -> i32 {
    let mut i: i32 = 0;
    let mut frame_size: i32 = 0;
    let mut success = 1;

    if mp3_bytes > 4 && dec.header[0] == 0xff && hdr_compare(&dec.header, mp3) {
        frame_size = hdr_frame_bytes(mp3, dec.free_format_bytes) + hdr_padding(mp3);
        if frame_size != mp3_bytes
            && (frame_size + DRMP3_HDR_SIZE > mp3_bytes
                || !hdr_compare(mp3, &mp3[frame_size as usize..]))
        {
            frame_size = 0;
        }
    }
    if frame_size == 0 {
        dec.zero();
        let mut ffb = dec.free_format_bytes;
        i = d_find_frame(mp3, mp3_bytes, &mut ffb, &mut frame_size);
        dec.free_format_bytes = ffb;
        if frame_size == 0 || i + frame_size > mp3_bytes {
            info.frame_bytes = i;
            return 0;
        }
    }

    let hdr = &mp3[i as usize..];
    dec.header.copy_from_slice(&hdr[..DRMP3_HDR_SIZE as usize]);
    info.frame_bytes = i + frame_size;
    info.channels = if hdr_is_mono(hdr) { 1 } else { 2 };
    info.sample_rate = hdr_sample_rate_hz(hdr) as i32;
    info.layer = 4 - hdr_get_layer(hdr);
    info.bitrate_kbps = hdr_bitrate_kbps(hdr) as i32;

    let frame = &hdr[DRMP3_HDR_SIZE as usize..frame_size as usize];
    let mut bs_frame = Drmp3Bs::new(frame, frame_size - DRMP3_HDR_SIZE);
    if hdr_is_crc(hdr) {
        bs_frame.get_bits(16);
    }

    let nch = info.channels as usize;
    if info.layer == 3 {
        let main_data_begin = l3_read_side_info(&mut bs_frame, &mut dec.scratch.gr_info, hdr);
        if main_data_begin < 0 || bs_frame.pos > bs_frame.limit {
            drmp3dec_init(dec);
            return 0;
        }
        success =
            l3_restore_reservoir(dec, frame, bs_frame.pos, bs_frame.limit, main_data_begin) as i32;
        if success != 0 {
            if let Some(pcm) = pcm.as_deref_mut() {
                let ngr = if hdr_test_mpeg1(hdr) { 2 } else { 1 };
                for igr in 0..ngr {
                    dec.scratch.grbuf.fill(0.0);
                    l3_decode(dec, igr * nch, nch as i32);
                    d_synth_granule(dec, 18, nch, pcm, 576 * nch * igr);
                }
            }
        }
        l3_save_reservoir(dec);
    } else {
        let Some(pcm) = pcm else {
            return hdr_frame_samples(hdr) as i32;
        };

        let mut sci = Drmp3L12ScaleInfo {
            scf: [0.0; 192],
            total_bands: 0,
            stereo_bands: 0,
            bitalloc: [0; 64],
            scfcod: [0; 64],
        };
        l12_read_scale_info(hdr, &mut bs_frame, &mut sci);

        dec.scratch.grbuf.fill(0.0);
        let mut pcm_off = 0usize;
        let mut i = 0i32;
        for igr in 0..3 {
            i += l12_dequantize_granule(
                &mut dec.scratch.grbuf,
                i as usize,
                &mut bs_frame,
                &sci,
                info.layer | 1,
            );
            if i == 12 {
                i = 0;
                l12_apply_scf_384(&sci, igr, &mut dec.scratch.grbuf);
                d_synth_granule(dec, 12, nch, pcm, pcm_off);
                dec.scratch.grbuf.fill(0.0);
                pcm_off += 384 * nch;
            }
            if bs_frame.pos > bs_frame.limit {
                drmp3dec_init(dec);
                return 0;
            }
        }
    }

    success * hdr_frame_samples(&dec.header) as i32
}

/************************************************************************************************************************************************************

Main Public API

************************************************************************************************************************************************************/
const DRMP3_SEEK_LEADING_MP3_FRAMES: usize = 2;
const DRMP3_MIN_DATA_CHUNK_SIZE: usize = 16384;
/* The size in bytes of each chunk of data to read from the MP3 stream. minimp3 recommends at least 16K, but in an attempt to reduce data movement I'm making this slightly larger. */
const DRMP3_DATA_CHUNK_SIZE: usize = DRMP3_MIN_DATA_CHUNK_SIZE * 4;
const DRMP3_UINT64_MAX: u64 = u64::MAX;

// drmp3 -> SDL_IOStream bridge (SDL_mixer's callbacks)...

/// SDL_mixer's `DRMP3_IoRead()`.
fn io_read(io: &mut IoStream<'_>, buf: &mut [u8]) -> usize {
    io.read(buf)
}

/// SDL_mixer's `DRMP3_IoSeek()`.
fn io_seek(io: &mut IoStream<'_>, offset: i32, origin: Drmp3SeekOrigin) -> bool {
    // SDL_IOWhence and drmp3_seek_origin happen to match up.
    let whence = match origin {
        Drmp3SeekOrigin::Set => IoWhence::Set,
        Drmp3SeekOrigin::Cur => IoWhence::Cur,
        Drmp3SeekOrigin::End => IoWhence::End,
    };
    io.seek(offset as i64, whence).is_ok()
}

/// SDL_mixer's `DRMP3_IoTell()`.
fn io_tell(io: &mut IoStream<'_>, pos: &mut i64) -> bool {
    *pos = io.tell().unwrap_or(-1);
    *pos >= 0
}

/// The Xing/Info/LAME header parsing of `drmp3_init_internal()`, over the
/// first frame's data (`frame`, which runs to the end of the data buffer).
/// `Some` if the frame is such a tag, with whether it's a Xing one and the
/// LAME delay and padding if there are some; `detected_mp3_frame_count` is
/// set as soon as the count is found (even if a later part is invalid, as
/// upstream does).
fn parse_xing_info(
    frame: &[u8],
    frame_bytes_in: i32,
    detected_mp3_frame_count: &mut u32,
) -> Option<(bool, Option<(u32, u32)>)> {
    let mut gr_info = [Drmp3L3GrInfo::default(); 4];
    let mut bs = Drmp3Bs::new(
        &frame[DRMP3_HDR_SIZE as usize..],
        frame_bytes_in - DRMP3_HDR_SIZE,
    );

    if hdr_is_crc(frame) {
        bs.get_bits(16); /* CRC. */
    }

    if l3_read_side_info(&mut bs, &mut gr_info, frame) < 0 {
        /* Failed to read the side info. */
        return None;
    }

    let tag_data_beg = DRMP3_HDR_SIZE as usize + (bs.pos / 8) as usize;
    let mut tag_data = tag_data_beg;

    /*
    We need to determine how many bytes are actually available in pTagData. Unfortunately this is different depending on
    whether or not it's being decoded from memory or callbacks.
    */
    let frame_bytes = (frame_bytes_in as usize).min(frame.len());

    if frame_bytes < tag_data + 8 {
        return None; /* Frame too small for a Xing/Info tag. */
    }

    /* Check for both "Xing" and "Info" identifiers. */
    let is_xing = &frame[tag_data..tag_data + 4] == b"Xing";
    let is_info = &frame[tag_data..tag_data + 4] == b"Info";

    if !(is_xing || is_info) {
        return None;
    }

    let flags = frame[tag_data + 7] as u32;
    let be32 = |p: usize| u32::from_be_bytes([frame[p], frame[p + 1], frame[p + 2], frame[p + 3]]);

    tag_data += 8; /* Skip past the ID and flags. */

    if flags & 0x01 != 0 {
        /* FRAMES flag. */
        if frame_bytes < tag_data + 4 {
            return None; /* Invalid Xing/Info tag. */
        }

        *detected_mp3_frame_count = be32(tag_data);
        tag_data += 4;
    }

    if flags & 0x02 != 0 {
        /* BYTES flag. */
        if frame_bytes < tag_data + 4 {
            return None; /* Invalid Xing/Info tag. */
        }

        let _bytes = be32(tag_data); /* <-- Just to silence a warning about `bytes` being assigned but unused. Want to leave this here in case I want to make use of it later. */
        tag_data += 4;
    }

    if flags & 0x04 != 0 {
        /* TOC flag. */
        if frame_bytes < tag_data + 100 {
            return None; /* Invalid Xing/Info tag. */
        }

        /* TODO: Extract and bind seek points. */
        tag_data += 100;
    }

    if flags & 0x08 != 0 {
        /* SCALE flag. */
        if frame_bytes < tag_data + 4 {
            return None; /* Invalid Xing/Info tag. */
        }

        tag_data += 4;
    }

    /* At this point we're done with the Xing/Info header. Now we can look at the LAME data. */
    if frame_bytes < tag_data + 1 {
        return None; /* Not enough data left to check for a LAME header. */
    }

    let mut delay_padding = None;
    if frame[tag_data] != 0 {
        if frame_bytes < tag_data + 36 {
            return None; /* Invalid Xing/Info tag. */
        }

        tag_data += 21;

        let p = &frame[tag_data..];
        let delay_in_pcm_frames = (((p[0] as u32) << 4) | ((p[1] as u32) >> 4)) as i32 + (528 + 1);
        let mut padding_in_pcm_frames =
            ((((p[1] as u32) & 0xF) << 8) | (p[2] as u32)) as i32 - (528 + 1);
        if padding_in_pcm_frames < 0 {
            padding_in_pcm_frames = 0; /* Padding cannot be negative. Probably a malformed file. Ignore. */
        }

        delay_padding = Some((delay_in_pcm_frames as u32, padding_in_pcm_frames as u32));
    }

    // (`tag_data_beg` would go to the metadata callback.)
    let _ = tag_data_beg;

    Some((is_xing, delay_padding))
}

impl<'a> Drmp3<'a> {
    /// Translation of `drmp3__on_read()`, into `data[start..start + bytes_to_read]`.
    fn on_read(&mut self, start: usize, bytes_to_read: usize) -> usize {
        /*
        Don't try reading 0 bytes from the callback. This can happen when the stream is clamped against
        ID3v1 or APE tags at the end of the stream.
        */
        if bytes_to_read == 0 {
            return 0;
        }

        let bytes_read = io_read(&mut self.io, &mut self.data[start..start + bytes_to_read]);
        self.stream_cursor = self.stream_cursor.wrapping_add(bytes_read as u64);

        bytes_read
    }

    /// Translation of `drmp3__on_read_clamped()`.
    fn on_read_clamped(&mut self, start: usize, mut bytes_to_read: usize) -> usize {
        if self.stream_length == DRMP3_UINT64_MAX {
            self.on_read(start, bytes_to_read)
        } else {
            let bytes_remaining = self.stream_length.wrapping_sub(self.stream_cursor);
            if bytes_to_read as u64 > bytes_remaining {
                bytes_to_read = bytes_remaining as usize;
            }

            self.on_read(start, bytes_to_read)
        }
    }

    /// Translation of `drmp3__on_seek()`.
    fn on_seek(&mut self, offset: i32, origin: Drmp3SeekOrigin) -> bool {
        debug_assert!(offset >= 0);
        debug_assert!(origin == Drmp3SeekOrigin::Set || origin == Drmp3SeekOrigin::Cur);

        if !io_seek(&mut self.io, offset, origin) {
            return false;
        }

        if origin == Drmp3SeekOrigin::Set {
            self.stream_cursor = offset as u64;
        } else {
            self.stream_cursor = self.stream_cursor.wrapping_add(offset as u64);
        }

        true
    }

    /// Translation of `drmp3__on_seek_64()`.
    fn on_seek_64(&mut self, mut offset: u64, origin: Drmp3SeekOrigin) -> bool {
        if offset <= 0x7FFFFFFF {
            return self.on_seek(offset as i32, origin);
        }

        /* Getting here "offset" is too large for a 32-bit integer. We just keep seeking forward until we hit the offset. */
        if !self.on_seek(0x7FFFFFFF, Drmp3SeekOrigin::Set) {
            return false;
        }

        offset -= 0x7FFFFFFF;
        while offset > 0 {
            if offset <= 0x7FFFFFFF {
                if !self.on_seek(offset as i32, Drmp3SeekOrigin::Cur) {
                    return false;
                }
                offset = 0;
            } else {
                if !self.on_seek(0x7FFFFFFF, Drmp3SeekOrigin::Cur) {
                    return false;
                }
                offset -= 0x7FFFFFFF;
            }
        }

        true
    }

    /// Grow the data buffer to `new_data_cap` bytes (the
    /// `drmp3__realloc_from_callbacks()` calls); `false` if out of memory.
    fn realloc_data(&mut self, new_data_cap: usize) -> bool {
        if self
            .data
            .try_reserve_exact(new_data_cap - self.data.len())
            .is_err()
        {
            return false;
        }
        self.data.resize(new_data_cap, 0);
        self.data_capacity = new_data_cap;
        true
    }

    /// Translation of `drmp3_decode_next_frame_ex__callbacks()` (and
    /// `drmp3_decode_next_frame_ex()`, since there is no memory variant
    /// here): decode into `pcm_frames` if `want_pcm_frames`. The frame's
    /// data is `data[*mp3_frame_data..]`.
    fn decode_next_frame_ex(
        &mut self,
        want_pcm_frames: bool,
        mut mp3_frame_info: Option<&mut Drmp3decFrameInfo>,
        mut mp3_frame_data: Option<&mut usize>,
    ) -> u32 {
        let mut pcm_frames_read: u32;

        if self.at_end {
            return 0;
        }

        loop {
            let mut info = Drmp3decFrameInfo::default();

            /* minimp3 recommends doing data submission in chunks of at least 16K. If we don't have at least 16K bytes available, get more. */
            if self.data_size < DRMP3_MIN_DATA_CHUNK_SIZE {
                /* First we need to move the data down. */
                if self.data_capacity != 0 {
                    self.data
                        .copy_within(self.data_consumed..self.data_consumed + self.data_size, 0);
                }

                self.data_consumed = 0;

                if self.data_capacity < DRMP3_DATA_CHUNK_SIZE
                    && !self.realloc_data(DRMP3_DATA_CHUNK_SIZE)
                {
                    return 0; /* Out of memory. */
                }

                let bytes_read =
                    self.on_read_clamped(self.data_size, self.data_capacity - self.data_size);
                if bytes_read == 0 && self.data_size == 0 {
                    self.at_end = true;
                    return 0; /* No data. */
                }

                self.data_size += bytes_read;
            }

            if self.data_size > i32::MAX as usize {
                self.at_end = true;
                return 0; /* File too big. */
            }

            /* Do a runtime check here to try silencing a false-positive from clang-analyzer. */
            if self.data_capacity == 0 {
                return 0;
            }

            let pcm = if want_pcm_frames {
                Some(&mut self.pcm_frames[..])
            } else {
                None
            };
            pcm_frames_read = drmp3dec_decode_frame(
                &mut self.decoder,
                &self.data[self.data_consumed..],
                self.data_size as i32, /* <-- Safe size_t -> int conversion thanks to the check above. */
                pcm,
                &mut info,
            ) as u32;

            /* Consume the data. */
            self.data_consumed += info.frame_bytes as usize;
            self.data_size -= info.frame_bytes as usize;

            /* pcmFramesRead will be equal to 0 if decoding failed. If it is zero and info.frame_bytes > 0 then we have successfully decoded the frame. */
            if pcm_frames_read > 0 {
                pcm_frames_read = hdr_frame_samples(&self.decoder.header);
                self.pcm_frames_consumed_in_mp3_frame = 0;
                self.pcm_frames_remaining_in_mp3_frame = pcm_frames_read;
                self.mp3_frame_channels = info.channels as u32;
                self.mp3_frame_sample_rate = info.sample_rate as u32;

                if let Some(frame_info) = mp3_frame_info.as_deref_mut() {
                    *frame_info = info;
                }

                if let Some(frame_data) = mp3_frame_data.as_deref_mut() {
                    *frame_data = self.data_consumed - info.frame_bytes as usize;
                }

                break;
            } else if info.frame_bytes == 0 {
                /* Need more data. minimp3 recommends doing data submission in 16K chunks. */

                /* First we need to move the data down. */
                self.data
                    .copy_within(self.data_consumed..self.data_consumed + self.data_size, 0);
                self.data_consumed = 0;

                if self.data_capacity == self.data_size {
                    /* No room. Expand. */
                    let new_data_cap = self.data_capacity + DRMP3_DATA_CHUNK_SIZE;
                    if !self.realloc_data(new_data_cap) {
                        return 0; /* Out of memory. */
                    }
                }

                /* Fill in a chunk. */
                let bytes_read =
                    self.on_read_clamped(self.data_size, self.data_capacity - self.data_size);
                if bytes_read == 0 {
                    self.at_end = true;
                    return 0; /* Error reading more data. */
                }

                self.data_size += bytes_read;
            }
        }

        pcm_frames_read
    }

    /// Translation of `drmp3_decode_next_frame()`.
    fn decode_next_frame(&mut self) -> u32 {
        self.decode_next_frame_ex(true, None, None)
    }

    /// Translation of `drmp3_init()` with SDL_mixer's callbacks over `io`
    /// (and no metadata callback): `None` if it's not an MP3 stream.
    pub fn init(io: IoStream<'a>) -> Option<Drmp3<'a>> {
        let mut mp3 = Drmp3 {
            decoder: Drmp3dec::zeroed(),
            channels: 0,
            sample_rate: 0,
            io,
            mp3_frame_channels: 0,
            mp3_frame_sample_rate: 0,
            pcm_frames_consumed_in_mp3_frame: 0,
            pcm_frames_remaining_in_mp3_frame: 0,
            pcm_frames: Box::new([0.0; DRMP3_MAX_SAMPLES_PER_FRAME]),
            current_pcm_frame: 0,
            stream_cursor: 0,
            stream_length: 0,
            stream_start_offset: 0,
            seek_points: Vec::new().into(),
            delay_in_pcm_frames: 0,
            padding_in_pcm_frames: 0,
            total_pcm_frame_count: 0,
            is_vbr: false,
            is_cbr: false,
            data_size: 0,
            data_capacity: 0,
            data_consumed: 0,
            data: Vec::new(),
            at_end: false,
        };
        if mp3.init_internal() {
            Some(mp3)
        } else {
            None
        }
    }

    /// Translation of `drmp3_init_internal()`. SDL_mixer always passes seek
    /// and tell callbacks and no metadata callback, so the paths for the
    /// other cases aren't translated.
    fn init_internal(&mut self) -> bool {
        let mut first_frame_info = Drmp3decFrameInfo::default();
        let mut first_frame_data = 0usize;
        let mut detected_mp3_frame_count: u32 = 0xFFFFFFFF;

        /* This function assumes the output object has already been reset to 0. Do not do that here, otherwise things will break. */
        drmp3dec_init(&mut self.decoder);

        self.stream_cursor = 0;
        self.stream_length = DRMP3_UINT64_MAX;
        self.stream_start_offset = 0;
        self.delay_in_pcm_frames = 0;
        self.padding_in_pcm_frames = 0;
        self.total_pcm_frame_count = DRMP3_UINT64_MAX;

        /* We'll first check for any ID3v1 or APE tags. */
        let io = &mut self.io;
        if io_seek(io, 0, Drmp3SeekOrigin::End) {
            let mut stream_len: i64 = 0;
            let mut stream_end_offset: i32 = 0;

            /* First get the length of the stream. We need this so we can ensure the stream is big enough to store the tags. */
            if io_tell(io, &mut stream_len) {
                /* ID3v1 */
                if stream_len > 128 {
                    let mut id3 = [0u8; 3];
                    if io_seek(io, stream_end_offset - 128, Drmp3SeekOrigin::End) {
                        if io_read(io, &mut id3) == 3 && &id3 == b"TAG" {
                            /* We have an ID3v1 tag. */
                            stream_end_offset -= 128;
                            stream_len -= 128;

                            /* (The metadata callback would get the TAG data here.) */
                        } else {
                            /* No ID3v1 tag. */
                        }
                    } else {
                        /* Failed to seek to the ID3v1 tag. */
                    }
                } else {
                    /* Stream too short. No ID3v1 tag. */
                }

                /* APE */
                if stream_len > 32 {
                    let mut ape = [0u8; 32]; /* The footer. */
                    if io_seek(io, stream_end_offset - 32, Drmp3SeekOrigin::End)
                        && io_read(io, &mut ape) == 32
                        && &ape[..8] == b"APETAGEX"
                    {
                        /* We have an APE tag. */
                        // FIXME (upstream): `ape` is a `char` array, so on
                        // targets where `char` is signed (x86) a size byte
                        // of 0x80 or more is sign extended over the higher
                        // ones. Done the same here.
                        let c = |b: u8| b as i8 as i32 as u32;
                        let tag_size: u32 = c(ape[24])
                            | (c(ape[25]) << 8)
                            | (c(ape[26]) << 16)
                            | (c(ape[27]) << 24);

                        if (32u32.wrapping_add(tag_size) as i64) < stream_len {
                            stream_end_offset = (stream_end_offset as u32)
                                .wrapping_sub(32u32.wrapping_add(tag_size))
                                as i32;
                            stream_len -= 32u32.wrapping_add(tag_size) as i64;

                            /* (The metadata callback would get the APE data here, from streamEndOffset.) */
                        } else {
                            /* The tag size is larger than the stream. Invalid APE tag. */
                        }
                    }
                } else {
                    /* Stream too short. No APE tag. */
                }
                let _ = stream_end_offset;

                /* Seek back to the start. */
                if !io_seek(io, 0, Drmp3SeekOrigin::Set) {
                    return false; /* Failed to seek back to the start. */
                }

                self.stream_length = stream_len as u64;
            } else {
                /* Failed to get the length of the stream. ID3v1 and APE tags cannot be skipped. */
                if !io_seek(io, 0, Drmp3SeekOrigin::Set) {
                    return false; /* Failed to seek back to the start. */
                }
            }
        } else {
            /* Failed to seek to the end. Cannot skip ID3v1 or APE tags. */
        }

        /* ID3v2 tags */
        {
            let io = &mut self.io;
            let mut header = [0u8; 10];
            if io_read(io, &mut header) == 10 {
                if &header[..3] == b"ID3" {
                    let mut tag_size: u32 = (((header[6] as u32) & 0x7F) << 21)
                        | (((header[7] as u32) & 0x7F) << 14)
                        | (((header[8] as u32) & 0x7F) << 7)
                        | ((header[9] as u32) & 0x7F);

                    /* Account for the footer. */
                    if header[5] & 0x10 != 0 {
                        tag_size += 10;
                    }

                    /* Don't have a metadata callback, so just skip the tag. */
                    if !io_seek(io, tag_size as i32, Drmp3SeekOrigin::Cur) {
                        return false; /* Failed to seek past the ID3v2 tag. */
                    }

                    self.stream_start_offset += (10 + tag_size) as u64; /* +10 for the header. */
                    self.stream_cursor = self.stream_start_offset;
                } else {
                    /* Not an ID3v2 tag. Seek back to the start. */
                    if !io_seek(io, 0, Drmp3SeekOrigin::Set) {
                        return false; /* Failed to seek back to the start. */
                    }
                }
            } else {
                /* Failed to read the header. We can return false here. If we couldn't read 10 bytes there's no way we'll have a valid MP3 stream. */
                return false;
            }
        }

        /*
        Decode the first frame to confirm that it is indeed a valid MP3 stream. Note that it's possible the first frame
        is actually a Xing/LAME/VBRI header. If this is the case we need to skip over it.
        */
        let first_frame_pcm_frame_count = self.decode_next_frame_ex(
            true,
            Some(&mut first_frame_info),
            Some(&mut first_frame_data),
        );
        if first_frame_pcm_frame_count > 0 {
            /*
            It might be a header. If so, we need to clear out the cached PCM frames in order to trigger a reload of fresh
            data when decoding starts. We can assume all validation has already been performed to check if this is a valid
            MP3 frame and that there is more than 0 bytes making up the frame.

            We're going to be basing this parsing code off the minimp3_ex implementation.
            */
            debug_assert!(first_frame_info.frame_bytes > 0);
            // (Note that `first_frame_data` is where the bytes the decoder
            // consumed start, which is before the frame itself if it
            // skipped some to find it; upstream looks there all the same.)
            if let Some((is_xing, delay_padding)) = parse_xing_info(
                &self.data[first_frame_data..],
                first_frame_info.frame_bytes,
                &mut detected_mp3_frame_count,
            ) {
                if let Some((delay, padding)) = delay_padding {
                    self.delay_in_pcm_frames = delay;
                    self.padding_in_pcm_frames = padding;
                }

                /*
                My understanding is that if the "Xing" header is present we can consider this to be a VBR stream and if the "Info" header is
                present it's a CBR stream. If this is not the case let me know! I'm just tracking this for the time being in case I want to
                look at doing some CBR optimizations later on, such as faster seeking.
                */
                if is_xing {
                    self.is_vbr = true;
                } else {
                    self.is_cbr = true;
                }

                /* Since this was identified as a tag, we don't want to treat it as audio. We need to clear out the PCM cache. */
                self.pcm_frames_remaining_in_mp3_frame = 0;

                /* The start offset needs to be moved to the end of this frame so it's not included in any audio processing after seeking. */
                self.stream_start_offset += first_frame_info.frame_bytes as u32 as u64;
                self.stream_cursor = self.stream_start_offset;

                /*
                The internal decoder needs to be reset to clear out any state. If we don't reset this state, it's possible for
                there to be inconsistencies in the number of samples read when reading to the end of the stream depending on
                whether or not the caller seeks to the start of the stream.
                */
                drmp3dec_init(&mut self.decoder);
            }
        } else {
            /* Not a valid MP3 stream. */
            return false;
        }

        if detected_mp3_frame_count != 0xFFFFFFFF {
            self.total_pcm_frame_count =
                detected_mp3_frame_count as u64 * first_frame_pcm_frame_count as u64;
        }

        self.channels = self.mp3_frame_channels;
        self.sample_rate = self.mp3_frame_sample_rate;

        true
    }

    /// Translation of `drmp3_read_pcm_frames_raw()`.
    fn read_pcm_frames_raw(
        &mut self,
        mut frames_to_read: u64,
        mut buffer_out: Option<&mut [f32]>,
    ) -> u64 {
        let mut total_frames_read: u64 = 0;

        while frames_to_read > 0 {
            /* Skip frames if necessary. */
            if self.current_pcm_frame < self.delay_in_pcm_frames as u64 {
                let frames_to_skip = (self.pcm_frames_remaining_in_mp3_frame as u64)
                    .min(self.delay_in_pcm_frames as u64 - self.current_pcm_frame)
                    as u32;

                self.current_pcm_frame += frames_to_skip as u64;
                self.pcm_frames_consumed_in_mp3_frame += frames_to_skip;
                self.pcm_frames_remaining_in_mp3_frame -= frames_to_skip;
            }

            let mut frames_to_consume =
                (self.pcm_frames_remaining_in_mp3_frame as u64).min(frames_to_read) as u32;

            /* Clamp the number of frames to read to the padding. */
            if self.total_pcm_frame_count != DRMP3_UINT64_MAX
                && self.total_pcm_frame_count > self.padding_in_pcm_frames as u64
            {
                if self.current_pcm_frame
                    < (self.total_pcm_frame_count - self.padding_in_pcm_frames as u64)
                {
                    let frames_remainig_to_padding = (self.total_pcm_frame_count
                        - self.padding_in_pcm_frames as u64)
                        - self.current_pcm_frame;
                    if frames_to_consume as u64 > frames_remainig_to_padding {
                        frames_to_consume = frames_remainig_to_padding as u32;
                    }
                } else {
                    /* We're into the padding. Abort. */
                    break;
                }
            }

            if let Some(out) = buffer_out.as_deref_mut() {
                /* f32 */
                let channels = self.channels as usize;
                let out_off = total_frames_read as usize * channels;
                let in_off = self.pcm_frames_consumed_in_mp3_frame as usize
                    * self.mp3_frame_channels as usize;
                let n = frames_to_consume as usize * channels;
                out[out_off..out_off + n].copy_from_slice(&self.pcm_frames[in_off..in_off + n]);
            }

            self.current_pcm_frame += frames_to_consume as u64;
            self.pcm_frames_consumed_in_mp3_frame += frames_to_consume;
            self.pcm_frames_remaining_in_mp3_frame -= frames_to_consume;
            total_frames_read += frames_to_consume as u64;
            frames_to_read -= frames_to_consume as u64;

            if frames_to_read == 0 {
                break;
            }

            /* If the cursor is already at the padding we need to abort. */
            if self.total_pcm_frame_count != DRMP3_UINT64_MAX
                && self.total_pcm_frame_count > self.padding_in_pcm_frames as u64
                && self.current_pcm_frame
                    >= (self.total_pcm_frame_count - self.padding_in_pcm_frames as u64)
            {
                break;
            }

            debug_assert!(self.pcm_frames_remaining_in_mp3_frame == 0);

            /* At this point we have exhausted our in-memory buffer so we need to re-fill. */
            if self.decode_next_frame() == 0 {
                break;
            }
        }

        total_frames_read
    }

    /// Translation of `drmp3_read_pcm_frames_f32()`: `channels` samples per
    /// frame into `buffer_out`, if given.
    pub fn read_pcm_frames_f32(
        &mut self,
        frames_to_read: u64,
        buffer_out: Option<&mut [f32]>,
    ) -> u64 {
        /* Fast path. No conversion required. */
        self.read_pcm_frames_raw(frames_to_read, buffer_out)
    }

    /// Translation of `drmp3_reset()`.
    fn reset(&mut self) {
        self.pcm_frames_consumed_in_mp3_frame = 0;
        self.pcm_frames_remaining_in_mp3_frame = 0;
        self.current_pcm_frame = 0;
        self.data_size = 0;
        self.at_end = false;
        drmp3dec_init(&mut self.decoder);
    }

    /// Translation of `drmp3_seek_to_start_of_stream()`.
    fn seek_to_start_of_stream(&mut self) -> bool {
        /* Seek to the start of the stream to begin with. */
        if !self.on_seek_64(self.stream_start_offset, Drmp3SeekOrigin::Set) {
            return false;
        }

        /* Clear any cached data. */
        self.reset();
        true
    }

    /// Translation of `drmp3_seek_forward_by_pcm_frames__brute_force()`.
    fn seek_forward_by_pcm_frames_brute_force(&mut self, frame_offset: u64) -> bool {
        /*
        Just using a dumb read-and-discard for now. What would be nice is to parse only the header of the MP3 frame, and then skip over leading
        frames without spending the time doing a full decode. I cannot see an easy way to do this in minimp3, however, so it may involve some
        kind of manual processing.
        */
        let frames_read = self.read_pcm_frames_f32(frame_offset, None);
        frames_read == frame_offset
    }

    /// Translation of `drmp3_seek_to_pcm_frame__brute_force()`.
    fn seek_to_pcm_frame_brute_force(&mut self, frame_index: u64) -> bool {
        if frame_index == self.current_pcm_frame {
            return true;
        }

        /*
        If we're moving foward we just read from where we're at. Otherwise we need to move back to the start of
        the stream and read from the beginning.
        */
        if frame_index < self.current_pcm_frame {
            /* Moving backward. Move to the start of the stream and then move forward. */
            if !self.seek_to_start_of_stream() {
                return false;
            }
        }

        debug_assert!(frame_index >= self.current_pcm_frame);
        self.seek_forward_by_pcm_frames_brute_force(frame_index - self.current_pcm_frame)
    }

    /// Translation of `drmp3_find_closest_seek_point()`.
    fn find_closest_seek_point(&self, frame_index: u64, seek_point_index: &mut u32) -> bool {
        *seek_point_index = 0;

        if frame_index < self.seek_points[0].pcm_frame_index {
            return false;
        }

        /* Linear search for simplicity to begin with while I'm getting this thing working. Once it's all working change this to a binary search. */
        for (i_seek_point, seek_point) in self.seek_points.iter().enumerate() {
            if seek_point.pcm_frame_index > frame_index {
                break; /* Found it. */
            }

            *seek_point_index = i_seek_point as u32;
        }

        true
    }

    /// Translation of `drmp3_seek_to_pcm_frame__seek_table()`.
    fn seek_to_pcm_frame_seek_table(&mut self, frame_index: u64) -> bool {
        debug_assert!(!self.seek_points.is_empty());

        /* If there is no prior seekpoint it means the target PCM frame comes before the first seek point. Just assume a seekpoint at the start of the file in this case. */
        let mut prior_seek_point_index = 0;
        let seek_point = if self.find_closest_seek_point(frame_index, &mut prior_seek_point_index) {
            self.seek_points[prior_seek_point_index as usize]
        } else {
            Drmp3SeekPoint::default()
        };

        /* First thing to do is seek to the first byte of the relevant MP3 frame. */
        if !self.on_seek_64(seek_point.seek_pos_in_bytes, Drmp3SeekOrigin::Set) {
            return false; /* Failed to seek. */
        }

        /* Clear any cached data. */
        self.reset();

        /* Whole MP3 frames need to be discarded first. */
        for i_mp3_frame in 0..seek_point.mp3_frames_to_discard {
            /* Pass in non-null for the last frame because we want to ensure the sample rate converter is preloaded correctly. */
            let want_pcm_frames = i_mp3_frame == seek_point.mp3_frames_to_discard - 1;

            /* We first need to decode the next frame. */
            let pcm_frames_read = self.decode_next_frame_ex(want_pcm_frames, None, None);
            if pcm_frames_read == 0 {
                return false;
            }
        }

        /* We seeked to an MP3 frame in the raw stream so we need to make sure the current PCM frame is set correctly. */
        self.current_pcm_frame = seek_point
            .pcm_frame_index
            .wrapping_sub(seek_point.pcm_frames_to_discard as u64);

        /*
        Now at this point we can follow the same process as the brute force technique where we just skip over unnecessary MP3 frames and then
        read-and-discard at least 2 whole MP3 frames.
        */
        let leftover_frames = frame_index.wrapping_sub(self.current_pcm_frame);
        self.seek_forward_by_pcm_frames_brute_force(leftover_frames)
    }

    /// Translation of `drmp3_seek_to_pcm_frame()`.
    pub fn seek_to_pcm_frame(&mut self, frame_index: u64) -> bool {
        if frame_index == 0 {
            return self.seek_to_start_of_stream();
        }

        /* Use the seek table if we have one. */
        if !self.seek_points.is_empty() {
            self.seek_to_pcm_frame_seek_table(frame_index)
        } else {
            self.seek_to_pcm_frame_brute_force(frame_index)
        }
    }

    /// Translation of `drmp3_get_mp3_and_pcm_frame_count()`: the MP3 and
    /// PCM frame counts.
    pub fn get_mp3_and_pcm_frame_count(&mut self) -> Option<(u64, u64)> {
        /*
        The way this works is we move back to the start of the stream, iterate over each MP3 frame and calculate the frame count based
        on our output sample rate, the seek back to the PCM frame we were sitting on before calling this function.
        */

        /* We'll need to seek back to where we were, so grab the PCM frame we're currently sitting on so we can restore later. */
        let current_pcm_frame = self.current_pcm_frame;

        if !self.seek_to_start_of_stream() {
            return None;
        }

        let mut total_pcm_frame_count: u64 = 0;
        let mut total_mp3_frame_count: u64 = 0;

        loop {
            let pcm_frames_in_current_mp3_frame = self.decode_next_frame_ex(false, None, None);
            if pcm_frames_in_current_mp3_frame == 0 {
                break;
            }

            total_pcm_frame_count += pcm_frames_in_current_mp3_frame as u64;
            total_mp3_frame_count += 1;
        }

        /* Finally, we need to seek back to where we were. */
        if !self.seek_to_start_of_stream() {
            return None;
        }

        if !self.seek_to_pcm_frame(current_pcm_frame) {
            return None;
        }

        Some((total_mp3_frame_count, total_pcm_frame_count))
    }

    /// Translation of `drmp3__accumulate_running_pcm_frame_count()`.
    fn accumulate_running_pcm_frame_count(
        &self,
        pcm_frame_count_in: u32,
        running_pcm_frame_count: &mut u64,
        running_pcm_frame_count_fractional_part: &mut f32,
    ) {
        let src_ratio = self.mp3_frame_sample_rate as f32 / self.sample_rate as f32;
        debug_assert!(src_ratio > 0.0);

        let pcm_frame_count_out_f =
            *running_pcm_frame_count_fractional_part + (pcm_frame_count_in as f32 / src_ratio);
        let pcm_frame_count_out = pcm_frame_count_out_f as u32;
        *running_pcm_frame_count_fractional_part =
            pcm_frame_count_out_f - pcm_frame_count_out as f32;
        *running_pcm_frame_count += pcm_frame_count_out as u64;
    }

    /// Translation of `drmp3_calculate_seek_points()`: `*seek_point_count`
    /// is the number of points wanted in, and of points written to
    /// `seek_points` out.
    pub fn calculate_seek_points(
        &mut self,
        seek_point_count_inout: &mut u32,
        seek_points: &mut [Drmp3SeekPoint],
    ) -> bool {
        // (`*pSeekPointCount` is the size of the caller's array.)
        let mut seek_point_count =
            (*seek_point_count_inout).min(seek_points.len().min(u32::MAX as usize) as u32);
        if seek_point_count == 0 {
            return false; /* The client has requested no seek points. Consider this to be invalid arguments since the client has probably not intended this. */
        }

        /* We'll need to seek back to the current sample after calculating the seekpoints so we need to go ahead and grab the current location at the top. */
        let current_pcm_frame = self.current_pcm_frame;

        /* We never do more than the total number of MP3 frames and we limit it to 32-bits. */
        let Some((total_mp3_frame_count, total_pcm_frame_count)) =
            self.get_mp3_and_pcm_frame_count()
        else {
            return false;
        };

        /* If there's less than DRMP3_SEEK_LEADING_MP3_FRAMES+1 frames we just report 1 seek point which will be the very start of the stream. */
        if total_mp3_frame_count < DRMP3_SEEK_LEADING_MP3_FRAMES as u64 + 1 {
            seek_point_count = 1;
            seek_points[0] = Drmp3SeekPoint::default();
        } else {
            #[derive(Clone, Copy, Default)]
            struct SeekingMp3FrameInfo {
                byte_pos: u64,
                pcm_frame_index: u64, /* <-- After sample rate conversion. */
            }
            let mut mp3_frame_info =
                [SeekingMp3FrameInfo::default(); DRMP3_SEEK_LEADING_MP3_FRAMES + 1];
            let mut running_pcm_frame_count: u64 = 0;
            let mut running_pcm_frame_count_fractional_part: f32 = 0.0;

            if seek_point_count as u64 > total_mp3_frame_count - 1 {
                seek_point_count = (total_mp3_frame_count as u32).wrapping_sub(1);
            }

            let pcm_frames_between_seek_points =
                total_pcm_frame_count / (seek_point_count as u64 + 1);

            /*
            Here is where we actually calculate the seek points. We need to start by moving the start of the stream. We then enumerate over each
            MP3 frame.
            */
            if !self.seek_to_start_of_stream() {
                return false;
            }

            /*
            We need to cache the byte positions of the previous MP3 frames. As a new MP3 frame is iterated, we cycle the byte positions in this
            array. The value in the first item in this array is the byte position that will be reported in the next seek point.
            */

            /* We need to initialize the array of MP3 byte positions for the leading MP3 frames. */
            for i_mp3_frame in 0..DRMP3_SEEK_LEADING_MP3_FRAMES + 1 {
                /* The byte position of the next frame will be the stream's cursor position, minus whatever is sitting in the buffer. */
                mp3_frame_info[i_mp3_frame].byte_pos =
                    self.stream_cursor.wrapping_sub(self.data_size as u64);
                mp3_frame_info[i_mp3_frame].pcm_frame_index = running_pcm_frame_count;

                /* We need to get information about this frame so we can know how many samples it contained. */
                let pcm_frames_in_current_mp3_frame_in =
                    self.decode_next_frame_ex(false, None, None);
                if pcm_frames_in_current_mp3_frame_in == 0 {
                    return false; /* This should never happen. */
                }

                self.accumulate_running_pcm_frame_count(
                    pcm_frames_in_current_mp3_frame_in,
                    &mut running_pcm_frame_count,
                    &mut running_pcm_frame_count_fractional_part,
                );
            }

            /*
            At this point we will have extracted the byte positions of the leading MP3 frames. We can now start iterating over each seek point and
            calculate them.
            */
            let mut next_target_pcm_frame: u64 = 0;
            for i_seek_point in 0..seek_point_count as usize {
                next_target_pcm_frame =
                    next_target_pcm_frame.wrapping_add(pcm_frames_between_seek_points);

                loop {
                    if next_target_pcm_frame < running_pcm_frame_count {
                        /* The next seek point is in the current MP3 frame. */
                        seek_points[i_seek_point] = Drmp3SeekPoint {
                            seek_pos_in_bytes: mp3_frame_info[0].byte_pos,
                            pcm_frame_index: next_target_pcm_frame,
                            mp3_frames_to_discard: DRMP3_SEEK_LEADING_MP3_FRAMES as u16,
                            pcm_frames_to_discard: next_target_pcm_frame.wrapping_sub(
                                mp3_frame_info[DRMP3_SEEK_LEADING_MP3_FRAMES - 1].pcm_frame_index,
                            ) as u16,
                        };
                        break;
                    } else {
                        /*
                        The next seek point is not in the current MP3 frame, so continue on to the next one. The first thing to do is cycle the cached
                        MP3 frame info.
                        */
                        for i in 0..mp3_frame_info.len() - 1 {
                            mp3_frame_info[i] = mp3_frame_info[i + 1];
                        }

                        /* Cache previous MP3 frame info. */
                        let last = mp3_frame_info.len() - 1;
                        mp3_frame_info[last].byte_pos =
                            self.stream_cursor.wrapping_sub(self.data_size as u64);
                        mp3_frame_info[last].pcm_frame_index = running_pcm_frame_count;

                        /*
                        Go to the next MP3 frame. This shouldn't ever fail, but just in case it does we just set the seek point and break. If it happens, it
                        should only ever do it for the last seek point.
                        */
                        let pcm_frames_in_current_mp3_frame_in =
                            self.decode_next_frame_ex(false, None, None);
                        if pcm_frames_in_current_mp3_frame_in == 0 {
                            seek_points[i_seek_point] = Drmp3SeekPoint {
                                seek_pos_in_bytes: mp3_frame_info[0].byte_pos,
                                pcm_frame_index: next_target_pcm_frame,
                                mp3_frames_to_discard: DRMP3_SEEK_LEADING_MP3_FRAMES as u16,
                                pcm_frames_to_discard: next_target_pcm_frame.wrapping_sub(
                                    mp3_frame_info[DRMP3_SEEK_LEADING_MP3_FRAMES - 1]
                                        .pcm_frame_index,
                                ) as u16,
                            };
                            break;
                        }

                        self.accumulate_running_pcm_frame_count(
                            pcm_frames_in_current_mp3_frame_in,
                            &mut running_pcm_frame_count,
                            &mut running_pcm_frame_count_fractional_part,
                        );
                    }
                }
            }

            /* Finally, we need to seek back to where we were. */
            if !self.seek_to_start_of_stream() {
                return false;
            }
            if !self.seek_to_pcm_frame(current_pcm_frame) {
                return false;
            }
        }

        *seek_point_count_inout = seek_point_count;
        true
    }

    /// Translation of `drmp3_bind_seek_table()`; an empty table unbinds.
    pub fn bind_seek_table(&mut self, seek_points: std::sync::Arc<[Drmp3SeekPoint]>) {
        self.seek_points = seek_points;
    }
}
