/*
    frame: Heap of routines dealing with the core mpg123 data structure.

    copyright 2008-2023 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org
    initially written by Thomas Orgis
*/
// Modified 2026-10-17: translated into Rust from frame.h, frame.c and the definitions of mpg123lib_intern.h in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The handle (`struct mpg123_handle_struct`) and the routines that
//! initialize, reset and free it, map sample offsets to frames and
//! prepare seeks and gapless decoding.

use crate::format::{mpg123_fmt_all, mpg123_invalidate_format};
use crate::icy::{clear_icy, init_icy, reset_icy, IcyMeta};
use crate::id3::{exit_id3, init_id3, reset_id3, Mpg123Id3v2};
use crate::index::{fi_exit, fi_init, fi_reset, fi_resize, FrameIndex};
use crate::mpg123::*;
use crate::optimize::{decclass, defdec, OptCla, OptDec, SynthS};
use crate::parse::set_pointer;
use crate::readers::{open_bad, Lseek, ReaderData, ReaderKind};

// (from mpg123lib_intern.h)
pub(crate) const MPG123_RATES: usize = 9;
pub(crate) const MPG123_ENCODINGS: usize = 12;

/* used to be: AUDIOBUFSIZE = n*64 with n=1,2,3 ...
now: factor on minimum frame buffer size (which takes upsampling into account) */
#[allow(dead_code)]
pub(crate) const AUDIOBUFSIZE: i32 = 2;

pub(crate) const SBLIMIT: usize = 32;
pub(crate) const SCALE_BLOCK: usize = 12;
pub(crate) const SSLIMIT: usize = 18;

/* Same as MPG_M_* */
#[allow(dead_code)]
pub(crate) const MPG_MD_STEREO: i32 = 0;
pub(crate) const MPG_MD_JOINT_STEREO: i32 = 1;
#[allow(dead_code)]
pub(crate) const MPG_MD_DUAL_CHANNEL: i32 = 2;
pub(crate) const MPG_MD_MONO: i32 = 3;

/* We support short or float output samples...
Short integer amplitude is scaled by this. */
pub(crate) const SHORT_SCALE: i32 = 32768;
/* That scales a short-scaled value to a 32bit integer scaled one
value = 2**31/2**15 */
pub(crate) const S32_RESCALE: i32 = 65536;

/* Pre Shift fo 16 to 8 bit converter table */
pub(crate) const AUSHIFT: i32 = 3;

/* max = 1728 */
pub(crate) const MAXFRAMESIZE: usize = 3456;

/// One of `bsspace[2]`: `MAXFRAMESIZE+512+4` bytes.
pub(crate) const BSSPACE_SIZE: usize = MAXFRAMESIZE + 512 + 4;

/* that's doubled in decode_ntom.c */
#[allow(dead_code)]
pub(crate) const NTOM_MUL: i32 = 32768;

/// Translation of `struct al_table`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AlTable {
    pub bits: i16,
    pub d: i16,
}

/// Translation of `struct outbuffer`, the output buffer, used to be
/// pcm_sample, pcm_point and audiobufsize. `data.len()` is `size`; `p` is
/// the read position in it.
#[derive(Debug, Default)]
pub(crate) struct OutBuffer {
    /* main data pointer, aligned */
    pub data: Vec<u8>,
    /* read pointer  */
    pub p: usize,
    /* fill from read pointer */
    pub fill: usize,
    pub size: usize,
}

/// Translation of `struct audioformat`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AudioFormat {
    /* Final encoding, after post-processing. */
    pub encoding: i32,
    /* Size of one sample in bytes, plain int should be fine here... */
    pub encsize: i32,
    /* Encoding of decoder synth. */
    pub dec_enc: i32,
    /* Size of one decoder sample. */
    pub dec_encsize: i32,
    pub channels: i32,
    pub rate: i64,
}

pub(crate) const NUM_CHANNELS: usize = 2;

/// Translation of `struct mpg123_pars_struct`. (`force_rate`, `feedpool`
/// and `feedbuffer` are left out: this is built without the NtoM
/// resampler and the feeder; see the crate docs.)
#[derive(Clone, Debug)]
pub struct Mpg123Pars {
    /* verbose level */
    pub(crate) verbose: i32,
    /* combination of above */
    pub(crate) flags: i64,
    pub(crate) down_sample: i32,
    /* (which) rva to do: 0: nothing, 1: radio/mix/track 2: album/audiophile */
    pub(crate) rva: i32,
    pub(crate) halfspeed: i64,
    pub(crate) doublespeed: i64,
    pub(crate) timeout: i64,
    pub(crate) audio_caps: [[[u8; MPG123_ENCODINGS]; MPG123_RATES + 1]; NUM_CHANNELS],
    pub(crate) icy_interval: i64,
    pub(crate) outscale: f64,
    pub(crate) resync_limit: i64,
    /* Long, because: negative values have a meaning. */
    pub(crate) index_size: i64,
    pub(crate) preframes: i64,
    pub(crate) freeformat_framesize: i64,
}

impl Default for Mpg123Pars {
    fn default() -> Self {
        Mpg123Pars {
            verbose: 0,
            flags: 0,
            down_sample: 0,
            rva: 0,
            halfspeed: 0,
            doublespeed: 0,
            timeout: 0,
            audio_caps: [[[0; MPG123_ENCODINGS]; MPG123_RATES + 1]; NUM_CHANNELS],
            icy_interval: 0,
            outscale: 0.0,
            resync_limit: 0,
            index_size: 0,
            preframes: 0,
            freeformat_framesize: 0,
        }
    }
}

/**<     0001 Positions are considered accurate. */
pub(crate) const FRAME_ACCURATE: i32 = 0x1;
/**<     0010 This stream is concatenated. */
pub(crate) const FRAME_FRANKENSTEIN: i32 = 0x2;
/**<     0100 Decoder is fleshly initialized. */
pub(crate) const FRAME_FRESH_DECODER: i32 = 0x4;
/**<     1000 Decoder can be used. */
pub(crate) const FRAME_DECODER_LIVE: i32 = 0x8;

/// Translation of `struct frame_header`: separate frame header structure
/// for safe decoding of headers without modifying the main frame struct
/// before we are sure that we can read a frame into it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrameHeader {
    pub lay: i32,
    // lots of flags that could share storage, should reform that
    /* 0: MPEG 1.0; 1: MPEG 2.0/2.5 -- both used as bool and array index! */
    pub lsf: i32,
    pub mpeg25: i32,
    pub error_protection: i32,
    pub bitrate_index: i32,
    pub sampling_frequency: i32,
    pub padding: i32,
    pub extension: i32,
    pub mode: i32,
    pub mode_ext: i32,
    pub copyright: i32,
    pub original: i32,
    pub emphasis: i32,
    // Even 16 bit int is enough for MAXFRAMESIZE
    /* computed framesize */
    pub framesize: i32,
    pub freeformat: i32,
    pub freeformat_framesize: i32,
    // Derived from header and checked against the above.
    pub ssize: i32,
}

pub(crate) const SINGLE_STEREO: i32 = -1;
pub(crate) const SINGLE_LEFT: i32 = 0;
pub(crate) const SINGLE_RIGHT: i32 = 1;
pub(crate) const SINGLE_MIX: i32 = 3;

/// The RVA data (`fr->rva`), used in common.c, set in id3.c.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Rva {
    pub level: [i32; 2],
    pub gain: [f32; 2],
    pub peak: [f32; 2],
}

/// The layer scratch buffers (`fr->layerscratch` and the `layer1`,
/// `layer2` and `layer3` pointers into it).
pub(crate) struct LayerScratch {
    /* ALIGNED(16) real fraction[2][SBLIMIT]; */
    pub layer1_fraction: [[f32; SBLIMIT]; 2],
    /* ALIGNED(16) real fraction[2][4][SBLIMIT] */
    pub layer2_fraction: [[[f32; SBLIMIT]; 4]; 2],
    /* ALIGNED(16) real hybridIn[2][SBLIMIT][SSLIMIT]; */
    pub hybrid_in: [[[f32; SSLIMIT]; SBLIMIT]; 2],
    /* ALIGNED(16) real hybridOut[2][SSLIMIT][SBLIMIT]; */
    pub hybrid_out: [[[f32; SBLIMIT]; SSLIMIT]; 2],
}

/// The synth's real buffers: `fr->real_buffs[2][2]`, each `0x110` reals.
pub(crate) type RealBuffs = [[[f32; 0x110]; 2]; 2];

/// The size of `fr->gainpow2` (`256+118+4`).
pub(crate) const GAINPOW2_SIZE: usize = 256 + 118 + 4;

/// Translation of `struct mpg123_handle_struct`: the decoder state.
///
/// Where upstream's struct members are pointers into its own buffers, this
/// keeps offsets (`bsbuf`, `bsbufold`, `wordpointer` into `bsspace`); the
/// dynamically allocated buffers are `Option`s that are `None` until
/// `INT123_frame_buffers()` makes them. `gainpow2` and `muls` share one
/// array, as they follow each other in upstream's struct: corrupt layer III
/// scale factors make upstream read past `gainpow2` into `muls`.
pub struct Mpg123Handle<'a> {
    /* to be moved into flags */
    pub(crate) fresh: i32,
    pub(crate) new_format: i32,
    pub(crate) hybrid_block: Box<[[[f32; SBLIMIT * SSLIMIT]; 2]; 2]>,
    pub(crate) hybrid_blc: [i32; 2],
    /* the scratch vars for the decoders, sometimes real, sometimes short... sometimes int/long */
    pub(crate) real_buffs: Option<Box<RealBuffs>>,
    /* Just have it always here. */
    pub(crate) bo: i32,
    /* _the_ decode table */
    pub(crate) decwin: Option<Box<[f32; 512 + 32]>>,
    pub(crate) have_eq_settings: i32,
    pub(crate) equalizer: [[f32; 32]; 2],
    /* for halfspeed mode */
    pub(crate) ssave: [u8; 34],
    pub(crate) halfphase: i32,
    /* a raw buffer and a pointer into the middle for signed short conversion, only allocated on demand */
    pub(crate) conv16to8_buf: Option<Box<[u8; 8192]>>,
    /* layer3 */
    pub(crate) long_limit: [[i32; 23]; 9],
    pub(crate) short_limit: [[i32; 14]; 9],
    /* not really dynamic, just different for mmx */
    // real gainpow2[256+118+4]; and then, layer2: real muls[27][64]; /* also used by layer 1 */
    pub(crate) gainpow2_muls: Box<[f32; GAINPOW2_SIZE + 27 * 64]>,
    pub(crate) synths: SynthS,
    pub(crate) cpu_opts_type: OptDec,
    pub(crate) cpu_opts_class: OptCla,
    /* 0: nothing, 1: just print chosen decoder, 2: be verbose */
    #[allow(dead_code)]
    pub(crate) verbose: i32,
    pub(crate) alloc: &'static [AlTable],
    /* The runtime-chosen decoding, based on input and output format. */
    pub(crate) synth: Option<crate::optimize::FuncSynth>,
    pub(crate) synth_stereo: Option<crate::optimize::FuncSynthStereo>,
    pub(crate) synth_mono: Option<crate::optimize::FuncSynthMono>,
    /* Yes, this function is runtime-switched, too. */
    /* That is the volume control. */
    pub(crate) make_decode_tables: Option<fn(&mut Mpg123Handle<'_>)>,
    /* I _think_ 1 for mono and 2 for stereo */
    pub(crate) stereo: i32,
    pub(crate) jsbound: i32,
    pub(crate) single: i32,
    pub(crate) ii_sblimit: i32,
    pub(crate) down_sample_sblimit: i32,
    /* Many flags in disguise as integers... wasting bytes. */
    pub(crate) down_sample: i32,
    pub(crate) header_change: i32,
    pub(crate) hdr: FrameHeader,
    /* cached count of samples per frame */
    pub(crate) spf: i64,
    pub(crate) do_layer: Option<fn(&mut Mpg123Handle<'_>) -> i32>,
    /* 1 if variable bitrate was detected */
    pub(crate) vbr: i32,
    /* frame offset ... */
    pub(crate) num: i64,
    /* byte offset of this frame in input stream */
    pub(crate) input_offset: i64,
    /* playback offset... includes repetitions, reset at seeks */
    pub(crate) playnum: i64,
    /* The byte offset in the file where audio data begins. */
    pub(crate) audio_start: i64,
    pub(crate) state_flags: i32,
    /* Do not complain for the next n resyncs. */
    pub(crate) silent_resync: i8,
    /* The seek TOC from Xing header. */
    pub(crate) xing_toc: Option<Box<[u8; 100]>>,

    /* bitstream info; bsi */
    pub(crate) bitindex: i32,
    pub(crate) bits_avail: i64,
    /// `wordpointer`, as an offset into `bsspace`.
    pub(crate) wordpointer: usize,
    /* temporary storage for getbits stuff */
    pub(crate) ultmp: u64,
    pub(crate) uctmp: u8,

    /* rva data, used in common.c, set in id3.c */
    #[allow(dead_code)]
    pub(crate) maxoutburst: f64,
    pub(crate) lastscale: f64,
    pub(crate) rva: Rva,

    /* input data */
    pub(crate) track_frames: i64,
    pub(crate) track_samples: i64,
    pub(crate) mean_framesize: f64,
    pub(crate) mean_frames: i64,
    pub(crate) fsizeold: i32,
    pub(crate) bitreservoir: u32,
    /* MAXFRAMESIZE */
    /// `bsspace[2][MAXFRAMESIZE+512+4]`, the two buffers one after the
    /// other.
    pub(crate) bsspace: Box<[u8]>,
    /// `bsbuf`, as an offset into `bsspace`.
    pub(crate) bsbuf: usize,
    /// `bsbufold`, as an offset into `bsspace`.
    pub(crate) bsbufold: usize,
    pub(crate) bsnum: i32,
    /* That is the header matching the last read frame body. */
    pub(crate) oldhead: u64,
    /* That is the header that is supposedly the first of the stream. */
    pub(crate) firsthead: u64,
    pub(crate) abr_rate: i32,
    pub(crate) index: FrameIndex,

    /* output data */
    pub(crate) buffer: OutBuffer,
    pub(crate) af: AudioFormat,
    pub(crate) own_buffer: i32,
    /* number of bytes that this frame produces (upper bound) */
    pub(crate) outblock: usize,
    /* this frame holds data to be decoded */
    pub(crate) to_decode: i32,
    /* the same, somehow */
    pub(crate) to_ignore: i32,
    /* start decoding from here */
    pub(crate) firstframe: i64,
    /* last frame to decode (for gapless or num_frames limit) */
    pub(crate) lastframe: i64,
    /* frames to decode but discard before firstframe */
    pub(crate) ignoreframe: i64,
    /* frame count for the gapless part */
    pub(crate) gapless_frames: i64,
    /* number of samples to ignore from firstframe */
    pub(crate) firstoff: i64,
    /* number of samples to use from lastframe */
    pub(crate) lastoff: i64,
    /* overall begin offset in samples */
    pub(crate) begin_s: i64,
    pub(crate) begin_os: i64,
    /* overall end offset in samples */
    pub(crate) end_s: i64,
    pub(crate) end_os: i64,
    /* gapless_frames translated to output samples */
    pub(crate) fullend_os: i64,
    /* Well, I need a safe 16bit type, actually. But wider doesn't hurt. */
    pub(crate) crc: u32,
    /* pointer to the reading functions */
    pub(crate) rd: ReaderKind,
    /* reader data and state info */
    pub(crate) rdat: ReaderData<'a>,
    pub(crate) p: Mpg123Pars,
    pub(crate) err: i32,
    pub(crate) decoder_change: i32,
    #[allow(dead_code)]
    pub(crate) delayed_change: i32,
    pub(crate) clip: i64,
    /* the meta crap */
    pub(crate) metaflags: i32,
    pub(crate) id3buf: [u8; 128],
    pub(crate) id3v2: Mpg123Id3v2,
    pub(crate) id3v2_raw: Option<Vec<u8>>,
    pub(crate) id3v2_size: usize,
    pub(crate) icy: IcyMeta,
    /*
    More variables needed for decoders, layerX.c.
    This time it is not about static variables but about the need for alignment which cannot be guaranteed on the stack by certain compilers (Sun Studio).
    We do not require the compiler to align stuff for our hand-written assembly. We only hope that it's able to align stuff for SSE and similar ops it generates itself.
    */
    /*
    Those layer-specific structs could actually share memory, as they are not in use simultaneously. One might allocate on decoder switch, too.
    They all reside in one lump of memory (after each other), allocated to layerscratch.
    */
    pub(crate) layerscratch: Option<Box<LayerScratch>>,
    pub(crate) enc_delay: i32,
    pub(crate) enc_padding: i32,
}

impl std::fmt::Debug for Mpg123Handle<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mpg123Handle")
            .field("num", &self.num)
            .field("af", &self.af)
            .finish_non_exhaustive()
    }
}

/// Allocate a zeroed box without putting the value on the stack first.
pub(crate) fn zeroed_box<T: Copy + Default, const N: usize>() -> Box<[T; N]> {
    vec![T::default(); N]
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!())
}

/// Translation of `frame_default_pars()`.
pub(crate) fn frame_default_pars(mp: &mut Mpg123Pars) {
    mp.outscale = 1.0;
    mp.flags = 0;
    mp.flags |= MPG123_GAPLESS;
    mp.flags |= MPG123_AUTO_RESAMPLE | MPG123_FLOAT_FALLBACK;
    mp.down_sample = 0;
    mp.rva = 0;
    mp.halfspeed = 0;
    mp.doublespeed = 0;
    mp.verbose = 0;
    mp.icy_interval = 0;
    mp.timeout = 0;
    mp.resync_limit = 1024;
    mp.index_size = INDEX_SIZE;
    mp.preframes = 4; /* That's good  for layer 3 ISO compliance bitstream. */
    mpg123_fmt_all(mp);
    mp.freeformat_framesize = -1;
}

/// `INDEX_SIZE`: the seek table size SDL_mixer's CMake build configures
/// (`WITH_SEEKTABLE` 1000).
pub(crate) const INDEX_SIZE: i64 = 1000;

impl<'a> Mpg123Handle<'a> {
    /// The handle as upstream's `malloc()` leaves it, before
    /// `INT123_frame_init_par()` (the members that the latter doesn't set
    /// are zero here).
    pub(crate) fn raw() -> Box<Mpg123Handle<'a>> {
        Box::new(Mpg123Handle {
            fresh: 0,
            new_format: 0,
            hybrid_block: Box::new([[[0.0; SBLIMIT * SSLIMIT]; 2]; 2]),
            hybrid_blc: [0; 2],
            real_buffs: None,
            bo: 0,
            decwin: None,
            have_eq_settings: 0,
            equalizer: [[0.0; 32]; 2],
            ssave: [0; 34],
            halfphase: 0,
            conv16to8_buf: None,
            long_limit: [[0; 23]; 9],
            short_limit: [[0; 14]; 9],
            gainpow2_muls: zeroed_box(),
            synths: SynthS::default(),
            cpu_opts_type: OptDec::Nodec,
            cpu_opts_class: OptCla::Nocla,
            verbose: 0,
            alloc: &[],
            synth: None,
            synth_stereo: None,
            synth_mono: None,
            make_decode_tables: None,
            stereo: 0,
            jsbound: 0,
            single: 0,
            ii_sblimit: 0,
            down_sample_sblimit: 0,
            down_sample: 0,
            header_change: 0,
            hdr: FrameHeader::default(),
            spf: 0,
            do_layer: None,
            vbr: 0,
            num: 0,
            input_offset: 0,
            playnum: 0,
            audio_start: 0,
            state_flags: 0,
            silent_resync: 0,
            xing_toc: None,
            bitindex: 0,
            bits_avail: 0,
            wordpointer: 0,
            ultmp: 0,
            uctmp: 0,
            maxoutburst: 0.0,
            lastscale: 0.0,
            rva: Rva::default(),
            track_frames: 0,
            track_samples: 0,
            mean_framesize: 0.0,
            mean_frames: 0,
            fsizeold: 0,
            bitreservoir: 0,
            bsspace: vec![0; 2 * BSSPACE_SIZE].into_boxed_slice(),
            bsbuf: 0,
            bsbufold: 0,
            bsnum: 0,
            oldhead: 0,
            firsthead: 0,
            abr_rate: 0,
            index: FrameIndex::default(),
            buffer: OutBuffer::default(),
            af: AudioFormat::default(),
            own_buffer: 0,
            outblock: 0,
            to_decode: 0,
            to_ignore: 0,
            firstframe: 0,
            lastframe: 0,
            ignoreframe: 0,
            gapless_frames: 0,
            firstoff: 0,
            lastoff: 0,
            begin_s: 0,
            begin_os: 0,
            end_s: 0,
            end_os: 0,
            fullend_os: 0,
            crc: 0,
            rd: ReaderKind::Bad,
            rdat: ReaderData::default(),
            p: Mpg123Pars::default(),
            err: 0,
            decoder_change: 0,
            delayed_change: 0,
            clip: 0,
            metaflags: 0,
            id3buf: [0; 128],
            id3v2: Mpg123Id3v2::default(),
            id3v2_raw: None,
            id3v2_size: 0,
            icy: IcyMeta::default(),
            layerscratch: None,
            enc_delay: 0,
            enc_padding: 0,
        })
    }

    /// `fr->gainpow2[i]`
    #[inline]
    pub(crate) fn gainpow2(&self, i: usize) -> f32 {
        self.gainpow2_muls[i]
    }

    /// `fr->muls[k][i]`
    #[inline]
    pub(crate) fn muls(&self, k: usize, i: usize) -> f32 {
        self.gainpow2_muls[GAINPOW2_SIZE + k * 64 + i]
    }

    /// `NOQUIET`
    #[inline]
    pub(crate) fn noquiet(&self) -> bool {
        self.p.flags & MPG123_QUIET == 0
    }
}

/// Translation of `INT123_frame_init_par()`.
pub(crate) fn frame_init_par(fr: &mut Mpg123Handle<'_>, mp: Option<&Mpg123Pars>) {
    fr.own_buffer = 1;
    fr.buffer.data = Vec::new();
    fr.buffer.fill = 0;
    fr.buffer.size = 0;
    fr.real_buffs = None;
    fr.decwin = None;
    fr.conv16to8_buf = None;
    fr.layerscratch = None;
    fr.xing_toc = None;
    fr.cpu_opts_type = defdec();
    fr.cpu_opts_class = decclass(fr.cpu_opts_type);
    /* unnecessary: fr->buffer.size = fr->buffer.fill = 0; */
    mpg123_reset_eq(fr);
    init_icy(&mut fr.icy);
    init_id3(fr);
    /* INT123_frame_outbuffer is missing... */
    /* INT123_frame_buffers is missing... that one needs cpu opt setting! */
    /* after these... INT123_frame_reset is needed before starting full decode */
    mpg123_invalidate_format(&mut fr.af);
    fr.rdat.iohandle = None;
    fr.rdat.r_read64 = false;
    fr.rdat.lseek_kind = Lseek::Unset;
    fr.decoder_change = 1;
    fr.err = MPG123_OK;
    match mp {
        None => frame_default_pars(&mut fr.p),
        Some(mp) => fr.p = mp.clone(),
    }

    fr.down_sample = 0; /* Initialize to silence harmless errors when debugging. */
    fr.id3v2_raw = None;
    frame_fixed_reset(fr); /* Reset only the fixed data, dynamic buffers are not there yet! */
    fr.synth = None;
    fr.synth_mono = None;
    fr.make_decode_tables = None;
    fi_init(&mut fr.index);
    frame_index_setup(fr); /* Apply the size setting. */
}

/// Translation of `mpg123_reset_eq()`.
pub(crate) fn mpg123_reset_eq(mh: &mut Mpg123Handle<'_>) -> i32 {
    mh.have_eq_settings = 0;
    for i in 0..32 {
        mh.equalizer[0][i] = 1.0;
        mh.equalizer[1][i] = 1.0;
    }
    MPG123_OK
}

/// Translation of `INT123_frame_outbuffer()`.
pub(crate) fn frame_outbuffer(fr: &mut Mpg123Handle<'_>) -> i32 {
    let size = fr.outblock;
    if fr.own_buffer == 0 && fr.buffer.size < size {
        fr.err = MPG123_BAD_BUFFER;
        return MPG123_ERR;
    }

    if !fr.buffer.data.is_empty() && fr.buffer.size != size {
        fr.buffer.data = Vec::new();
    }
    fr.buffer.size = size;
    /* be generous: use 16 byte alignment */
    if fr.buffer.data.is_empty() {
        let mut v = Vec::new();
        if v.try_reserve_exact(size + 15).is_err() {
            fr.err = MPG123_OUT_OF_MEM;
            return MPG123_ERR;
        }
        // (the buffer is the size, with no use for the alignment slack)
        v.resize(size, 0);
        fr.buffer.data = v;
    }
    fr.own_buffer = 1;
    fr.buffer.fill = 0;
    MPG123_OK
}

/// Translation of `INT123_frame_index_setup()`.
pub(crate) fn frame_index_setup(fr: &mut Mpg123Handle<'_>) -> i32 {
    let ret;
    if fr.p.index_size >= 0 {
        /* Simple fixed index. */
        fr.index.grow_size = 0;
        ret = fi_resize(&mut fr.index, fr.p.index_size as usize);
    } else {
        /* A growing index. We give it a start, though. */
        fr.index.grow_size = fr.p.index_size.unsigned_abs() as usize;
        if fr.index.size() < fr.index.grow_size {
            let grow_size = fr.index.grow_size;
            ret = fi_resize(&mut fr.index, grow_size);
        } else {
            ret = MPG123_OK; /* We have minimal size already... and since growing is OK... */
        }
    }
    ret
}

fn frame_decode_buffers_reset(fr: &mut Mpg123Handle<'_>) {
    if let Some(b) = fr.real_buffs.as_mut() {
        /* memset(NULL, 0, 0) not desired */
        **b = [[[0.0; 0x110]; 2]; 2];
    }
}

/// Translation of `INT123_frame_buffers()`.
pub(crate) fn frame_buffers(fr: &mut Mpg123Handle<'_>) -> i32 {
    /*
    the used-to-be-static buffer of the synth functions, has some subtly different types/sizes

    2to1, 4to1, ntom, generic, i386: real[2][2][0x110]
    mmx, sse: short[2][2][0x110]
    i586(_dither): 4352 bytes; int/long[2][2][0x110]
    i486: int[2][2][17*FIR_BUFFER_SIZE]
    altivec: static real __attribute__ ((aligned (16))) buffs[4][4][0x110]

    Huh, altivec looks like fun. Well, let it be large... then, the 16 byte alignment seems to be implicit on MacOSX malloc anyway.
    Let's make a reasonable attempt to allocate enough memory...
    Keep in mind: biggest ones are i486 and altivec (mutually exclusive!), then follows i586 and normal real.
    mmx/sse use short but also real for resampling.
    Thus, minimum is 2*2*0x110*sizeof(real).
    */
    // (only the generic decoder is here: real[2][2][0x110])
    if fr.real_buffs.is_none() {
        fr.real_buffs = Some(Box::new([[[0.0; 0x110]; 2]; 2]));
    }
    /* now the different decwins... all of the same size, actually */
    /* The MMX ones want 32byte alignment, which I'll try to ensure manually */
    if fr.decwin.is_none() {
        fr.decwin = Some(zeroed_box());
    }

    /* Layer scratch buffers are of compile-time fixed size, so allocate only once. */
    if fr.layerscratch.is_none() {
        /* Allocate specific layer1/2/3 buffers, so that we know they'll work for SSE. */
        fr.layerscratch = Some(Box::new(LayerScratch {
            layer1_fraction: [[0.0; SBLIMIT]; 2],
            layer2_fraction: [[[0.0; SBLIMIT]; 4]; 2],
            hybrid_in: [[[0.0; SSLIMIT]; SBLIMIT]; 2],
            hybrid_out: [[[0.0; SBLIMIT]; SSLIMIT]; 2],
        }));
        /* Note: These buffers don't need resetting here. */
    }

    /* Only reset the buffers we created just now. */
    frame_decode_buffers_reset(fr);

    0
}

/// Translation of `INT123_frame_buffers_reset()`.
pub(crate) fn frame_buffers_reset(fr: &mut Mpg123Handle<'_>) -> i32 {
    fr.buffer.fill = 0; /* hm, reset buffer fill... did we do a flush? */
    fr.bsnum = 0;
    /* Wondering: could it be actually _wanted_ to retain buffer contents over different files? (special gapless / cut stuff) */
    fr.bsbuf = BSSPACE_SIZE; // fr->bsspace[1]
    fr.bsbufold = fr.bsbuf;
    fr.bitreservoir = 0;
    frame_decode_buffers_reset(fr);
    // (memset(fr->bsspace, 0, 2*(MAXFRAMESIZE+512)): all but the last 8 bytes)
    fr.bsspace[..2 * (MAXFRAMESIZE + 512)].fill(0);
    fr.ssave = [0; 34];
    fr.hybrid_blc = [0; 2];
    *fr.hybrid_block = [[[0.0; SBLIMIT * SSLIMIT]; 2]; 2];
    0
}

fn frame_icy_reset(fr: &mut Mpg123Handle<'_>) {
    fr.icy.data = None;
    fr.icy.interval = 0;
    fr.icy.next = 0;
}

fn frame_free_toc(fr: &mut Mpg123Handle<'_>) {
    fr.xing_toc = None;
}

/// Translation of `INT123_frame_fill_toc()`: just copy the Xing TOC over...
pub(crate) fn frame_fill_toc(fr: &mut Mpg123Handle<'_>, inp: &[u8]) -> bool {
    let toc = fr.xing_toc.get_or_insert_with(|| Box::new([0; 100]));
    toc.copy_from_slice(&inp[..100]);
    true
}

/// Translation of `INT123_frame_reset()`: prepare the handle for a new
/// track. Reset variables, buffers...
pub(crate) fn frame_reset(fr: &mut Mpg123Handle<'_>) -> i32 {
    frame_buffers_reset(fr);
    frame_fixed_reset(fr);
    frame_free_toc(fr);
    fi_reset(&mut fr.index);

    0
}

/* Reset everythign except dynamic memory. */
fn frame_fixed_reset(fr: &mut Mpg123Handle<'_>) {
    frame_icy_reset(fr);
    open_bad(fr);
    fr.hdr = FrameHeader::default();
    fr.to_decode = 0;
    fr.to_ignore = 0;
    fr.metaflags = 0;
    fr.outblock = 0; /* This will be set before decoding! */
    fr.num = -1;
    fr.input_offset = -1;
    fr.playnum = -1;
    fr.state_flags = FRAME_ACCURATE;
    fr.silent_resync = 0;
    fr.audio_start = 0;
    fr.clip = 0;
    fr.oldhead = 0;
    fr.firsthead = 0;
    fr.vbr = MPG123_CBR;
    fr.abr_rate = 0;
    fr.track_frames = 0;
    fr.track_samples = -1;
    fr.mean_frames = 0;
    fr.mean_framesize = 0.0;
    fr.lastscale = -1.0;
    fr.rva.level[0] = -1;
    fr.rva.level[1] = -1;
    fr.rva.gain[0] = 0.0;
    fr.rva.gain[1] = 0.0;
    fr.rva.peak[0] = 0.0;
    fr.rva.peak[1] = 0.0;
    fr.fsizeold = 0;
    fr.firstframe = 0;
    fr.ignoreframe = fr.firstframe - fr.p.preframes;
    fr.header_change = 0;
    fr.lastframe = -1;
    fr.fresh = 1;
    fr.new_format = 0;
    frame_gapless_init(fr, -1, 0, 0);
    fr.lastoff = 0;
    fr.firstoff = 0;
    fr.bo = 1; /* the usual bo */
    reset_id3(fr);
    reset_icy(&mut fr.icy);
    /* ICY stuff should go into icy.c, eh? */
    fr.icy.interval = 0;
    fr.icy.next = 0;
    fr.halfphase = 0; /* here or indeed only on first-time init? */
    fr.hdr.freeformat_framesize = fr.p.freeformat_framesize as i32;
    fr.enc_delay = -1;
    fr.enc_padding = -1;
    fr.id3buf = [0; 128];
    fr.id3v2_raw = None;
    fr.id3v2_size = 0;
}

fn frame_free_buffers(fr: &mut Mpg123Handle<'_>) {
    fr.real_buffs = None;
    fr.decwin = None;
    fr.conv16to8_buf = None;
    fr.layerscratch = None;
}

/// Translation of `INT123_frame_exit()`.
pub(crate) fn frame_exit(fr: &mut Mpg123Handle<'_>) {
    fr.buffer.data = Vec::new();
    frame_free_buffers(fr);
    frame_free_toc(fr);
    fi_exit(&mut fr.index);
    exit_id3(fr);
    clear_icy(&mut fr.icy);
}

/*
Fuzzy frame offset searching (guessing).
When we don't have an accurate position, we may use an inaccurate one.
Possibilities:
    - use approximate positions from Xing TOC (not yet parsed)
    - guess wildly from mean framesize and offset of first frame / beginning of file.
*/

fn frame_fuzzy_find(fr: &mut Mpg123Handle<'_>, want_frame: i64, get_frame: &mut i64) -> i64 {
    /* Default is to go to the beginning. */
    let mut ret = fr.audio_start;
    *get_frame = 0;

    /* But we try to find something better. */
    /* Xing VBR TOC works with relative positions, both in terms of audio frames and stream bytes.
    Thus, it only works when whe know the length of things.
    Oh... I assume the offsets are relative to the _total_ file length. */
    if let (Some(toc), true) = (
        fr.xing_toc.as_deref(),
        fr.track_frames > 0 && fr.rdat.filelen > 0,
    ) {
        /* One could round... */
        let mut toc_entry = (want_frame as f64 * 100.0 / fr.track_frames as f64) as i32;
        /* It is an index in the 100-entry table. */
        if toc_entry < 0 {
            toc_entry = 0;
        }
        if toc_entry > 99 {
            toc_entry = 99;
        }

        /* Now estimate back what frame we get. */
        *get_frame = (toc_entry as f64 / 100.0 * fr.track_frames as f64) as i64;
        /* Question: Is the TOC for whole file size (with/without ID3) or the "real" audio data only?
        ID3v1 info could also matter. */
        ret = (toc[toc_entry as usize] as f64 / 256.0 * fr.rdat.filelen as f64) as i64;
        fr.state_flags &= !FRAME_ACCURATE;
        fr.silent_resync = 1;
    } else if fr.mean_framesize > 0.0 {
        /* Just guess with mean framesize (may be exact with CBR files). */
        /* Query filelen here or not? */
        fr.state_flags &= !FRAME_ACCURATE; /* Fuzzy! */
        fr.silent_resync = 1;
        *get_frame = want_frame;
        ret = (fr.audio_start as f64 + fr.mean_framesize * want_frame as f64) as i64;
    }
    ret
}

/*
find the best frame in index just before the wanted one, seek to there
then step to just before wanted one with INT123_read_frame
do not care tabout the stuff that was in buffer but not played back
everything that left the decoder is counted as played

Decide if you want low latency reaction and accurate timing info or stable long-time playback with buffer!
*/

/// Translation of `INT123_frame_index_find()`.
pub(crate) fn frame_index_find(
    fr: &mut Mpg123Handle<'_>,
    want_frame: i64,
    get_frame: &mut i64,
) -> i64 {
    /* default is file start if no index position */
    let mut gopos;
    *get_frame = 0;
    /* Possibly use VBRI index, too? I'd need an example for this... */
    if fr.index.fill != 0 {
        /* find in index */
        /* at index fi there is frame step*fi... */
        let mut fi = (want_frame / fr.index.step) as usize;
        if fi >= fr.index.fill {
            /* If we are beyond the end of frame index...*/
            /* When fuzzy seek is allowed, we have some limited tolerance for the frames we want to read rather then jump over. */
            if fr.p.flags & MPG123_FUZZY != 0
                && want_frame - (fr.index.fill as i64 - 1) * fr.index.step > 10
            {
                gopos = frame_fuzzy_find(fr, want_frame, get_frame);
                if gopos > fr.audio_start {
                    return gopos; /* Only in that case, we have a useful guess. */
                }
                /* Else... just continue, fuzzyness didn't help. */
            }
            /* Use the last available position, slowly advancing from that one. */
            fi = fr.index.fill - 1;
        }
        /* We have index position, that yields frame and byte offsets. */
        *get_frame = fi as i64 * fr.index.step;
        gopos = fr.index.data[fi];
        fr.state_flags |= FRAME_ACCURATE; /* When using the frame index, we are accurate. */
    } else {
        if fr.p.flags & MPG123_FUZZY != 0 {
            return frame_fuzzy_find(fr, want_frame, get_frame);
        }
        /* A bit hackish here... but we need to be fresh when looking for the first header again. */
        fr.firsthead = 0;
        fr.oldhead = 0;
        gopos = 0;
    }
    gopos
}

/// Translation of `INT123_frame_ins2outs()`.
pub(crate) fn frame_ins2outs(fr: &Mpg123Handle<'_>, ins: i64) -> i64 {
    let mut outs = 0;
    // (built as with NO_DOWNSAMPLE and NO_NTOM: see the crate docs)
    if fr.down_sample == 0 {
        outs = ins >> fr.down_sample;
    }
    outs
}

/// Translation of `INT123_frame_outs()`.
pub(crate) fn frame_outs(fr: &Mpg123Handle<'_>, num: i64) -> i64 {
    let mut outs = 0;
    if fr.down_sample == 0 {
        outs = (fr.spf >> fr.down_sample) * num;
    }
    outs
}

/// Translation of `INT123_frame_expect_outsamples()`: compute the number
/// of output samples we expect from this frame. This is either simple
/// spf() or a tad more elaborate for ntom.
pub(crate) fn frame_expect_outsamples(fr: &Mpg123Handle<'_>) -> i64 {
    let mut outs = 0;
    if fr.down_sample == 0 {
        outs = fr.spf >> fr.down_sample;
    }
    outs
}

/// Translation of `INT123_frame_offset()`.
pub(crate) fn frame_offset(fr: &Mpg123Handle<'_>, outs: i64) -> i64 {
    let mut num = 0;
    if fr.down_sample == 0 {
        num = outs / (fr.spf >> fr.down_sample);
    }
    num
}

/* input in _input_ samples */
/// Translation of `INT123_frame_gapless_init()`.
pub(crate) fn frame_gapless_init(
    fr: &mut Mpg123Handle<'_>,
    framecount: i64,
    bskip: i64,
    eskip: i64,
) {
    fr.gapless_frames = framecount;
    if fr.gapless_frames > 0 && bskip >= 0 && eskip >= 0 {
        fr.begin_s = bskip + GAPLESS_DELAY;
        fr.end_s = framecount.wrapping_mul(fr.spf) - eskip + GAPLESS_DELAY;
    } else {
        fr.begin_s = 0;
        fr.end_s = 0;
    }
    /* These will get proper values later, from above plus resampling info. */
    fr.begin_os = 0;
    fr.end_os = 0;
    fr.fullend_os = 0;
}

/// Translation of `INT123_frame_gapless_realinit()`.
pub(crate) fn frame_gapless_realinit(fr: &mut Mpg123Handle<'_>) {
    fr.begin_os = frame_ins2outs(fr, fr.begin_s);
    fr.end_os = frame_ins2outs(fr, fr.end_s);
    if fr.gapless_frames > 0 {
        fr.fullend_os = frame_ins2outs(fr, fr.gapless_frames.wrapping_mul(fr.spf));
    } else {
        fr.fullend_os = 0;
    }
}

/* At least note when there is trouble... */
/// Translation of `INT123_frame_gapless_update()`.
pub(crate) fn frame_gapless_update(fr: &mut Mpg123Handle<'_>, total_samples: i64) {
    let gapless_samples = fr.gapless_frames.wrapping_mul(fr.spf);
    if fr.gapless_frames < 1 {
        return;
    }

    if gapless_samples > total_samples {
        /* This invalidates the current position... but what should I do? */
        frame_gapless_init(fr, -1, 0, 0);
        frame_gapless_realinit(fr);
        fr.lastframe = -1;
        fr.lastoff = 0;
    }
}

// Well, I take that one for granted... at least layer3.
// The value is needed for mpg123_getstate() in any build.
pub(crate) const GAPLESS_DELAY: i64 = 529;

/* Compute the needed frame to ignore from, for getting accurate/consistent output for intended firstframe. */
fn ignoreframe(fr: &Mpg123Handle<'_>) -> i64 {
    let mut preshift = fr.p.preframes;
    /* Layer 3 _really_ needs at least one frame before. */
    if fr.hdr.lay == 3 && preshift < 1 {
        preshift = 1;
    }
    /* Layer 1 & 2 reall do not need more than 2. */
    if fr.hdr.lay != 3 && preshift > 2 {
        preshift = 2;
    }

    fr.firstframe - preshift
}

/* The frame seek... This is not simply the seek to fe*fr->spf samples in output because we think of _input_ frames here.
Seek to frame offset 1 may be just seek to 200 samples offset in output since the beginning of first frame is delay/padding.
Hm, is that right? OK for the padding stuff, but actually, should the decoder delay be better totally hidden or not?
With gapless, even the whole frame position could be advanced further than requested (since Homey don't play dat). */
/// Translation of `INT123_frame_set_frameseek()`.
pub(crate) fn frame_set_frameseek(fr: &mut Mpg123Handle<'_>, fe: i64) {
    fr.firstframe = fe;
    if fr.p.flags & MPG123_GAPLESS != 0 && fr.gapless_frames > 0 {
        /* Take care of the beginning... */
        let beg_f = frame_offset(fr, fr.begin_os);
        if fe <= beg_f {
            fr.firstframe = beg_f;
            fr.firstoff = fr.begin_os - frame_outs(fr, beg_f);
        } else {
            fr.firstoff = 0;
        }
        /* The end is set once for a track at least, on the INT123_frame_set_frameseek called in get_next_frame() */
        if fr.end_os > 0 {
            fr.lastframe = frame_offset(fr, fr.end_os);
            fr.lastoff = fr.end_os - frame_outs(fr, fr.lastframe);
        } else {
            fr.lastframe = -1;
            fr.lastoff = 0;
        }
    } else {
        fr.firstoff = 0;
        fr.lastoff = 0;
        fr.lastframe = -1;
    }
    fr.ignoreframe = ignoreframe(fr);
}

/// Translation of `INT123_frame_skip()`.
pub(crate) fn frame_skip(fr: &mut Mpg123Handle<'_>) {
    if fr.hdr.lay == 3 {
        set_pointer(fr, 1, 512);
    }
}

/* Sample accurate seek prepare for decoder. */
/* This gets unadjusted output samples and takes resampling into account */
/// Translation of `INT123_frame_set_seek()`.
pub(crate) fn frame_set_seek(fr: &mut Mpg123Handle<'_>, sp: i64) {
    fr.firstframe = frame_offset(fr, sp);
    fr.ignoreframe = ignoreframe(fr);
    /* The sample offset is used for non-gapless mode, too! */
    fr.firstoff = sp - frame_outs(fr, fr.firstframe);
}

fn get_rva(fr: &Mpg123Handle<'_>, peak: Option<&mut f64>, gain: Option<&mut f64>) -> bool {
    let mut p = -1.0;
    let mut g = 0.0;
    let mut ret = false;
    if fr.p.rva != 0 {
        let mut rt = 0;
        /* Should one assume a zero RVA as no RVA? */
        if fr.p.rva == 2 && fr.rva.level[1] != -1 {
            rt = 1;
        }
        if fr.rva.level[rt] != -1 {
            p = fr.rva.peak[rt] as f64;
            g = fr.rva.gain[rt] as f64;
            ret = true; /* Success. */
        }
    }
    if let Some(peak) = peak {
        *peak = p;
    }
    if let Some(gain) = gain {
        *gain = g;
    }
    ret
}

/* adjust the volume, taking both fr->outscale and rva values into account */
/// Translation of `INT123_do_rva()`.
pub(crate) fn do_rva(fr: &mut Mpg123Handle<'_>) {
    let mut peak = 0.0;
    let mut gain = 0.0;
    let mut rvafact = 1.0;
    if get_rva(fr, Some(&mut peak), Some(&mut gain)) {
        rvafact = sdl3::stdlib::math::pow(10.0, gain / 20.0);
    }

    let mut newscale = fr.p.outscale * rvafact;

    /* if peak is unknown (== 0) this check won't hurt */
    if (peak * newscale) > 1.0 {
        newscale = 1.0 / peak;
    }
    /* first rva setting is forced with fr->lastscale < 0 */
    if newscale != fr.lastscale || fr.decoder_change != 0 {
        fr.lastscale = newscale;
        /* It may be too early, actually. */
        if let Some(f) = fr.make_decode_tables {
            f(fr); /* the actual work */
        }
    }
}
