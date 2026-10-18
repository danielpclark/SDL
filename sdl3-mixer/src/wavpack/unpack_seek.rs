// Rust translation of src/unpack_seek.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 1998 - 2013 Conifer Software.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Seeking (`WavpackSeekSample64()`), without `ENABLE_LEGACY`; the DSD
//! decimation context of `ENABLE_DSD` never exists (SDL_mixer doesn't open
//! files with `OPEN_DSD_AS_PCM`).

// This module provides the high-level API for unpacking audio data from
// a specific sample index (i.e., seeking).

use super::common_utils::free_streams;
use super::open_utils::{
    check_header, read_header_bytes, read_next_header, read_wvc_block, try_buffer, unpack_init,
    wavpack_verify_single_block,
};
use super::unpack::unpack_samples;
use super::unpack_dsd::unpack_dsd_samples;
use super::*;

///////////////////////////// executable code ////////////////////////////////

/// `(int64_t) value` as x86-64's `cvttsd2si` does it: out of range (and
/// NaN) is `INT64_MIN`.
fn double_to_int64(value: f64) -> i64 {
    if value.is_nan() || value >= 9223372036854775808.0 || value < -9223372036854775808.0 {
        i64::MIN
    } else {
        value as i64
    }
}

// Seek to the specified sample index, returning TRUE on success. Note that
// files generated with version 4.0 or newer will seek almost immediately.
// Older files can take quite long if required to seek through unplayed
// portions of the file, but will create a seek map so that reverse seeks
// (or forward seeks to already scanned areas) will be very fast. After a
// FALSE return the file should not be accessed again (other than to close
// it); this is a fatal error.

/// Translation of `WavpackSeekSample64()`.
pub(crate) fn wavpack_seek_sample64(wpc: &mut WavpackContext<'_>, sample: i64) -> i32 {
    let mut streams = std::mem::take(&mut wpc.streams);
    let result = seek_sample_streams(wpc, &mut streams, sample);
    wpc.streams = streams;
    result
}

/// Read the block at `pos` of `reader` into `blockbuff` (the `!wps->blockbuff`
/// part of `WavpackSeekSample64()`, for the .wv and the .wvc file): `false`
/// (after `free_streams()`) if it can't be.
fn read_block_at(
    reader: &mut (dyn WavpackStreamReader64 + Send + '_),
    pos: i64,
    wphdr: &mut WavpackHeader,
    verify_checksum: bool,
    initial_index: i64,
) -> Option<Vec<u8>> {
    reader.set_pos_abs(pos);
    read_header_bytes(reader, wphdr);

    if (wphdr.ck_size & 1) != 0 || wphdr.ck_size < 24 || wphdr.ck_size >= 1024 * 1024 {
        return None;
    }

    let mut blockbuff = try_buffer(wphdr.ck_size as usize + 8)?;
    put_header(&mut blockbuff, wphdr);

    let want = wphdr.ck_size as usize - 24;
    if reader.read_bytes(&mut blockbuff[32..32 + want]) != want as i32 {
        return None;
    }

    // render corrupt blocks harmless
    if wavpack_verify_single_block(&blockbuff, verify_checksum) == 0 {
        wphdr.ck_size = 32 - 8;
        wphdr.block_samples = 0;
        put_header(&mut blockbuff, wphdr);
    }

    set_block_index(wphdr, get_block_index(wphdr) - initial_index);
    put_header(&mut blockbuff, wphdr);

    Some(blockbuff)
}

fn seek_sample_streams(
    wpc: &mut WavpackContext<'_>,
    streams: &mut Vec<Box<WavpackStream>>,
    sample: i64,
) -> i32 {
    let mut stream_index = 0usize;

    if streams.is_empty()
        || wpc.total_samples == -1
        || sample >= wpc.total_samples
        || wpc.wv_in.can_seek() == 0
        || (wpc.open_flags & OPEN_STREAMING) != 0
        || (wpc.wvc_flag != 0
            && wpc
                .wvc_in
                .as_mut()
                .is_none_or(|wvc_in| wvc_in.can_seek() == 0))
    {
        return FALSE;
    }

    let wps = &mut streams[0];
    if wps.wphdr.block_samples == 0
        || (wps.wphdr.flags & INITIAL_BLOCK) == 0
        || sample < get_block_index(&wps.wphdr)
        || sample >= get_block_index(&wps.wphdr) + wps.wphdr.block_samples as i64
    {
        free_streams(streams);
        let wps = &mut streams[0];
        let total_samples = wpc.total_samples;
        let initial_index = wpc.initial_index;
        wpc.filepos = find_sample(
            &mut *wpc.wv_in,
            total_samples,
            initial_index,
            wps,
            wpc.filepos,
            sample,
        );

        if wpc.filepos == -1 {
            return FALSE;
        }

        if wpc.wvc_flag != 0 {
            let Some(wvc_in) = wpc.wvc_in.as_mut() else {
                return FALSE;
            };
            wpc.file2pos = find_sample(&mut **wvc_in, total_samples, initial_index, wps, 0, sample);

            if wpc.file2pos == -1 {
                return FALSE;
            }
        }
    }

    let wps = &mut streams[0];
    if wps.blockbuff.is_none() {
        let verify = (wpc.open_flags & OPEN_NO_CHECKSUM) == 0;
        let Some(blockbuff) = read_block_at(
            &mut *wpc.wv_in,
            wpc.filepos,
            &mut wps.wphdr,
            verify,
            wpc.initial_index,
        ) else {
            free_streams(streams);
            return FALSE;
        };
        wps.blockbuff = Some(blockbuff);
        wps.init_done = FALSE;

        if wpc.wvc_flag != 0 {
            let block2buff = match wpc.wvc_in.as_mut() {
                Some(wvc_in) => read_block_at(
                    &mut **wvc_in,
                    wpc.file2pos,
                    &mut wps.wphdr,
                    verify,
                    wpc.initial_index,
                ),
                None => None,
            };
            let Some(block2buff) = block2buff else {
                free_streams(streams);
                return FALSE;
            };
            wps.block2buff = Some(block2buff);
        }

        if wps.init_done == 0 && unpack_init(wpc, wps) == 0 {
            free_streams(streams);
            return FALSE;
        }

        wps.init_done = TRUE;
    }

    while wpc.reduced_channels == 0 && (streams[stream_index].wphdr.flags & FINAL_BLOCK) == 0 {
        stream_index += 1;
        if stream_index == streams.len() {
            if streams.len() as i32 == wpc.max_streams {
                free_streams(streams);
                return FALSE;
            }

            streams.push(Box::default());
            let wps = &mut streams[stream_index];
            wps.stream_index = stream_index as i32;
            let bcount = read_next_header(&mut *wpc.wv_in, &mut wps.wphdr);

            if bcount == u32::MAX {
                free_streams(streams);
                return FALSE;
            }

            let Some(mut blockbuff) = try_buffer(wps.wphdr.ck_size as usize + 8) else {
                free_streams(streams);
                return FALSE;
            };
            put_header(&mut blockbuff, &wps.wphdr);

            let want = wps.wphdr.ck_size as usize - 24;
            if wpc.wv_in.read_bytes(&mut blockbuff[32..32 + want]) != want as i32 {
                free_streams(streams);
                return FALSE;
            }

            // render corrupt blocks harmless
            if wavpack_verify_single_block(&blockbuff, (wpc.open_flags & OPEN_NO_CHECKSUM) == 0)
                == 0
            {
                wps.wphdr.ck_size = 32 - 8;
                wps.wphdr.block_samples = 0;
                put_header(&mut blockbuff, &wps.wphdr);
            }

            // Note (upstream): unlike WavpackUnpackSamples(), this doesn't take
            // the initial index off the block index of the other streams
            // there (so in a file that doesn't start at 0, their blocks
            // matched no correction block and started at the wrong sample);
            // the translation does, as WavpackUnpackSamples() does.
            let index = get_block_index(&wps.wphdr) - wpc.initial_index;
            set_block_index(&mut wps.wphdr, index);
            put_header(&mut blockbuff, &wps.wphdr);
            wps.blockbuff = Some(blockbuff);
            wps.init_done = FALSE;

            if wpc.wvc_flag != 0 && read_wvc_block(wpc, wps) == 0 {
                free_streams(streams);
                return FALSE;
            }

            if wps.init_done == 0 && unpack_init(wpc, wps) == 0 {
                free_streams(streams);
                return FALSE;
            }

            wps.init_done = TRUE;
        }
    }

    if sample < streams[stream_index].sample_index {
        for wps in streams.iter_mut() {
            if unpack_init(wpc, wps) == 0 {
                return FALSE;
            } else {
                wps.init_done = TRUE;
            }
        }
    }

    let samples_to_skip = (sample - streams[stream_index].sample_index) as u32;

    if samples_to_skip > 131072 {
        free_streams(streams);
        return FALSE;
    }

    if samples_to_skip != 0 {
        let mut buffer = vec![0i32; samples_to_skip as usize * 2];

        for wps in streams.iter_mut() {
            if (wps.wphdr.flags & DSD_FLAG) != 0 {
                unpack_dsd_samples(wpc, wps, &mut buffer, samples_to_skip);
            } else {
                unpack_samples(wpc, wps, &mut buffer, samples_to_skip);
            }
        }
    }

    TRUE
}

// Find a valid WavPack header, searching either from the current file position
// (or from the specified position if not -1) and store it (endian corrected)
// at the specified pointer. The return value is the exact file position of the
// header, although we may have actually read past it. Because this function
// is used for seeking to a specific audio sample, it only considers blocks
// that contain audio samples for the initial stream to be valid.

const BUFSIZE: usize = 4096;

/// Translation of `find_header()`.
fn find_header(
    reader: &mut (dyn WavpackStreamReader64 + Send + '_),
    filepos: i64,
    wphdr: &mut WavpackHeader,
) -> i64 {
    let mut buffer = vec![0u8; BUFSIZE];
    let mut sp: i64 = 0;
    let mut ep: i64 = 0;

    if filepos != u32::MAX as i64 && reader.set_pos_abs(filepos) != 0 {
        return -1;
    }

    loop {
        let bleft: i64;

        if sp < ep {
            bleft = ep - sp;
            buffer.copy_within(sp as usize..ep as usize, 0);
            ep -= sp;
            sp = 0;
        } else {
            if sp > ep && reader.set_pos_rel((sp - ep) as i32 as i64, SEEK_CUR) != 0 {
                return -1;
            }

            sp = 0;
            ep = 0;
            bleft = 0;
        }

        ep += reader.read_bytes(&mut buffer[ep as usize..BUFSIZE - bleft as usize + ep as usize])
            as i64;

        if ep - sp < 32 {
            return -1;
        }

        while sp + 32 <= ep {
            match check_header(&buffer, sp as usize) {
                Err(at) => sp = at as i64,
                Ok(()) => {
                    sp += 4;
                    *wphdr = WavpackHeader::from_bytes(&buffer[(sp - 4) as usize..]);

                    if wphdr.block_samples != 0 && (wphdr.flags & INITIAL_BLOCK) != 0 {
                        return reader.get_pos() - (ep - sp + 4);
                    }

                    if wphdr.ck_size > 1024 {
                        sp += wphdr.ck_size as i64 - 1024;
                    }
                }
            }
        }
    }
}

// Find the WavPack block that contains the specified sample. If "header_pos"
// is zero, then no information is assumed except the total number of samples
// in the file and its size in bytes. If "header_pos" is non-zero then we
// assume that it is the file position of the valid header image contained in
// the first stream and we can limit our search to either the portion above
// or below that point. If a .wvc file is being used, then this must be called
// for that file also.

/// Translation of `find_sample()`; `infile` is the stream to search,
/// `wps` is `wpc->streams [0]`.
fn find_sample(
    infile: &mut (dyn WavpackStreamReader64 + Send + '_),
    total_samples: i64,
    initial_index: i64,
    wps: &mut WavpackStream,
    header_pos: i64,
    sample: i64,
) -> i64 {
    let mut file_pos1: i64 = 0;
    let mut file_pos2: i64 = infile.get_length();
    let mut sample_pos1: i64 = 0;
    let mut sample_pos2: i64 = total_samples;
    let mut ratio: f64 = 0.96;
    let mut file_skip = false;

    if sample >= total_samples {
        return -1;
    }

    if header_pos != 0 && wps.wphdr.block_samples != 0 {
        if get_block_index(&wps.wphdr) > sample {
            sample_pos2 = get_block_index(&wps.wphdr);
            file_pos2 = header_pos;
        } else if get_block_index(&wps.wphdr) + wps.wphdr.block_samples as i64 <= sample {
            sample_pos1 = get_block_index(&wps.wphdr);
            file_pos1 = header_pos;
        } else {
            return header_pos;
        }
    }

    loop {
        let mut bytes_per_sample: f64;
        let mut seek_pos: i64;

        bytes_per_sample = file_pos2 as f64 - file_pos1 as f64;
        bytes_per_sample /= sample_pos2.wrapping_sub(sample_pos1) as f64;
        seek_pos = file_pos1 + if file_skip { 32 } else { 0 };
        seek_pos = seek_pos.wrapping_add(double_to_int64(
            bytes_per_sample * sample.wrapping_sub(sample_pos1) as f64 * ratio,
        ));
        seek_pos = find_header(infile, seek_pos, &mut wps.wphdr);

        if seek_pos != -1 {
            let index = get_block_index(&wps.wphdr) - initial_index;
            set_block_index(&mut wps.wphdr, index);
        }

        if seek_pos == -1 || seek_pos >= file_pos2 {
            if ratio > 0.0 {
                ratio -= 0.24;
                if ratio < 0.0 {
                    ratio = 0.0;
                }
            } else {
                return -1;
            }
        } else if get_block_index(&wps.wphdr) > sample {
            sample_pos2 = get_block_index(&wps.wphdr);
            file_pos2 = seek_pos;
        } else if get_block_index(&wps.wphdr) + wps.wphdr.block_samples as i64 <= sample {
            if seek_pos == file_pos1 {
                file_skip = true;
            } else {
                sample_pos1 = get_block_index(&wps.wphdr);
                file_pos1 = seek_pos;
            }
        } else {
            return seek_pos;
        }
    }
}
