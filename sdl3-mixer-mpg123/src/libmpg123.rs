/*
    libmpg123: MPEG Audio Decoder library

    copyright 1995-2023 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org

*/
// Modified 2026-10-17: translated into Rust from libmpg123.c and gapless.h in SDL_mixer 3.3.0's libmpg123 1.33.7 (see NOTICE).

//! The library API: handles, parameters, opening a stream over I/O
//! callbacks, decoding, seeking, the length and the metadata, as the
//! methods of [`Mpg123Handle`].

use crate::format::*;
use crate::frame::*;
use crate::id3::{id3_link, Mpg123Id3v1, Mpg123Id3v2};
use crate::mpg123::*;
use crate::optimize::{frame_cpu_opt, set_synth_functions};
use crate::parse::{compute_bpf, frame_bitrate, frame_freq, read_frame};
use crate::readers::*;

/*
    sampleadjust: gapless sample offset math

    copyright 1995-2023 by the mpg123 project - free software under the terms of the LGPL 2.1
    see COPYING and AUTHORS files in distribution or http://mpg123.org

    This is no stand-alone header, precisely to be able to fool it into using fake handle types for testing the math.
*/

/* From internal sample number to external. */
fn sample_adjust(mh: &Mpg123Handle<'_>, x: i64) -> i64 {
    let s;
    if mh.p.flags & MPG123_GAPLESS != 0 {
        /* It's a bit tricky to do this computation for the padding samples.
        They are not there on the outside. */
        if x > mh.end_os {
            if x < mh.fullend_os {
                s = mh.end_os - mh.begin_os;
            } else {
                s = x - (mh.fullend_os - mh.end_os + mh.begin_os);
            }
        } else {
            s = x - mh.begin_os;
        }
    } else {
        s = x;
    }

    s
}

/* from external samples to internal */
fn sample_unadjust(mh: &Mpg123Handle<'_>, x: i64) -> i64 {
    let mut s;
    if mh.p.flags & MPG123_GAPLESS != 0 {
        s = x + mh.begin_os;
        /* There is a hole; we don't create sample positions in there.
        Jump from the end of the gapless track directly to after the padding. */
        if s >= mh.end_os {
            s += mh.fullend_os - mh.end_os;
        }
    } else {
        s = x;
    }

    s
}

/*
    Take the buffer after a frame decode (strictly: it is the data from frame fr->num!) and cut samples out.
    fr->buffer.fill may then be smaller than before...
*/
fn frame_buffercheck(fr: &mut Mpg123Handle<'_>) {
    /* When we have no accurate position, gapless code does not make sense. */
    if fr.state_flags & FRAME_ACCURATE == 0 {
        return;
    }

    /* Get a grip on dirty streams that start with a gapless header.
    Simply accept all data from frames that are too much,
    they are supposedly attached to the stream after the fact. */
    if fr.gapless_frames > 0 && fr.num >= fr.gapless_frames {
        return;
    }

    /* Important: We first cut samples from the end, then cut from beginning (including left-shift of the buffer).
    This order works also for the case where firstframe == lastframe. */

    /* The last interesting (planned) frame: Only use some leading samples.
    Note a difference from the below: The last frame and offset are unchanges by seeks.
    The lastoff keeps being valid. */
    if fr.lastframe > -1 && fr.num >= fr.lastframe {
        /* There can be more than one frame of padding at the end, so we ignore the whole frame if we are beyond lastframe. */
        let byteoff = if fr.num == fr.lastframe {
            samples_to_bytes(fr, fr.lastoff)
        } else {
            0
        };
        if fr.buffer.fill as i64 > byteoff {
            fr.buffer.fill = byteoff as usize;
        }
    }

    /* The first interesting frame: Skip some leading samples. */
    if fr.firstoff != 0 && fr.num == fr.firstframe {
        let byteoff = samples_to_bytes(fr, fr.firstoff);
        if fr.buffer.fill as i64 > byteoff {
            fr.buffer.fill -= byteoff as usize;
            /* buffer.p != buffer.data only for own buffer */
            if fr.own_buffer != 0 {
                fr.buffer.p = byteoff as usize;
            } else {
                let (b, n) = (byteoff as usize, fr.buffer.fill);
                fr.buffer.data.copy_within(b..b + n, 0);
            }
        } else {
            fr.buffer.fill = 0;
        }

        /* We can only reach this frame again by seeking. And on seeking, firstoff will be recomputed.
        So it is safe to null it here (and it makes the if() decision abort earlier). */
        fr.firstoff = 0;
    }
}

/// `SEEKFRAME(mh)`
fn seekframe(mh: &Mpg123Handle<'_>) -> i64 {
    if mh.ignoreframe < 0 {
        0
    } else {
        mh.ignoreframe
    }
}

/// Translation of `mpg123_init()`: since 1.27.0, this is a no-op and
/// shall stay that way.
pub fn mpg123_init() -> i32 {
    MPG123_OK
}

/// Translation of `mpg123_exit()`: nothing to ever happen here.
pub fn mpg123_exit() {}

/// Translation of `mpg123_plain_strerror()`.
pub fn mpg123_plain_strerror(errcode: i32) -> &'static str {
    static MPG123_ERROR: [&str; 45] = [
        "No error... (code 0)",
        "Unable to set up output format! (code 1)",
        "Invalid channel number specified. (code 2)",
        "Invalid sample rate specified. (code 3)",
        "Unable to allocate memory for 16 to 8 converter table! (code 4)",
        "Bad parameter id! (code 5)",
        "Bad buffer given -- invalid pointer or too small size. (code 6)",
        "Out of memory -- some malloc() failed. (code 7)",
        "You didn't initialize the library! (code 8)",
        "Invalid decoder choice. (code 9)",
        "Invalid mpg123 handle. (code 10)",
        "Unable to initialize frame buffers (out of memory?)! (code 11)",
        "Invalid RVA mode. (code 12)",
        "This build doesn't support gapless decoding. (code 13)",
        "Not enough buffer space. (code 14)",
        "Incompatible numeric data types. (code 15)",
        "Bad equalizer band. (code 16)",
        "Null pointer given where valid storage address needed. (code 17)",
        "Error reading the stream. (code 18)",
        "Cannot seek from end (end is not known). (code 19)",
        "Invalid 'whence' for seek function. (code 20)",
        "Build does not support stream timeouts. (code 21)",
        "File access error. (code 22)",
        "Seek not supported by stream. (code 23)",
        "No stream opened or missing reader setup while opening. (code 24)",
        "Bad parameter handle. (code 25)",
        "Invalid parameter addresses for index retrieval. (code 26)",
        "Lost track in the bytestream and did not attempt resync. (code 27)",
        "Failed to find valid MPEG data within limit on resync. (code 28)",
        "No 8bit encoding possible. (code 29)",
        "Stack alignment is not good. (code 30)",
        "You gave me a NULL buffer? (code 31)",
        "File position is screwed up, please do an absolute seek (code 32)",
        "Inappropriate NULL-pointer provided.",
        "Bad key value given.",
        "There is no frame index (disabled in this build).",
        "Frame index operation failed.",
        "Decoder setup failed (invalid combination of settings?)",
        "Feature not in this build.",
        "Some bad value has been provided.",
        "Low-level seeking has failed (call to lseek(), usually).",
        "Custom I/O obviously not prepared.",
        "Overflow in LFS (large file support) conversion.",
        "Overflow in integer conversion.",
        "Bad IEEE 754 rounding. Re-build libmpg123 properly.",
    ];
    if errcode >= 0 && (errcode as usize) < MPG123_ERROR.len() {
        MPG123_ERROR[errcode as usize]
    } else {
        match errcode {
            MPG123_ERR => "A generic mpg123 error.",
            MPG123_DONE => "Message: I am done with this track.",
            MPG123_NEED_MORE => "Message: Feed me more input data!",
            MPG123_NEW_FORMAT => "Message: Prepare for a changed audio format (query the new one)!",
            _ => "I have no idea - an unknown error code!",
        }
    }
}

/// Translation of `struct mpg123_frameinfo2`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mpg123FrameInfo {
    /// The MPEG version (1.0/2.0/2.5), enum mpg123_version.
    pub version: i32,
    /// The MPEG Audio Layer (MP1/MP2/MP3).
    pub layer: i32,
    /// The sampling rate in Hz.
    pub rate: i64,
    /// The audio mode (enum mpg123_mode, Mono, Stereo, Joint-stero, Dual Channel).
    pub mode: i32,
    /// The mode extension bit flag.
    pub mode_ext: i32,
    /// The size of the frame (in bytes, including header).
    pub framesize: i32,
    /// MPEG Audio flag bits. Bitwise combination of enum mpg123_flags values.
    pub flags: i32,
    /// The emphasis type.
    pub emphasis: i32,
    /// Bitrate of the frame (kbps).
    pub bitrate: i32,
    /// The target average bitrate.
    pub abr_rate: i32,
    /// The VBR mode, enum mpg123_vbr.
    pub vbr: i32,
}

impl<'a> Mpg123Handle<'a> {
    /// Translation of `mpg123_new()`: create a new handle with specified
    /// decoder, decoder can be "", "auto" or `None` for auto-detection
    /// ("generic" is the only decoder of this build). The error is the
    /// `*error` code.
    pub fn new(decoder: Option<&str>) -> Result<Box<Mpg123Handle<'a>>, i32> {
        Self::parnew(None, decoder)
    }

    /// Translation of `mpg123_parnew()`: the full routine with optional
    /// initial parameters to override defaults.
    pub(crate) fn parnew(
        mp: Option<&Mpg123Pars>,
        decoder: Option<&str>,
    ) -> Result<Box<Mpg123Handle<'a>>, i32> {
        // Silly paranoia checks. Really silly, but libmpg123 has been ported to strange
        // platforms in the past.
        /* This is rather pointless but it eases my mind to check that we did
        not enable the special rounding on a VAX or something. */
        if 12346 != crate::synth::ftoi16(12345.67f32) {
            return Err(MPG123_BAD_FLOAT);
        }

        let mut fr = Mpg123Handle::raw();
        frame_init_par(&mut fr, mp);
        if frame_cpu_opt(&mut fr, decoder) != 1 {
            frame_exit(&mut fr);
            return Err(MPG123_BAD_DECODER);
        }
        fr.decoder_change = 1;

        Ok(fr)
    }

    /// Translation of `mpg123_param()` (and `mpg123_param2()`).
    pub fn param(&mut self, key: i32, val: i64, fval: f64) -> i32 {
        let mh = self;
        let mut r = mpg123_par(&mut mh.p, key, val, fval);
        if r != MPG123_OK {
            mh.err = r;
            r = MPG123_ERR;
        } else {
            /* Special treatment for some settings. */
            if key == MPG123_INDEX_SIZE {
                /* Apply frame index size and grow property on the fly. */
                r = frame_index_setup(mh);
                if r != MPG123_OK {
                    mh.err = MPG123_INDEX_FAIL;
                }
            }
        }
        r
    }

    /// Translation of `mpg123_param2()`.
    pub fn param2(&mut self, key: i32, val: i64, fval: f64) -> i32 {
        self.param(key, val, fval)
    }

    /// Translation of `mpg123_getstate()`.
    pub fn getstate(&mut self, key: i32, val: Option<&mut i64>, fval: Option<&mut f64>) -> i32 {
        let mh = self;
        let mut ret = MPG123_OK;
        let mut theval: i64 = 0;
        let thefval = 0.0;

        match key {
            MPG123_ACCURATE => theval = (mh.state_flags & FRAME_ACCURATE) as i64,
            MPG123_FRANKENSTEIN => theval = (mh.state_flags & FRAME_FRANKENSTEIN) as i64,
            MPG123_BUFFERFILL => {
                // (NO_FEEDER)
                mh.err = MPG123_MISSING_FEATURE;
                ret = MPG123_ERR;
            }
            MPG123_FRESH_DECODER => {
                theval = (mh.state_flags & FRAME_FRESH_DECODER) as i64;
                mh.state_flags &= !FRAME_FRESH_DECODER;
            }
            MPG123_ENC_DELAY => theval = mh.enc_delay as i64,
            MPG123_ENC_PADDING => theval = mh.enc_padding as i64,
            MPG123_DEC_DELAY => theval = if mh.hdr.lay == 3 { GAPLESS_DELAY } else { -1 },
            _ => {
                mh.err = MPG123_BAD_KEY;
                ret = MPG123_ERR;
            }
        }

        if let Some(val) = val {
            *val = theval;
        }
        if let Some(fval) = fval {
            *fval = thefval;
        }

        ret
    }

    /// Translation of `mpg123_open_handle64()` (which only works with an
    /// `int64_t` reader setup): `iohandle` is the I/O handle the callbacks
    /// of [`reader64()`](Self::reader64) are called on.
    pub fn open_handle64(&mut self, iohandle: Box<dyn Mpg123Io + 'a>) -> i32 {
        self.close();
        open_stream_handle(self, Some(iohandle))
    }

    /// Translation of `mpg123_reader64()`: the I/O callbacks are the
    /// [`Mpg123Io`] methods of the handle given to
    /// [`open_handle64()`](Self::open_handle64); with `lseek` false, there
    /// is no seek callback (`r_lseek == NULL`, so upstream's `no_lseek64()`).
    /// The cleanup callback is the handle's `Drop`.
    pub fn reader64(&mut self, lseek: bool) -> i32 {
        self.close();

        self.rdat.r_read64 = true;
        self.rdat.lseek_kind = if lseek { Lseek::Handle } else { Lseek::Missing };
        MPG123_OK
    }

    /// Translation of `mpg123_outblock()`.
    pub fn outblock(&self) -> usize {
        /* Try to be helpful and never return zero output block size. */
        if self.outblock > 0 {
            self.outblock
        } else {
            mpg123_safe_buffer()
        }
    }

    /// Translation of `mpg123_read()`: `done` is set to the number of
    /// bytes decoded into `out`.
    pub fn read(&mut self, out: &mut [u8], done: &mut usize) -> i32 {
        mpg123_decode(self, out, done)
    }

    /// Translation of `mpg123_clip()`.
    pub fn clip(&mut self) -> i64 {
        let ret = self.clip;
        self.clip = 0;
        ret
    }

    /// Translation of `mpg123_info2()`.
    pub fn info(&mut self, mi: &mut Mpg123FrameInfo) -> i32 {
        let mh = self;
        let b = init_track(mh);
        if b < 0 {
            return b;
        }

        mi.version = if mh.hdr.mpeg25 != 0 {
            MPG123_2_5
        } else if mh.hdr.lsf != 0 {
            MPG123_2_0
        } else {
            MPG123_1_0
        };
        mi.layer = mh.hdr.lay;
        mi.rate = frame_freq(mh);
        mi.mode = match mh.hdr.mode {
            0 => MPG123_M_STEREO,
            1 => MPG123_M_JOINT,
            2 => MPG123_M_DUAL,
            3 => MPG123_M_MONO,
            _ => 0, /* Nothing good to do here. */
        };
        mi.mode_ext = mh.hdr.mode_ext;
        mi.framesize = mh.hdr.framesize + 4; /* Include header. */
        mi.flags = 0;
        if mh.hdr.error_protection != 0 {
            mi.flags |= MPG123_CRC;
        }
        if mh.hdr.copyright != 0 {
            mi.flags |= MPG123_COPYRIGHT;
        }
        if mh.hdr.extension != 0 {
            mi.flags |= MPG123_PRIVATE;
        }
        if mh.hdr.original != 0 {
            mi.flags |= MPG123_ORIGINAL;
        }
        mi.emphasis = mh.hdr.emphasis;
        mi.bitrate = frame_bitrate(mh);
        mi.abr_rate = mh.abr_rate;
        mi.vbr = mh.vbr;
        MPG123_OK
    }

    /// Translation of `mpg123_getformat2()`.
    pub fn getformat2(
        &mut self,
        rate: Option<&mut i64>,
        channels: Option<&mut i32>,
        encoding: Option<&mut i32>,
        clear_flag: bool,
    ) -> i32 {
        let mh = self;
        let b = init_track(mh);
        if b < 0 {
            return b;
        }

        if let Some(rate) = rate {
            *rate = mh.af.rate;
        }
        if let Some(channels) = channels {
            *channels = mh.af.channels;
        }
        if let Some(encoding) = encoding {
            *encoding = mh.af.encoding;
        }
        if clear_flag {
            mh.new_format = 0;
        }
        MPG123_OK
    }

    /// Translation of `mpg123_getformat()`.
    pub fn getformat(
        &mut self,
        rate: Option<&mut i64>,
        channels: Option<&mut i32>,
        encoding: Option<&mut i32>,
    ) -> i32 {
        self.getformat2(rate, channels, encoding, true)
    }

    /*
        Now, where are we? We need to know the last decoded frame... and what's left of it in buffer.
        The current frame number can mean the last decoded frame or the to-be-decoded frame.
        If mh->to_decode, then mh->num frames have been decoded, the frame mh->num now coming next.
        If not, we have the possibility of mh->num+1 frames being decoded or nothing at all.
        Then, there is firstframe...when we didn't reach it yet, then the next data will come from there.
        mh->num starts with -1
    */
    /// Translation of `mpg123_tell64()`.
    pub fn tell64(&self) -> i64 {
        let mh = self;
        if track_need_init(mh) {
            return 0;
        }
        /* Now we have all the info at hand. */

        {
            /* Funny block to keep C89 happy. */
            let mut pos;
            if (mh.num < mh.firstframe) || (mh.num == mh.firstframe && mh.to_decode != 0) {
                /* We are at the beginning, expect output from firstframe on. */
                pos = frame_outs(mh, mh.firstframe);
                pos += mh.firstoff;
            } else if mh.to_decode != 0 {
                /* We start fresh with this frame. Buffer should be empty, but we make sure to count it in.  */
                pos = frame_outs(mh, mh.num) - bytes_to_samples(mh, mh.buffer.fill as i64);
            } else {
                /* We serve what we have in buffer and then the beginning of next frame... */
                pos = frame_outs(mh, mh.num + 1) - bytes_to_samples(mh, mh.buffer.fill as i64);
            }
            /* Substract padding and delay from the beginning. */
            pos = sample_adjust(mh, pos);
            /* Negative sample offsets are not right, less than nothing is still nothing. */
            if pos > 0 {
                pos
            } else {
                0
            }
        }
    }

    /// Translation of `mpg123_seek64()`. `whence` is `SEEK_SET`,
    /// `SEEK_CUR` or `SEEK_END` (0, 1, 2).
    pub fn seek64(&mut self, sampleoff: i64, whence: i32) -> i64 {
        let mh = self;
        let mut sampleoff = sampleoff;

        let mut pos = mh.tell64(); /* adjusted samples */
        /* pos < 0 also can mean that simply a former seek failed at the lower levels.
        In that case, we only allow absolute seeks. */
        if pos < 0 && whence != SEEK_SET {
            /* Unless we got the obvious error of NULL handle, this is a special seek failure. */
            mh.err = MPG123_NO_RELSEEK;
            return MPG123_ERR as i64;
        }
        let b = init_track(mh);
        if b < 0 {
            return b as i64;
        }
        match whence {
            SEEK_CUR => pos += sampleoff,
            SEEK_SET => pos = sampleoff,
            SEEK_END => {
                // Fix for a bug that existed since the beginning of libmpg123: SEEK_END offsets are
                // also pointing forward for SEEK_END in lseek(). In libmpg123, they used to interpreted
                // as positive from the end towards the beginning. Since just swapping the sign now would
                // break existing programs and seeks beyond the end just don't make sense for a
                // read-only library, we simply ignore the sign and always assumne negative offsets
                // (pointing towards the beginning). Assuming INT64_MIN <= -INT64_MAX.
                if sampleoff > 0 {
                    sampleoff = -sampleoff;
                }
                /* When we do not know the end already, we can try to find it. */
                if mh.track_frames < 1 && (mh.rdat.flags & READER_SEEKABLE) != 0 {
                    mh.scan();
                }
                if mh.track_frames > 0 {
                    pos = sample_adjust(mh, frame_outs(mh, mh.track_frames)) + sampleoff;
                } else if mh.end_os > 0 {
                    pos = sample_adjust(mh, mh.end_os) + sampleoff;
                } else {
                    mh.err = MPG123_NO_SEEK_FROM_END;
                    return MPG123_ERR as i64;
                }
            }
            _ => {
                mh.err = MPG123_BAD_WHENCE;
                return MPG123_ERR as i64;
            }
        }
        if pos < 0 {
            pos = 0;
        }
        /* pos now holds the wanted sample offset in adjusted samples */
        let unadj = sample_unadjust(mh, pos);
        frame_set_seek(mh, unadj);
        let pos = do_the_seek(mh) as i64;
        if pos < 0 {
            return pos;
        }

        mh.tell64()
    }

    /// Translation of `mpg123_length64()`.
    pub fn length64(&mut self) -> i64 {
        let mh = self;
        let b = init_track(mh);
        if b < 0 {
            return b as i64;
        }
        let mut length = if mh.track_samples > -1 {
            mh.track_samples
        } else if mh.track_frames > 0 {
            mh.track_frames * mh.spf
        } else if mh.rdat.filelen > 0 {
            /* Let the case of 0 length just fall through. */
            /* A bad estimate. Ignoring tags 'n stuff. */
            let bpf = if mh.mean_framesize != 0.0 {
                mh.mean_framesize
            } else {
                compute_bpf(mh)
            };
            (mh.rdat.filelen as f64 / bpf * mh.spf as f64) as i64
        } else if mh.rdat.filelen == 0 {
            return mh.tell64(); /* we could be in feeder mode */
        } else {
            return MPG123_ERR as i64; /* No length info there! */
        };

        length = frame_ins2outs(mh, length);
        length = sample_adjust(mh, length);
        length
    }

    /// Translation of `mpg123_scan()`.
    pub fn scan(&mut self) -> i32 {
        let mh = self;
        if mh.rdat.flags & READER_SEEKABLE == 0 {
            mh.err = MPG123_NO_SEEK;
            return MPG123_ERR;
        }
        /* Scan through the _whole_ file, since the current position is no count but computed assuming constant samples per frame. */
        /* Also, we can just keep the current buffer and seek settings. Just operate on input frames here. */
        let b = init_track(mh); /* mh->num >= 0 !! */
        if b < 0 {
            if b == MPG123_DONE {
                return MPG123_OK;
            } else {
                return MPG123_ERR; /* Must be error here, NEED_MORE is not for seekable streams. */
            }
        }
        let oldpos = mh.tell64();
        let b = rd_seek_frame(mh, 0);
        if b < 0 || mh.num != 0 {
            return MPG123_ERR;
        }
        /* One frame must be there now. */
        let mut track_frames: i64 = 1;
        let mut track_samples: i64 = mh.spf; /* Internal samples. */
        /* Do not increment mh->track_frames in the loop as tha would confuse Frankenstein detection. */
        while read_frame(mh) == 1 {
            track_frames += 1;
            track_samples += mh.spf;
        }
        mh.track_frames = track_frames;
        mh.track_samples = track_samples;
        /* Also, think about usefulness of that extra value track_samples ... it could be used for consistency checking. */
        if mh.p.flags & MPG123_GAPLESS != 0 {
            frame_gapless_update(mh, mh.track_samples);
        }
        if mh.seek64(oldpos, SEEK_SET) >= 0 {
            MPG123_OK
        } else {
            MPG123_ERR
        }
    }

    /// Translation of `mpg123_meta_check()`.
    pub fn meta_check(&self) -> i32 {
        self.metaflags
    }

    /// Translation of `mpg123_id3()`: the ID3v1 tag (if the stream has
    /// one) and the ID3v2 data.
    pub fn id3(&mut self) -> (Option<&Mpg123Id3v1>, Option<&Mpg123Id3v2>) {
        let mh = self;
        let mut v1 = false;
        let mut v2 = false;

        if mh.metaflags & MPG123_ID3 != 0 {
            id3_link(mh);
            if mh.rdat.flags & READER_ID3TAG != 0 {
                v1 = true;
            }
            v2 = true;

            mh.metaflags |= MPG123_ID3;
            mh.metaflags &= !MPG123_NEW_ID3;
        }
        (
            if v1 { Some(&mh.id3buf) } else { None },
            if v2 { Some(&mh.id3v2) } else { None },
        )
    }

    /// Translation of `mpg123_id3_raw()`: the raw ID3v1 and ID3v2 tags
    /// (the latter kept with `MPG123_STORE_RAW_ID3`).
    pub fn id3_raw(&self) -> (Option<&[u8]>, Option<&[u8]>) {
        let mh = self;
        let v1 = if mh.rdat.flags & READER_ID3TAG != 0 {
            Some(&mh.id3buf[..])
        } else {
            None
        };
        let v2 = mh.id3v2_raw.as_ref().map(|r| &r[..mh.id3v2_size]);
        (v1, v2)
    }

    /// Translation of `mpg123_icy()`: the last ICY metadata block (with
    /// upstream's terminating NUL) of a stream read with an
    /// `MPG123_ICY_INTERVAL` (which only applies without a seek callback).
    pub fn icy(&mut self) -> Option<&[u8]> {
        let mh = self;
        if mh.metaflags & MPG123_ICY != 0 {
            mh.metaflags |= MPG123_ICY;
            mh.metaflags &= !MPG123_NEW_ICY;
            return mh.icy.data.as_deref();
        }
        None
    }

    /// Translation of `mpg123_close()`.
    pub fn close(&mut self) -> i32 {
        let mh = self;
        /* mh->rd is never NULL! */
        rd_close(mh);

        if mh.new_format != 0 {
            mpg123_invalidate_format(&mut mh.af);
            mh.new_format = 0;
        }
        /* Always reset the frame buffers on close, so we cannot forget it in funky opening routines (wrappers, even). */
        frame_reset(mh);
        MPG123_OK
    }

    /// Translation of `mpg123_errcode()`.
    pub fn errcode(&self) -> i32 {
        self.err
    }

    /// Translation of `mpg123_strerror()`.
    pub fn strerror(&self) -> &'static str {
        mpg123_plain_strerror(self.errcode())
    }

    /// Translation of `mpg123_format_none()`.
    pub fn format_none(&mut self) -> i32 {
        let mut r = mpg123_fmt_none(&mut self.p);
        if r != MPG123_OK {
            self.err = r;
            r = MPG123_ERR;
        }
        r
    }

    /// Translation of `mpg123_format_all()`.
    pub fn format_all(&mut self) -> i32 {
        let mut r = mpg123_fmt_all(&mut self.p);
        if r != MPG123_OK {
            self.err = r;
            r = MPG123_ERR;
        }
        r
    }

    /// Translation of `mpg123_format2()`.
    pub fn format2(&mut self, rate: i64, channels: i32, encodings: i32) -> i32 {
        let mut r = mpg123_fmt2(&mut self.p, rate, channels, encodings);
        if r != MPG123_OK {
            self.err = r;
            r = MPG123_ERR;
        }
        r
    }

    /// Translation of `mpg123_format()`: keep old behaviour.
    pub fn format(&mut self, rate: i64, channels: i32, encodings: i32) -> i32 {
        let mut r = mpg123_fmt(&mut self.p, rate, channels, encodings);
        if r != MPG123_OK {
            self.err = r;
            r = MPG123_ERR;
        }
        r
    }

    /// Translation of `mpg123_format_support()`.
    pub fn format_support(&self, rate: i64, encoding: i32) -> i32 {
        mpg123_fmt_support(&self.p, rate, encoding)
    }

    /// Translation of `mpg123_current_decoder()`.
    pub fn current_decoder(&self) -> &'static str {
        crate::optimize::current_decoder(self)
    }
}

impl Drop for Mpg123Handle<'_> {
    /// Translation of `mpg123_delete()`.
    fn drop(&mut self) {
        self.close();
        frame_exit(self); /* free buffers in frame */
    }
}

/// Translation of `mpg123_par()`.
pub(crate) fn mpg123_par(mp: &mut Mpg123Pars, key: i32, val: i64, fval: f64) -> i32 {
    let mut ret = MPG123_OK;

    match key {
        MPG123_VERBOSE => mp.verbose = val as i32,
        MPG123_FLAGS => {
            if ret == MPG123_OK {
                mp.flags = val;
            }
        }
        MPG123_ADD_FLAGS => mp.flags |= val,
        MPG123_REMOVE_FLAGS => mp.flags &= !val,
        MPG123_FORCE_RATE => {
            /* should this trigger something? */
            // (NO_NTOM)
            if val > 0 {
                ret = MPG123_BAD_RATE;
            }
        }
        MPG123_DOWN_SAMPLE => {
            // (NO_DOWNSAMPLE)
            if val != 0 {
                ret = MPG123_BAD_RATE;
            }
        }
        MPG123_RVA => {
            if !(0..=MPG123_RVA_MAX as i64).contains(&val) {
                ret = MPG123_BAD_RVA;
            } else {
                mp.rva = val as i32;
            }
        }
        MPG123_DOWNSPEED => mp.halfspeed = if val < 0 { 0 } else { val },
        MPG123_UPSPEED => mp.doublespeed = if val < 0 { 0 } else { val },
        MPG123_ICY_INTERVAL => mp.icy_interval = if val > 0 { val } else { 0 },
        MPG123_OUTSCALE => {
            /* Choose the value that is non-zero, if any.
            Downscaling integers to 1.0 . */
            mp.outscale = if val == 0 {
                fval
            } else {
                val as f64 / SHORT_SCALE as f64
            };
        }
        MPG123_TIMEOUT => {
            // (TIMEOUT_READ: the build's NETWORK define, without network code in libmpg123)
            mp.timeout = if val >= 0 { val } else { 0 };
        }
        MPG123_RESYNC_LIMIT => mp.resync_limit = val,
        MPG123_INDEX_SIZE => mp.index_size = val,
        MPG123_PREFRAMES => {
            if val >= 0 {
                mp.preframes = val;
            } else {
                ret = MPG123_BAD_VALUE;
            }
        }
        MPG123_FEEDPOOL | MPG123_FEEDBUFFER => {
            // (NO_FEEDER)
            ret = MPG123_MISSING_FEATURE;
        }
        MPG123_FREEFORMAT_SIZE => mp.freeformat_framesize = val,
        _ => ret = MPG123_BAD_PARAM,
    }
    ret
}

/// Translation of `INT123_decode_update()`: update decoding engine for
/// a) a new choice of decoder, b) a changed native format of the MPEG
/// stream ... calls are only valid after parsing some MPEG frame!
pub(crate) fn decode_update(mh: &mut Mpg123Handle<'_>) -> i32 {
    mh.state_flags &= !FRAME_DECODER_LIVE;
    if mh.num < 0 {
        // INT123_decode_update() has been called before reading the first MPEG frame! Internal programming error.

        mh.err = MPG123_BAD_DECODER_SETUP;
        return MPG123_ERR;
    }

    mh.state_flags |= FRAME_FRESH_DECODER;
    let native_rate = frame_freq(mh);

    let b = frame_output_format(mh); /* Select the new output format based on given constraints. */
    if b < 0 {
        return MPG123_ERR;
    }

    if b == 1 {
        mh.new_format = 1; /* Store for later... */
    }

    if mh.af.rate == native_rate {
        mh.down_sample = 0;
    } else if mh.af.rate == native_rate >> 1 {
        mh.down_sample = 1;
    } else if mh.af.rate == native_rate >> 2 {
        mh.down_sample = 2;
    } else {
        mh.down_sample = 3; /* flexible (fixed) rate */
    }
    match mh.down_sample {
        0..=2 => {
            mh.down_sample_sblimit = SBLIMIT as i32 >> mh.down_sample;
            /* With downsampling I get less samples per frame */
            mh.outblock = outblock_bytes(mh, mh.spf >> mh.down_sample) as usize;
        }
        _ => {}
    }

    if mh.p.flags & MPG123_FORCE_MONO == 0 {
        if mh.af.channels == 1 {
            mh.single = SINGLE_MIX;
        } else {
            mh.single = SINGLE_STEREO;
        }
    } else {
        mh.single = (mh.p.flags & MPG123_FORCE_MONO) as i32 - 1;
    }
    if set_synth_functions(mh) != 0 {
        return -1;
    }

    /* The needed size of output buffer may have changed. */
    if frame_outbuffer(mh) != MPG123_OK {
        return -1;
    }

    do_rva(mh);

    mh.decoder_change = 0;
    mh.state_flags |= FRAME_DECODER_LIVE;
    0
}

/// Translation of `mpg123_safe_buffer()`.
pub fn mpg123_safe_buffer() -> usize {
    /* real is the largest possible output (it's 32bit float, 32bit int or 64bit double). */
    // (NTOM_MAX is 1, as with NO_NTOM)
    4 * 2 * 1152
}

/* Read in the next frame we actually want for decoding.
This includes skipping/ignoring frames, in additon to skipping junk in the parser. */
fn get_next_frame(mh: &mut Mpg123Handle<'_>) -> i32 {
    let mut change = mh.decoder_change;
    /* Ensure we got proper decoder for ignoring frames.
    Header can be changed from seeking around. But be careful: Only after at
    least one frame got read, decoder update makes sense. */
    if mh.header_change > 1 && mh.num >= 0 {
        change = 1;
        mh.header_change = 0;
        if decode_update(mh) < 0 {
            return MPG123_ERR;
        }
    }

    loop {
        /* Decode & discard some frame(s) before beginning. */
        if mh.to_ignore != 0 && mh.num < mh.firstframe && mh.num >= mh.ignoreframe {
            if mh.buffer.size < mh.outblock {
                return MPG123_NO_SPACE;
            }
            /* Decoder structure must be current! INT123_decode_update has been called before... */
            if let Some(f) = mh.do_layer {
                f(mh);
            }
            mh.buffer.fill = 0;
            mh.to_ignore = 0;
            mh.to_decode = 0;
        }
        /* Read new frame data; possibly breaking out here for MPG123_NEED_MORE. */
        mh.to_decode = 0;
        let b = read_frame(mh); /* That sets to_decode only if a full frame was read. */
        if b == MPG123_NEED_MORE {
            return MPG123_NEED_MORE; /* need another call with data */
        } else if b <= 0 {
            /* More sophisticated error control? */
            if b == 0 || (mh.rdat.filelen >= 0 && mh.rdat.filepos == mh.rdat.filelen) {
                /* We simply reached the end. */
                mh.track_frames = mh.num + 1;
                return MPG123_DONE;
            } else {
                return MPG123_ERR; /* Some real error. */
            }
        }
        /* Now, there should be new data to decode ... and also possibly new stream properties */
        if mh.header_change > 1 || mh.decoder_change != 0 {
            change = 1;
            mh.header_change = 0;
            /* Need to update decoder structure right away since frame might need to
            be decoded on next loop iteration for properly ignoring its output. */
            if decode_update(mh) < 0 {
                return MPG123_ERR;
            }
        }
        /* Now some accounting: Look at the numbers and decide if we want this frame. */
        mh.playnum += 1;
        /* Plain skipping without decoding, only when frame is not ignored on next cycle. */
        if mh.num < mh.firstframe || (mh.p.doublespeed != 0 && (mh.playnum % mh.p.doublespeed) != 0)
        {
            if !(mh.to_ignore != 0 && mh.num < mh.firstframe && mh.num >= mh.ignoreframe) {
                frame_skip(mh);
                /* Should one fix NtoM here or not?
                It is not work the trouble for doublespeed, but what with leading frames? */
            }
        }
        /* Or, we are finally done and have a new frame. */
        else {
            break;
        }
    }

    /* If we reach this point, we got a new frame ready to be decoded.
    All other situations resulted in returns from the loop. */
    if change != 0 && mh.fresh != 0 {
        let mut b = 0;
        /* Prepare offsets for gapless decoding. */
        frame_gapless_realinit(mh);
        frame_set_frameseek(mh, mh.num);
        mh.fresh = 0;
        /* Could this possibly happen? With a real big gapless offset... */
        if mh.num < mh.firstframe {
            b = get_next_frame(mh);
        }
        if b < 0 {
            return b; /* Could be error, need for more, new format... */
        }
    }
    MPG123_OK
}

/* Assumption: A buffer full of zero samples can be constructed by repetition of this byte.
Oh, and it handles some format conversion.
Only to be used by decode_the_frame() ... */
fn zero_byte(fr: &Mpg123Handle<'_>) -> u8 {
    if fr.af.encoding & MPG123_ENC_8 != 0 {
        // (fr->conv16to8[0])
        fr.conv16to8_buf.as_ref().map(|b| b[4096]).unwrap_or(0)
    } else {
        0
    }
}

/*
    Not part of the api. This just decodes the frame and fills missing bits with zeroes.
    There can be frames that are broken and thus make do_layer() fail.
*/
fn decode_the_frame(fr: &mut Mpg123Handle<'_>) {
    let needed_bytes = decoder_synth_bytes(fr, frame_expect_outsamples(fr)) as usize;
    if let Some(f) = fr.do_layer {
        let c = f(fr);
        fr.clip += c as i64;
    }
    /* There could be less data than promised.
    Also, then debugging, we look out for coding errors that could result in _more_ data than expected. */
    if fr.buffer.fill < needed_bytes {
        /*
            One could do a loop with individual samples instead... but zero is zero
            Actually, that is wrong: zero is mostly a series of null bytes,
            but we have funny 8bit formats that have a different opinion on zero...
            Unsigned 16 or 32 bit formats are handled later.
        */
        let z = zero_byte(fr);
        let fill = fr.buffer.fill;
        fr.buffer.data[fill..needed_bytes].fill(z);

        fr.buffer.fill = needed_bytes;
    }
    postprocess_buffer(fr);
}

/*
    The old picture:
    while(1) {
        len = read(0,buf,16384);
        if(len <= 0)
            break;
        ret = decodeMP3(&mp,buf,len,out,8192,&size);
        while(ret == MP3_OK) {
            write(1,out,size);
            ret = decodeMP3(&mp,NULL,0,out,8192,&size);
        }
    }
*/

/// Translation of `mpg123_decode()` without input (`mpg123_read()`; the
/// feeder isn't translated).
fn mpg123_decode(mh: &mut Mpg123Handle<'_>, outmem: &mut [u8], done: &mut usize) -> i32 {
    let mut ret = MPG123_OK;
    let mut mdone: usize = 0;
    let outmemsize = outmem.len();
    let mut outmemory = 0usize;

    *done = 0;

    'decodeend: {
        while ret == MPG123_OK {
            /* Decode a frame that has been read before.
            This only happens when buffer is empty! */
            if mh.to_decode != 0 {
                if mh.new_format != 0 {
                    mh.new_format = 0;
                    ret = MPG123_NEW_FORMAT;
                    break 'decodeend;
                }
                if mh.buffer.size - mh.buffer.fill < mh.outblock {
                    ret = MPG123_NO_SPACE;
                    break 'decodeend;
                }
                if (mh.decoder_change != 0 && decode_update(mh) < 0)
                    || (mh.state_flags & FRAME_DECODER_LIVE) == 0
                {
                    ret = MPG123_ERR;
                    break 'decodeend;
                }
                decode_the_frame(mh);
                mh.to_decode = 0;
                mh.to_ignore = 0;
                mh.buffer.p = 0;
                frame_buffercheck(mh);
            }
            if mh.buffer.fill != 0 {
                /* Copy (part of) the decoded data to the caller's buffer. */
                /* get what is needed - or just what is there */
                let a = if mh.buffer.fill > (outmemsize - mdone) {
                    outmemsize - mdone
                } else {
                    mh.buffer.fill
                };
                let p = mh.buffer.p;
                outmem[outmemory..outmemory + a].copy_from_slice(&mh.buffer.data[p..p + a]);
                /* less data in frame buffer, less needed, output pointer increase, more data given... */
                mh.buffer.fill -= a;
                outmemory += a;
                mdone += a;
                mh.buffer.p += a;
                if outmemsize <= mdone {
                    break 'decodeend;
                }
            } else {
                /* If we didn't have data, get a new frame. */
                let b = get_next_frame(mh);
                if b < 0 {
                    ret = b;
                    break 'decodeend;
                }
            }
        }
    }
    // decodeend:
    *done = mdone;
    ret
}

/* Simples: Track needs initializtion if no initial frame has been read yet. */
fn track_need_init(mh: &Mpg123Handle<'_>) -> bool {
    mh.num < 0
}

fn init_track(mh: &mut Mpg123Handle<'_>) -> i32 {
    if track_need_init(mh) {
        /* Fresh track, need first frame for basic info. */
        let b = get_next_frame(mh);
        if b < 0 {
            return b;
        }
    }
    0
}

fn do_the_seek(mh: &mut Mpg123Handle<'_>) -> i32 {
    let fnum = seekframe(mh);
    mh.buffer.fill = 0;

    /* If we are inside the ignoreframe - firstframe window, we may get away without actual seeking. */
    if mh.num < mh.firstframe {
        mh.to_decode = 0; /* In any case, don't decode the current frame, perhaps ignore instead. */
        if mh.num > fnum {
            return MPG123_OK;
        }
    }

    /* If we are already there, we are fine either for decoding or for ignoring. */
    if mh.num == fnum && (mh.to_decode != 0 || fnum < mh.firstframe) {
        return MPG123_OK;
    }
    /* We have the frame before... just go ahead as normal. */
    if mh.num == fnum - 1 {
        mh.to_decode = 0;
        return MPG123_OK;
    }

    /* OK, real seeking follows... clear buffers and go for it. */
    frame_buffers_reset(mh);
    let b = rd_seek_frame(mh, fnum);
    if mh.header_change > 1 {
        if decode_update(mh) < 0 {
            return MPG123_ERR;
        }
        mh.header_change = 0;
    }
    if b < 0 {
        return b;
    }
    /* Only mh->to_ignore is TRUE. */
    if mh.num < mh.firstframe {
        mh.to_decode = 0;
    }

    mh.playnum = mh.num;
    0
}
