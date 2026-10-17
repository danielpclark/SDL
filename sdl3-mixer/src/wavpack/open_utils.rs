// Rust translation of src/open_utils.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 1998 - 2024 David Bryant.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Opening a file with reader callbacks, finding and verifying blocks,
//! matching correction blocks, and parsing the metadata. Without
//! `ENABLE_LEGACY` (pre-4.0 files are refused) or `ENABLE_THREADS`; with
//! `ENABLE_DSD`. Not translated: `WavpackGetVersion()`,
//! `WavpackGetFileFormat()`, `WavpackGetFileExtension()`,
//! `WavpackSeekTrailingWrapper()` and `WavpackGetMD5Sum()`, which SDL_mixer
//! doesn't call. `bs_close_read()` is in unpack.rs, with its caller.

// This module provides all the code required to open an existing WavPack file
// for reading by using a reader callback mechanism (NOT a filename). This
// includes the code required to find and parse WavPack blocks, process any
// included metadata, and queue up the bitstreams containing the encoded audio
// data. It does not the actual code to unpack audio data and this was done so
// that programs that just want to query WavPack files for information (like,
// for example, taggers) don't need to link in a lot of unnecessary code.

use super::common_utils::SAMPLE_RATES;
use super::decorr_utils::{
    read_decorr_samples, read_decorr_terms, read_decorr_weights, read_shaping_info,
};
use super::entropy_utils::{read_entropy_vars, read_hybrid_profile};
use super::tags::{editable_tag, load_tag};
use super::unpack_dsd::init_dsd_block;
use super::*;

/// A zeroed buffer of `len` bytes, or `None` when it can't be had
/// (upstream's `malloc()` returning `NULL`).
pub(crate) fn try_buffer(len: usize) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).ok()?;
    v.resize(len, 0);
    Some(v)
}

/// `memcpy (&wphdr, ...)` from a read of 32 bytes over the header: the
/// bytes that were read replace the header's.
pub(crate) fn read_header_bytes(
    reader: &mut (dyn WavpackStreamReader64 + Send + '_),
    wphdr: &mut WavpackHeader,
) -> i32 {
    let mut bytes = wphdr.to_bytes();
    let n = reader.read_bytes(&mut bytes);
    *wphdr = WavpackHeader::from_bytes(&bytes);
    n
}

// This function is identical to WavpackOpenFileInput() except that instead
// of providing a filename to open, the caller provides a pointer to a set of
// reader callbacks and instances of up to two streams. The first of these
// streams is required and contains the regular WavPack data stream; the second
// contains the "correction" file if desired. Unlike the standard open
// function which handles the correction file transparently, in this case it
// is the responsibility of the caller to be aware of correction files.

/// Translation of `WavpackOpenFileInputEx64()`. The error (upstream's
/// `error` buffer) is the `Err`.
pub(crate) fn wavpack_open_file_input_ex64<'a>(
    wv_id: Box<dyn WavpackStreamReader64 + Send + 'a>,
    wvc_id: Option<Box<dyn WavpackStreamReader64 + Send + 'a>>,
    flags: i32,
    norm_offset: i32,
) -> Result<WavpackContext<'a>, String> {
    let mut wpc = WavpackContext {
        config: WavpackConfig::default(),
        wrapper_data: None,
        wrapper_bytes: 0,
        wv_in: wv_id,
        wvc_in: wvc_id,
        filelen: 0,
        file2len: 0,
        filepos: 0,
        file2pos: 0,
        total_samples: -1,
        initial_index: 0,
        crc_errors: 0,
        wvc_flag: 0,
        open_flags: flags,
        norm_offset,
        reduced_channels: 0,
        lossy_blocks: 0,
        version_five: 0,
        m_tag: MTag::default(),
        max_streams: OLD_MAX_STREAMS, // use this until overwritten with actual number
        streams: Vec::new(),
        file_format: 0,
        channel_reordering: None,
        channel_identities: None,
        channel_layout: 0,
        dsd_multiplier: 0,
        file_extension: [0; 8],
        error_message: String::new(),
    };
    let mut num_blocks = 0;
    let mut first_byte = [0u8; 1];
    let mut bcount: u32;

    wpc.filelen = wpc.wv_in.get_length();

    if (flags & (OPEN_TAGS | OPEN_EDIT_TAGS)) != 0 && wpc.wv_in.can_seek() != 0 {
        load_tag(&mut wpc);
        wpc.wv_in.set_pos_abs(0);

        if (flags & OPEN_EDIT_TAGS) != 0 && editable_tag(&wpc.m_tag) == 0 {
            return Err("can't edit tags located at the beginning of files!".into());
        }
    }

    if wpc.wv_in.read_bytes(&mut first_byte) != 1 {
        return Err("can't read all of WavPack file!".into());
    }

    wpc.wv_in.push_back_byte(first_byte[0] as i32);

    if first_byte[0] == b'R' {
        return Err(
            "this legacy WavPack file is deprecated, use version 4.80.0 to transcode".into(),
        );
    }

    let mut wps = Box::<WavpackStream>::default();

    while wps.wphdr.block_samples == 0 {
        wpc.filepos = wpc.wv_in.get_pos();
        bcount = read_next_header(&mut *wpc.wv_in, &mut wps.wphdr);

        if bcount == u32::MAX
            || (wps.wphdr.block_samples == 0 && {
                num_blocks += 1;
                num_blocks - 1 > 16
            })
        {
            return Err("not compatible with this version of WavPack file!".into());
        }

        wpc.filepos += bcount as i64;
        let Some(mut blockbuff) = try_buffer(wps.wphdr.ck_size as usize + 8) else {
            return Err("can't allocate memory".into());
        };
        put_header(&mut blockbuff, &wps.wphdr);

        let want = wps.wphdr.ck_size as usize - 24;
        if wpc.wv_in.read_bytes(&mut blockbuff[32..32 + want]) != want as i32 {
            return Err("can't read all of WavPack file!".into());
        }

        // if block does not verify, flag error, free buffer, and continue
        if wavpack_verify_single_block(&blockbuff, (flags & OPEN_NO_CHECKSUM) == 0) == 0 {
            wps.wphdr.block_samples = 0;
            wps.blockbuff = None;
            wpc.crc_errors += 1;
            continue;
        }
        wps.blockbuff = Some(blockbuff);

        wps.init_done = FALSE;

        if wps.wphdr.block_samples != 0 {
            if (flags & OPEN_STREAMING) != 0 {
                set_block_index(&mut wps.wphdr, 0);
            } else if wpc.total_samples == -1 {
                if get_block_index(&wps.wphdr) != 0 || get_total_samples(&wps.wphdr) == -1 {
                    wpc.initial_index = get_block_index(&wps.wphdr);
                    set_block_index(&mut wps.wphdr, 0);

                    if wpc.wv_in.can_seek() != 0 {
                        let mut final_index: i64 = -1;

                        seek_eof_information(&mut wpc, Some(&mut final_index), false);

                        if final_index != -1 {
                            wpc.total_samples = final_index.wrapping_sub(wpc.initial_index);
                        }
                    }
                } else {
                    wpc.total_samples = get_total_samples(&wps.wphdr);
                }
            }
        } else if wpc.total_samples == -1
            && get_block_index(&wps.wphdr) == 0
            && get_total_samples(&wps.wphdr) != 0
        {
            wpc.total_samples = get_total_samples(&wps.wphdr);
        }

        if wps.wphdr.block_samples != 0 && (wps.wphdr.flags & HYBRID_FLAG) != 0 {
            if let Some(wvc_in) = wpc.wvc_in.as_mut() {
                let mut ch = [0u8; 1];

                if wvc_in.read_bytes(&mut ch) == 1 {
                    wvc_in.push_back_byte(ch[0] as i32);
                    wpc.file2len = wvc_in.get_length();
                    wpc.wvc_flag = TRUE;
                }
            }
        }

        if wpc.wvc_flag != 0 && read_wvc_block(&mut wpc, &mut wps) == 0 {
            return Err("not compatible with this version of correction file!".into());
        }

        if wps.init_done == 0 && unpack_init(&mut wpc, &mut wps) == 0 {
            return Err(if !wpc.error_message.is_empty() {
                wpc.error_message.clone()
            } else {
                "not compatible with this version of WavPack file!".into()
            });
        }

        if wps.wphdr.block_samples == 0 {
            // free blockbuff if we're going to loop again
            wps.blockbuff = None;
        }

        wps.init_done = TRUE;
    }

    wpc.config.flags &= !0xff;
    wpc.config.flags |= (wps.wphdr.flags & 0xff) as i32;

    if wpc.config.num_channels == 0 {
        wpc.config.num_channels = if (wps.wphdr.flags & MONO_FLAG) != 0 {
            1
        } else {
            2
        };
        wpc.config.channel_mask = 0x5 - wpc.config.num_channels;
    }

    if (flags & OPEN_2CH_MAX) != 0 && (wps.wphdr.flags & FINAL_BLOCK) == 0 {
        wpc.reduced_channels = if (wps.wphdr.flags & MONO_FLAG) != 0 {
            1
        } else {
            2
        };
    }

    if (wps.wphdr.flags & DSD_FLAG) != 0 {
        if (flags & OPEN_DSD_NATIVE) != 0 {
            wpc.config.bytes_per_sample = 1;
            wpc.config.bits_per_sample = 8;
        } else {
            // (OPEN_DSD_AS_PCM's decimation isn't translated: SDL_mixer
            // doesn't ask for it.)
            let _ = OPEN_DSD_AS_PCM;
            return Err("not configured to handle DSD WavPack files!".into());
        }
    } else {
        wpc.config.bytes_per_sample = (wps.wphdr.flags & BYTES_STORED) as i32 + 1;
        wpc.config.float_norm_exp = wps.float_norm_exp as i32;

        wpc.config.bits_per_sample = (wpc.config.bytes_per_sample * 8)
            - ((wps.wphdr.flags & SHIFT_MASK) >> SHIFT_LSB) as i32;
    }

    if wpc.config.sample_rate == 0 {
        if wps.wphdr.block_samples == 0 || (wps.wphdr.flags & SRATE_MASK) == SRATE_MASK {
            wpc.config.sample_rate = 44100;
        } else {
            wpc.config.sample_rate =
                SAMPLE_RATES[((wps.wphdr.flags & SRATE_MASK) >> SRATE_LSB) as usize] as i32;
        }
    }

    wpc.streams.push(wps);

    Ok(wpc)
}

// This function initializes everything required to unpack a WavPack block
// and must be called before unpack_samples() is called to obtain audio data.
// It is assumed that the WavpackHeader has been read into the wps->wphdr
// (in the current WavpackStream) and that the entire block has been read at
// wps->blockbuff. If a correction file is available (wpc->wvc_flag = TRUE)
// then the corresponding correction block must be read into wps->block2buff
// and its WavpackHeader has overwritten the header at wps->wphdr. This is
// where all the metadata blocks are scanned including those that contain
// bitstream data.

/// Translation of `unpack_init()`, for the stream `wps` (which the caller
/// has taken out of `wpc->streams`).
pub(crate) fn unpack_init(wpc: &mut WavpackContext<'_>, wps: &mut WavpackStream) -> i32 {
    wps.num_terms = 0;
    wps.mute_error = FALSE;
    wps.crc = 0xffffffff;
    wps.crc_x = 0xffffffff;
    wps.dsd.ready = 0;
    wps.wvbits = Bitstream::default();
    wps.wvcbits = Bitstream::default();
    wps.wvxbits = Bitstream::default();
    wps.decorr_passes = [DecorrPass::default(); MAX_NTERMS];
    wps.dc = DcData::default();
    wps.w = WordsData::default();

    if (wps.wphdr.flags & MONO_FLAG) == 0
        && wpc.config.num_channels != 0
        && wps.wphdr.block_samples != 0
        && (wpc.reduced_channels == 1 || wpc.config.num_channels == 1)
    {
        wps.mute_error = TRUE;
        return FALSE;
    }

    if (wps.wphdr.flags & UNKNOWN_FLAGS) != 0 || (wps.wphdr.flags & MONO_DATA) == MONO_DATA {
        wps.mute_error = TRUE;
        return FALSE;
    }

    // FIXME (upstream): when a seek fails after reading a block (in
    // unpack_init() or for the correction block), it frees the block but
    // leaves its header, and the next WavpackUnpackSamples() calls this on
    // a stream with no block buffer, which upstream reads through NULL;
    // here there's no metadata (so the block is muted).
    let blockbuff = wps.blockbuff.take();
    let mut ok = true;
    if let Some(blockbuff) = &blockbuff {
        let mut blockptr = 32usize;

        while let Some(wpmd) = read_metadata_buff(blockbuff, &mut blockptr) {
            if process_metadata(wpc, wps, &wpmd) == 0 {
                ok = false;
                break;
            }
        }
    }
    wps.blockbuff = blockbuff;

    if !ok {
        wps.mute_error = TRUE;
        return FALSE;
    }

    if wps.wphdr.block_samples != 0 && wpc.wvc_flag != 0 && wps.block2buff.is_some() {
        let block2buff = wps.block2buff.take();
        if let Some(block2buff) = &block2buff {
            let mut block2ptr = 32usize;

            while let Some(wpmd) = read_metadata_buff(block2buff, &mut block2ptr) {
                if process_metadata(wpc, wps, &wpmd) == 0 {
                    ok = false;
                    break;
                }
            }
        }
        wps.block2buff = block2buff;

        if !ok {
            wps.mute_error = TRUE;
            return FALSE;
        }
    }

    if wps.wphdr.block_samples != 0
        && if (wps.wphdr.flags & DSD_FLAG) != 0 {
            wps.dsd.ready == 0
        } else {
            !bs_is_open(&wps.wvbits)
        }
    {
        if bs_is_open(&wps.wvcbits) {
            wpc.error_message = "can't unpack correction files alone!".into();
        }

        wps.mute_error = TRUE;
        return FALSE;
    }

    if wps.wphdr.block_samples != 0 && !bs_is_open(&wps.wvxbits) {
        if (wps.wphdr.flags & INT32_DATA) != 0 && wps.int32_sent_bits != 0 {
            wpc.lossy_blocks = TRUE;
        }

        if (wps.wphdr.flags & FLOAT_DATA) != 0
            && (wps.float_flags
                & (FLOAT_EXCEPTIONS | FLOAT_ZEROS_SENT | FLOAT_SHIFT_SENT | FLOAT_SHIFT_SAME))
                != 0
        {
            wpc.lossy_blocks = TRUE;
        }
    }

    if wps.wphdr.block_samples != 0 {
        wps.sample_index = get_block_index(&wps.wphdr);
    }

    TRUE
}

//////////////////////////////// metadata handlers ///////////////////////////////

// These functions handle specific metadata types and are called directly
// during WavPack block parsing by process_metadata() at the bottom.

// This function initializes the main bitstream for audio samples, which must
// be in the "wv" file.

/// Translation of `init_wv_bitstream()`.
fn init_wv_bitstream(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    if wpmd.byte_length == 0 || (wpmd.byte_length & 1) != 0 {
        return FALSE;
    }

    bs_open_read(&mut wps.wvbits, wpmd.bytes());
    TRUE
}

// This function initializes the "correction" bitstream for audio samples,
// which currently must be in the "wvc" file.

/// Translation of `init_wvc_bitstream()`.
fn init_wvc_bitstream(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    if wpmd.byte_length == 0 || (wpmd.byte_length & 1) != 0 {
        return FALSE;
    }

    bs_open_read(&mut wps.wvcbits, wpmd.bytes());
    TRUE
}

// This function initializes the "extra" bitstream for audio samples which
// contains the information required to losslessly decompress 32-bit float data
// or integer data that exceeds 24 bits. This bitstream is in the "wv" file
// for pure lossless data or the "wvc" file for hybrid lossless. This data
// would not be used for hybrid lossy mode. There is also a 32-bit CRC stored
// in the first 4 bytes of these blocks.

/// Translation of `init_wvx_bitstream()`.
fn init_wvx_bitstream(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    let cp = wpmd.bytes();

    if wpmd.byte_length <= 4 || (wpmd.byte_length & 1) != 0 {
        return FALSE;
    }

    wps.crc_wvx = cp[0] as u32;
    wps.crc_wvx |= (cp[1] as u32) << 8;
    wps.crc_wvx |= (cp[2] as u32) << 16;
    wps.crc_wvx |= (cp[3] as u32) << 24;

    bs_open_read(&mut wps.wvxbits, &cp[4..]);

    // the new WVX bitstream format starts with one or two new 5-bit fields

    if wpmd.id == ID_WVX_NEW_BITSTREAM {
        if (wps.wphdr.flags & FLOAT_DATA) != 0 {
            wps.float_min_shifted_zeros = getbits(5, &mut wps.wvxbits) as u8;
            wps.float_min_shifted_zeros &= 0x1f;
            wps.float_max_shifted_ones = getbits(5, &mut wps.wvxbits) as u8;
            wps.float_max_shifted_ones &= 0x1f;
        } else {
            wps.int32_max_width = getbits(5, &mut wps.wvxbits) as u8;
            wps.int32_max_width &= 0x1f;
        }
    }

    TRUE
}

// Read the int32 data from the specified metadata into the specified stream.
// This data is used for integer data that has more than 24 bits of magnitude
// or, in some cases, used to eliminate redundant bits from any audio stream.

/// Translation of `read_int32_info()`.
fn read_int32_info(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    let bytecnt = wpmd.byte_length;
    let byteptr = wpmd.bytes();

    if bytecnt != 4 {
        return FALSE;
    }

    wps.int32_sent_bits = byteptr[0];
    wps.int32_zeros = byteptr[1];
    wps.int32_ones = byteptr[2];
    wps.int32_dups = byteptr[3];

    TRUE
}

/// Translation of `read_float_info()`.
fn read_float_info(wps: &mut WavpackStream, wpmd: &WavpackMetadata<'_>) -> i32 {
    let bytecnt = wpmd.byte_length;
    let byteptr = wpmd.bytes();

    if bytecnt != 4 {
        return FALSE;
    }

    wps.float_flags = byteptr[0];
    wps.float_shift = byteptr[1];
    wps.float_max_exp = byteptr[2];
    wps.float_norm_exp = byteptr[3];
    TRUE
}

// Read multichannel information from metadata. The first byte is the total
// number of channels and the following bytes represent the channel_mask
// as described for Microsoft WAVEFORMATEX.

/// Translation of `read_channel_info()`.
fn read_channel_info(wpc: &mut WavpackContext<'_>, wpmd: &WavpackMetadata<'_>) -> i32 {
    let mut bytecnt = wpmd.byte_length;
    let mut shift = 0;
    let mut mask_bits: i32;
    let data = wpmd.bytes();
    let mut byteptr = 0usize;
    let mut mask: u32 = 0;

    if bytecnt == 0 || bytecnt > 7 {
        return FALSE;
    }

    if wpc.config.num_channels == 0 {
        // if bytecnt is 6 or 7 we are using new configuration with "unlimited" streams

        if bytecnt >= 6 {
            wpc.config.num_channels = (data[0] as i32 | ((data[2] as i32 & 0xf) << 8)) + 1;
            wpc.max_streams = (data[1] as i32 | ((data[2] as i32 & 0xf0) << 4)) + 1;

            if wpc.config.num_channels < wpc.max_streams {
                return FALSE;
            }

            byteptr += 3;
            mask = data[byteptr] as u32;
            byteptr += 1;
            mask |= (data[byteptr] as u32) << 8;
            byteptr += 1;
            mask |= (data[byteptr] as u32) << 16;
            byteptr += 1;

            if bytecnt == 7 {
                // this was introduced in 5.0
                mask |= (data[byteptr] as u32) << 24;
            }
        } else {
            wpc.config.num_channels = data[byteptr] as i32;
            byteptr += 1;

            loop {
                bytecnt -= 1;
                if bytecnt == 0 {
                    break;
                }
                mask |= (data[byteptr] as u32) << shift;
                byteptr += 1;
                shift += 8;
            }

            // FFmpeg 6.1 generates non-compliant multichannel WavPack files that do not use the
            // new (2016) channel format form introduced in WavPack 5.0. This hack allows those
            // files to be decoded by the reference decoder. Surprisingly, the last time I
            // checked, FFmpeg itself can't decode many of them either!

            if wpc.config.num_channels > OLD_MAX_STREAMS {
                wpc.max_streams = wpc.config.num_channels;
            }
        }

        if wpc.config.num_channels > wpc.max_streams * 2 {
            return FALSE;
        }

        wpc.config.channel_mask = mask as i32;

        mask_bits = 0;
        while mask != 0 {
            if (mask & 1) != 0 {
                mask_bits += 1;
                if mask_bits > wpc.config.num_channels {
                    return FALSE;
                }
            }
            mask >>= 1;
        }
    }

    TRUE
}

// Read multichannel identity information from metadata. Data is an array of
// unsigned characters representing any channels in the file that DO NOT
// match one the 18 Microsoft standard channels (and are represented in the
// channel mask). A value of 0 is not allowed and 0xff means an unknown or
// undefined channel identity.

/// Translation of `read_channel_identities()`.
fn read_channel_identities(wpc: &mut WavpackContext<'_>, wpmd: &WavpackMetadata<'_>) -> i32 {
    let Some(idents) = wpmd.data else {
        return FALSE;
    };

    if wpmd.byte_length == 0 {
        return FALSE;
    }

    if idents.contains(&0) {
        return FALSE;
    }

    if wpc.channel_identities.is_none() {
        let mut ids = idents.to_vec();
        ids.push(0);
        wpc.channel_identities = Some(ids);
    }

    TRUE
}

// Read configuration information from metadata.

/// Translation of `read_config_info()`.
fn read_config_info(wpc: &mut WavpackContext<'_>, wpmd: &WavpackMetadata<'_>) -> i32 {
    let mut bytecnt = wpmd.byte_length;
    let data = wpmd.bytes();

    if bytecnt >= 3 {
        wpc.config.flags &= 0xff;
        wpc.config.flags |= ((data[0] as u32) << 8) as i32;
        wpc.config.flags |= ((data[1] as u32) << 16) as i32;
        wpc.config.flags |= ((data[2] as u32) << 24) as i32;
        let mut byteptr = 3usize;
        bytecnt -= 3;

        if bytecnt != 0 && (wpc.config.flags & CONFIG_EXTRA_MODE) != 0 {
            wpc.config.xmode = data[byteptr] as i32;
            byteptr += 1;
            bytecnt -= 1;
        }

        // we used an extra config byte here for the 5.0.0 alpha, so still
        // honor it now (but this has been replaced with NEW_CONFIG)

        if bytecnt != 0 {
            wpc.config.qmode = (wpc.config.qmode & !0xff) | data[byteptr] as i32;
            wpc.version_five = 1;
        }
    }

    TRUE
}

// Read "new" configuration information from metadata.

/// Translation of `read_new_config_info()`.
fn read_new_config_info(wpc: &mut WavpackContext<'_>, wpmd: &WavpackMetadata<'_>) -> i32 {
    let mut bytecnt = wpmd.byte_length;
    let data = wpmd.bytes();
    let mut byteptr = 0usize;

    wpc.version_five = 1; // just having this block signals version 5.0

    wpc.file_format = 0;
    wpc.config.qmode = 0;
    wpc.channel_layout = 0;

    wpc.channel_reordering = None;

    // if there's any data, the first two bytes are file_format and qmode flags

    if bytecnt >= 2 {
        wpc.file_format = data[byteptr];
        byteptr += 1;
        wpc.config.qmode = (wpc.config.qmode & !0xff) | data[byteptr] as i32;
        byteptr += 1;
        bytecnt -= 2;

        // another byte indicates a channel layout

        if bytecnt != 0 {
            wpc.channel_layout = (data[byteptr] as u32) << 16;
            byteptr += 1;
            bytecnt -= 1;

            // another byte means we have a channel count for the layout and maybe a reordering

            if bytecnt != 0 {
                let nchans = data[byteptr] as i32;
                wpc.channel_layout = wpc.channel_layout.wrapping_add(nchans as u32);
                byteptr += 1;
                bytecnt -= 1;

                // any more means there's a reordering string

                if bytecnt != 0 {
                    if bytecnt > nchans {
                        return FALSE;
                    }

                    let mut reordering = vec![0u8; nchans as usize];

                    // note that redundant reordering info is not stored, so we fill in the rest

                    for (i, r) in reordering.iter_mut().enumerate() {
                        if bytecnt != 0 {
                            *r = data[byteptr];
                            byteptr += 1;

                            if *r as i32 >= nchans {
                                // make sure index is in range
                                *r = 0;
                            }

                            bytecnt -= 1;
                        } else {
                            *r = i as u8;
                        }
                    }

                    wpc.channel_reordering = Some(reordering);
                }
            } else {
                wpc.channel_layout = wpc
                    .channel_layout
                    .wrapping_add(wpc.config.num_channels as u32);
            }
        }
    }

    TRUE
}

// Read non-standard sampling rate from metadata.

/// Translation of `read_sample_rate()`.
fn read_sample_rate(wpc: &mut WavpackContext<'_>, wpmd: &WavpackMetadata<'_>) -> i32 {
    let bytecnt = wpmd.byte_length;
    let data = wpmd.bytes();

    if bytecnt == 3 || bytecnt == 4 {
        wpc.config.sample_rate = data[0] as i32;
        wpc.config.sample_rate |= (data[1] as i32) << 8;
        wpc.config.sample_rate |= (data[2] as i32) << 16;

        // for sampling rates > 16777215 (non-audio probably, or ...)

        if bytecnt == 4 {
            wpc.config.sample_rate |= (data[3] as i32 & 0x7f) << 24;
        }
    }

    TRUE
}

// Read wrapper data from metadata. Currently, this consists of the RIFF
// header and trailer that wav files contain around the audio data but could
// be used for other formats as well. Because WavPack files contain all the
// information required for decoding and playback, this data can probably
// be ignored except when an exact wavefile restoration is needed.

/// Translation of `read_wrapper_data()`.
fn read_wrapper_data(wpc: &mut WavpackContext<'_>, wpmd: &WavpackMetadata<'_>) -> i32 {
    if (wpc.open_flags & OPEN_WRAPPER) != 0
        && wpc.wrapper_bytes < MAX_WRAPPER_BYTES
        && wpmd.byte_length != 0
    {
        let wrapper = wpc.wrapper_data.get_or_insert_with(Vec::new);
        if wrapper.try_reserve(wpmd.bytes().len()).is_err() {
            wpc.wrapper_data = None;
            return FALSE;
        }
        wrapper.truncate(wpc.wrapper_bytes as usize);
        wrapper.extend_from_slice(wpmd.bytes());
        wpc.wrapper_bytes += wpmd.byte_length as u32;
    }

    TRUE
}

/// Translation of `read_metadata_buff()`: the next metadata of the block
/// in `blockbuff` at `*buffptr`, or `None` (`FALSE`) at its end or at a
/// malformed one.
fn read_metadata_buff<'b>(blockbuff: &'b [u8], buffptr: &mut usize) -> Option<WavpackMetadata<'b>> {
    let buffend = (buffer_ck_size(blockbuff) as usize + 8).min(blockbuff.len());

    if buffend < *buffptr + 2 {
        return None;
    }

    let mut id = blockbuff[*buffptr];
    *buffptr += 1;
    let mut byte_length = (blockbuff[*buffptr] as i32) << 1;
    *buffptr += 1;

    if (id & ID_LARGE) != 0 {
        id &= !ID_LARGE;

        if buffend < *buffptr + 2 {
            return None;
        }

        byte_length += (blockbuff[*buffptr] as i32) << 9;
        *buffptr += 1;
        byte_length += (blockbuff[*buffptr] as i32) << 17;
        *buffptr += 1;
    }

    if (id & ID_ODD_SIZE) != 0 {
        if byte_length == 0 {
            // odd size and zero length makes no sense
            return None;
        }
        id &= !ID_ODD_SIZE;
        byte_length -= 1;
    }

    let data;
    if byte_length != 0 {
        let padded = byte_length as usize + (byte_length & 1) as usize;
        if buffend - *buffptr < padded {
            return None;
        }

        data = Some(&blockbuff[*buffptr..*buffptr + byte_length as usize]);
        *buffptr += padded;
    } else {
        data = None;
    }

    Some(WavpackMetadata {
        byte_length,
        data,
        id,
    })
}

/// Translation of `process_metadata()`, for the stream `wps`.
fn process_metadata(
    wpc: &mut WavpackContext<'_>,
    wps: &mut WavpackStream,
    wpmd: &WavpackMetadata<'_>,
) -> i32 {
    match wpmd.id {
        ID_DUMMY => TRUE,

        ID_DECORR_TERMS => read_decorr_terms(wps, wpmd),

        ID_DECORR_WEIGHTS => read_decorr_weights(wps, wpmd),

        ID_DECORR_SAMPLES => read_decorr_samples(wps, wpmd),

        ID_ENTROPY_VARS => read_entropy_vars(wps, wpmd),

        ID_HYBRID_PROFILE => read_hybrid_profile(wps, wpmd),

        ID_SHAPING_WEIGHTS => read_shaping_info(wps, wpmd),

        ID_FLOAT_INFO => read_float_info(wps, wpmd),

        ID_INT32_INFO => read_int32_info(wps, wpmd),

        ID_CHANNEL_INFO => read_channel_info(wpc, wpmd),

        ID_CHANNEL_IDENTITIES => read_channel_identities(wpc, wpmd),

        ID_CONFIG_BLOCK => read_config_info(wpc, wpmd),

        ID_NEW_CONFIG_BLOCK => read_new_config_info(wpc, wpmd),

        ID_SAMPLE_RATE => read_sample_rate(wpc, wpmd),

        ID_WV_BITSTREAM => init_wv_bitstream(wps, wpmd),

        ID_WVC_BITSTREAM => init_wvc_bitstream(wps, wpmd),

        ID_WVX_BITSTREAM | ID_WVX_NEW_BITSTREAM => init_wvx_bitstream(wps, wpmd),

        ID_DSD_BLOCK => init_dsd_block(&mut wpc.dsd_multiplier, wps, wpmd),

        ID_ALT_HEADER | ID_ALT_TRAILER if (wpc.open_flags & OPEN_ALT_TYPES) == 0 => TRUE,

        // fall through
        ID_ALT_HEADER | ID_ALT_TRAILER | ID_RIFF_HEADER | ID_RIFF_TRAILER => {
            read_wrapper_data(wpc, wpmd)
        }

        ID_ALT_MD5_CHECKSUM if (wpc.open_flags & OPEN_ALT_TYPES) == 0 => TRUE,

        // fall through
        ID_ALT_MD5_CHECKSUM | ID_MD5_CHECKSUM => {
            if wpmd.byte_length == 16 {
                wpc.config.md5_checksum.copy_from_slice(wpmd.bytes());
                wpc.config.flags |= CONFIG_MD5_CHECKSUM;
                wpc.config.md5_read = 1;
            }

            TRUE
        }

        ID_ALT_EXTENSION => {
            if wpmd.byte_length != 0 && (wpmd.byte_length as usize) < wpc.file_extension.len() {
                let mut j = 0;

                for &c in wpmd.bytes() {
                    if c.is_ascii_alphanumeric() {
                        wpc.file_extension[j] = c;
                        j += 1;
                    }
                }

                wpc.file_extension[j] = 0;
            }

            TRUE
        }

        // we don't actually verify the checksum here (it's done right after the
        // block is read), but it's a good indicator of version 5 files
        ID_BLOCK_CHECKSUM => {
            wpc.version_five = 1;
            TRUE
        }

        _ => {
            if (wpmd.id & ID_OPTIONAL_DATA) != 0 {
                TRUE
            } else {
                FALSE
            }
        }
    }
}

//////////////////////////////// bitstream management ///////////////////////////////

// Open the specified BitStream and associate with the specified buffer.

/// Translation of `bs_open_read()`: the stream gets its own copy of the
/// buffer's words.
fn bs_open_read(bs: &mut Bitstream, buffer: &[u8]) {
    bs.error = 0;
    bs.sr = 0;
    bs.bc = 0;
    bs.buf = buffer
        .chunks_exact(2)
        .map(|w| u16::from_le_bytes([w[0], w[1]]))
        .collect();
    bs.ptr = -1;
    bs.open = true;
}

// Read from current file position until a valid 32-byte WavPack 4.0 header is
// found and read into the specified pointer. The number of bytes skipped is
// returned. If no WavPack header is found within 1 meg, then a -1 is returned
// to indicate the error. No additional bytes are read past the header and it
// is returned in the processor's native endian mode. Seeking is not required.

/// Translation of `read_next_header()`.
pub(crate) fn read_next_header(
    reader: &mut (dyn WavpackStreamReader64 + Send + '_),
    wphdr: &mut WavpackHeader,
) -> u32 {
    let mut buffer = [0u8; 32];
    let mut sp = 32usize;
    let ep = 32usize;
    let mut bytes_skipped: u32 = 0;
    let mut bleft: usize;

    loop {
        if sp < ep {
            bleft = ep - sp;
            buffer.copy_within(sp..ep, 0);
        } else {
            bleft = 0;
        }

        if reader.read_bytes(&mut buffer[bleft..]) != (32 - bleft) as i32 {
            return u32::MAX;
        }

        match check_header(&buffer, 0) {
            Ok(()) => {
                *wphdr = WavpackHeader::from_bytes(&buffer);
                return bytes_skipped;
            }
            Err(at) => sp = at,
        }

        while sp < ep && buffer[sp] != b'w' {
            sp += 1;
        }

        bytes_skipped = bytes_skipped.wrapping_add(sp as u32);
        if bytes_skipped > 1024 * 1024 {
            return u32::MAX;
        }
    }
}

/// Upstream's check for a valid header at `buf[at..]` (`buf` must hold 32
/// bytes from there), written with pointer increments: `*sp++ == 'w' &&
/// *sp == 'v' && *++sp == 'p' && *++sp == 'k' && !(*++sp & 1) && sp [2] <
/// 16 && !sp [3] && (sp [2] || sp [1] || *sp >= 24) && sp [5] == 4 && sp
/// [4] >= (MIN_STREAM_VERS & 0xff) && sp [4] <= (MAX_STREAM_VERS & 0xff) &&
/// sp [18] < 3 && !sp [19]`. `Err` holds where the check left the pointer
/// when it failed (`at + 1` to `at + 4`); after a match it's at `at + 4`.
#[inline]
pub(crate) fn check_header(buf: &[u8], at: usize) -> Result<(), usize> {
    let b = |i: usize| buf[at + i];
    if b(0) != b'w' || b(1) != b'v' {
        return Err(at + 1);
    }
    if b(2) != b'p' {
        return Err(at + 2);
    }
    if b(3) != b'k' {
        return Err(at + 3);
    }
    let sp = |i: usize| b(4 + i);
    if (sp(0) & 1) == 0
        && sp(2) < 16
        && sp(3) == 0
        && (sp(2) != 0 || sp(1) != 0 || sp(0) >= 24)
        && sp(5) == 4
        && sp(4) as u32 >= (MIN_STREAM_VERS & 0xff)
        && sp(4) as u32 <= (MAX_STREAM_VERS & 0xff)
        && sp(18) < 3
        && sp(19) == 0
    {
        return Ok(());
    }
    Err(at + 4)
}

// Compare the regular wv file block header to a potential matching wvc
// file block header and return action code based on analysis:
//
//   0 = use wvc block (assuming rest of block is readable)
//   1 = bad match; try to read next wvc block
//  -1 = bad match; ignore wvc file for this block and backup fp (if
//       possible) and try to use this block next time

/// Translation of `match_wvc_header()`.
fn match_wvc_header(wv_hdr: &WavpackHeader, wvc_hdr: &WavpackHeader) -> i32 {
    if get_block_index(wv_hdr) == get_block_index(wvc_hdr)
        && wv_hdr.block_samples == wvc_hdr.block_samples
    {
        let mut wvi = 0;
        let mut wvci = 0;

        if wv_hdr.flags == wvc_hdr.flags {
            return 0;
        }

        if (wv_hdr.flags & INITIAL_BLOCK) != 0 {
            wvi -= 1;
        }

        if (wv_hdr.flags & FINAL_BLOCK) != 0 {
            wvi += 1;
        }

        if (wvc_hdr.flags & INITIAL_BLOCK) != 0 {
            wvci -= 1;
        }

        if (wvc_hdr.flags & FINAL_BLOCK) != 0 {
            wvci += 1;
        }

        return if wvci - wvi < 0 { 1 } else { -1 };
    }

    if ((get_block_index(wvc_hdr) - get_block_index(wv_hdr)) & 0x8000000000) != 0 {
        1
    } else {
        -1
    }
}

// Read the wvc block that matches the regular wv block that has been
// read for the current stream. If an exact match is not found then
// we either keep reading or back up and (possibly) use the block
// later. The skip_wvc flag is set if not matching wvc block is found
// so that we can still decode using only the lossy version (although
// we flag this as an error). A return of FALSE indicates a serious
// error (not just that we missed one wvc block).

/// Translation of `read_wvc_block()`, for the stream `wps`.
pub(crate) fn read_wvc_block(wpc: &mut WavpackContext<'_>, wps: &mut WavpackStream) -> i32 {
    let mut bcount: i64;
    let mut file2pos: i64;
    let mut orig_wphdr: WavpackHeader;
    let mut wphdr = WavpackHeader::default();
    let mut compare_result: i32;

    let Some(wvc_in) = wpc.wvc_in.as_mut() else {
        return FALSE;
    };

    loop {
        file2pos = wvc_in.get_pos();
        bcount = read_next_header(&mut **wvc_in, &mut wphdr) as i64;

        if bcount == u32::MAX as i64 {
            wps.wvc_skip = TRUE;
            wpc.crc_errors += 1;
            return FALSE;
        }

        orig_wphdr = wphdr; // save original header for verify step

        if (wpc.open_flags & OPEN_STREAMING) != 0 {
            wps.sample_index = 0;
            set_block_index(&mut wphdr, 0);
        } else {
            let index = get_block_index(&wphdr) - wpc.initial_index;
            set_block_index(&mut wphdr, index);
        }

        if (wphdr.flags & INITIAL_BLOCK) != 0 {
            wpc.file2pos = file2pos + bcount;
        }

        compare_result = match_wvc_header(&wps.wphdr, &wphdr);

        if compare_result == 0 {
            let Some(mut block2buff) = try_buffer(wphdr.ck_size as usize + 8) else {
                return FALSE;
            };

            let want = wphdr.ck_size as usize - 24;
            if wvc_in.read_bytes(&mut block2buff[32..32 + want]) != want as i32 {
                wps.block2buff = None;
                wps.wvc_skip = TRUE;
                wpc.crc_errors += 1;
                return FALSE;
            }

            put_header(&mut block2buff, &orig_wphdr);

            // don't use corrupt blocks
            if wavpack_verify_single_block(&block2buff, (wpc.open_flags & OPEN_NO_CHECKSUM) == 0)
                == 0
            {
                wps.block2buff = None;
                wps.wvc_skip = TRUE;
                wpc.crc_errors += 1;
                return TRUE;
            }

            wps.wvc_skip = FALSE;
            put_header(&mut block2buff, &wphdr);
            wps.block2buff = Some(block2buff);
            wps.wphdr = wphdr;
            return TRUE;
        } else if compare_result == -1 {
            wps.wvc_skip = TRUE;
            wvc_in.set_pos_rel(-32, SEEK_CUR);
            wpc.crc_errors += 1;
            return TRUE;
        }
    }
}

// This function is used to seek to end of a file to obtain certain information
// that is stored there at the file creation time because it is not known at
// the start. This includes the MD5 sum and and trailing part of the file
// wrapper, and in some rare cases may include the total number of samples in
// the file (although we usually try to back up and write that at the front of
// the file). Note this function restores the file position to its original
// location (and obviously requires a seekable file). The normal return value
// is TRUE indicating no errors, although this does not actually mean that any
// information was retrieved. An error return of FALSE usually means the file
// terminated unexpectedly. Note that this could be used to get all three
// types of information in one go, but it's not actually used that way now.

/// Translation of `seek_eof_information()`.
fn seek_eof_information(
    wpc: &mut WavpackContext<'_>,
    mut final_index: Option<&mut i64>,
    get_wrapper: bool,
) -> i32 {
    let restore_pos: i64;
    let mut last_pos: i64 = -1;
    let alt_types = (wpc.open_flags & OPEN_ALT_TYPES) != 0;
    let mut blocks: u32 = 0;
    let mut audio_blocks: u32 = 0;
    let mut wphdr = WavpackHeader::default();
    let reader = &mut *wpc.wv_in;

    restore_pos = reader.get_pos(); // we restore file position when done

    // start 1MB from the end-of-file, or from the start if the file is not that big

    if reader.get_length() > 1048576 {
        reader.set_pos_rel(-1048576, SEEK_END);
    } else {
        reader.set_pos_abs(0);
    }

    // Note that we go backward (without parsing inside blocks) until we find a block
    // with audio (careful to not get stuck in a loop). Only then do we go forward
    // parsing all blocks in their entirety.

    loop {
        let mut bcount = read_next_header(reader, &mut wphdr);
        let current_pos = reader.get_pos();

        // if we just got to the same place as last time, we're stuck and need to give up

        if current_pos == last_pos {
            reader.set_pos_abs(restore_pos);
            return FALSE;
        }

        last_pos = current_pos;

        // We enter here if we just read 1 MB without seeing any WavPack block headers.
        // Since WavPack blocks are < 1 MB, that means we're in a big APE tag, or we got
        // to the end-of-file.

        if bcount == u32::MAX {
            // if we have not seen any blocks at all yet, back up almost 2 MB (or to the
            // beginning of the file) and try again

            if blocks == 0 {
                if current_pos > 2000000 {
                    reader.set_pos_rel(-2000000, SEEK_CUR);
                } else {
                    reader.set_pos_abs(0);
                }

                continue;
            }

            // if we have seen WavPack blocks, then this means we've done all we can do here

            reader.set_pos_abs(restore_pos);
            return TRUE;
        }

        blocks += 1;

        // If the block has audio samples, calculate a final index, although this is not
        // final since this may not be the last block with audio. On the other hand, if
        // this block does not have audio, and we haven't seen one with audio, we have
        // to go back some more.

        if wphdr.block_samples != 0 {
            if let Some(final_index) = final_index.as_deref_mut() {
                *final_index = get_block_index(&wphdr) + wphdr.block_samples as i64;
            }

            audio_blocks += 1;
        } else if audio_blocks == 0 {
            if current_pos > 1048576 {
                reader.set_pos_rel(-1048576, SEEK_CUR);
            } else {
                reader.set_pos_abs(0);
            }

            continue;
        }

        // at this point we have seen at least one block with audio, so we parse the
        // entire block looking for MD5 metadata or (conditionally) trailing wrappers

        bcount = wphdr.ck_size.wrapping_sub(32).wrapping_add(8);

        while bcount >= 2 {
            let mut meta_id = [0u8; 1];
            let mut c1 = [0u8; 1];
            let mut c2 = [0u8; 1];
            let mut meta_bc: u32;
            let meta_size: u32;

            if reader.read_bytes(&mut meta_id) != 1 || reader.read_bytes(&mut c1) != 1 {
                reader.set_pos_abs(restore_pos);
                return FALSE;
            }

            meta_bc = (c1[0] as u32) << 1;
            bcount -= 2;

            if (meta_id[0] & ID_LARGE) != 0 {
                if bcount < 2 || reader.read_bytes(&mut c1) != 1 || reader.read_bytes(&mut c2) != 1
                {
                    reader.set_pos_abs(restore_pos);
                    return FALSE;
                }

                meta_bc += ((c1[0] as u32) << 9) + ((c2[0] as u32) << 17);
                bcount -= 2;
            }

            meta_size = if (meta_id[0] & ID_ODD_SIZE) != 0 {
                meta_bc.wrapping_sub(1)
            } else {
                meta_bc
            };
            let meta_id = meta_id[0] & ID_UNIQUE;

            if get_wrapper
                && (meta_id == ID_RIFF_TRAILER || (alt_types && meta_id == ID_ALT_TRAILER))
                && meta_bc != 0
            {
                let wrapper = wpc.wrapper_data.get_or_insert_with(Vec::new);
                wrapper.truncate(wpc.wrapper_bytes as usize);
                if wrapper.try_reserve(meta_bc as usize).is_err() {
                    reader.set_pos_abs(restore_pos);
                    return FALSE;
                }
                let at = wrapper.len();
                wrapper.resize(at + meta_bc as usize, 0);

                if reader.read_bytes(&mut wrapper[at..]) == meta_bc as i32 {
                    wpc.wrapper_bytes = wpc.wrapper_bytes.wrapping_add(meta_size);
                } else {
                    reader.set_pos_abs(restore_pos);
                    return FALSE;
                }
            } else if meta_id == ID_MD5_CHECKSUM || (alt_types && meta_id == ID_ALT_MD5_CHECKSUM) {
                if meta_bc == 16 && bcount >= 16 {
                    if reader.read_bytes(&mut wpc.config.md5_checksum) == 16 {
                        wpc.config.md5_read = TRUE as u8;
                    } else {
                        reader.set_pos_abs(restore_pos);
                        return FALSE;
                    }
                } else {
                    reader.set_pos_rel(meta_bc as i64, SEEK_CUR);
                }
            } else {
                reader.set_pos_rel(meta_bc as i64, SEEK_CUR);
            }

            // FIXME (upstream): this wraps around when a metadata length runs
            // past the end of the block, and the loop goes on parsing what
            // follows the block as metadata (until a read fails).
            bcount = bcount.wrapping_sub(meta_bc);
        }
    }
}

// Quickly verify the referenced block. It is assumed that the WavPack header has been converted
// to native endian format. If a block checksum is performed, that is done in little-endian
// (file) format. It is also assumed that the caller has made sure that the block length
// indicated in the header is correct (we won't overflow the buffer). If a checksum is present,
// then it is checked, otherwise we just check that all the metadata blocks are formatted
// correctly (without looking at their contents). Returns FALSE for bad block.

/// Translation of `WavpackVerifySingleBlock()` (with `BITSTREAM_SHORTS`).
pub(crate) fn wavpack_verify_single_block(buffer: &[u8], verify_checksum: bool) -> i32 {
    let wphdr = WavpackHeader::from_bytes(buffer);
    let mut checksum_passed: u32 = 0;
    let mut bcount: u32;
    let mut meta_bc: u32;
    let mut dp: usize;
    let mut meta_id: u8;
    let mut c1: u8;
    let mut c2: u8;
    let at = |i: usize| buffer.get(i).copied().unwrap_or(0);

    if &wphdr.ck_id != b"wvpk" || wphdr.ck_size.wrapping_add(8) < 32 {
        return FALSE;
    }

    bcount = wphdr.ck_size.wrapping_sub(32).wrapping_add(8);
    dp = 32;

    while bcount >= 2 {
        meta_id = at(dp);
        c1 = at(dp + 1);
        dp += 2;

        meta_bc = (c1 as u32) << 1;
        bcount -= 2;

        if (meta_id & ID_LARGE) != 0 {
            if bcount < 2 {
                return FALSE;
            }

            c1 = at(dp);
            c2 = at(dp + 1);
            dp += 2;
            meta_bc += ((c1 as u32) << 9) + ((c2 as u32) << 17);
            bcount -= 2;
        }

        if bcount < meta_bc {
            return FALSE;
        }

        if verify_checksum && (meta_id & ID_UNIQUE) == ID_BLOCK_CHECKSUM {
            let wcount = (dp - 2) >> 1;
            let mut csum: u32 = u32::MAX;

            if (meta_id & ID_ODD_SIZE) != 0 || meta_bc < 2 || meta_bc > 4 {
                return FALSE;
            }

            for k in 0..wcount {
                csum = csum
                    .wrapping_mul(3)
                    .wrapping_add(at(2 * k) as u32 + ((at(2 * k + 1) as u32) << 8));
            }

            if meta_bc == 4 {
                if at(dp) as u32 != (csum & 0xff)
                    || at(dp + 1) as u32 != ((csum >> 8) & 0xff)
                    || at(dp + 2) as u32 != ((csum >> 16) & 0xff)
                    || at(dp + 3) as u32 != ((csum >> 24) & 0xff)
                {
                    return FALSE;
                }
            } else {
                csum ^= csum >> 16;

                if at(dp) as u32 != (csum & 0xff) || at(dp + 1) as u32 != ((csum >> 8) & 0xff) {
                    return FALSE;
                }
            }

            checksum_passed += 1;
        }

        bcount -= meta_bc;
        dp += meta_bc as usize;
    }

    (bcount == 0 && (!verify_checksum || (wphdr.flags & HAS_CHECKSUM) == 0 || checksum_passed != 0))
        as i32
}
