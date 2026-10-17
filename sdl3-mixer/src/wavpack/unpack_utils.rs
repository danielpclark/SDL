// Rust translation of src/unpack_utils.c from WavPack 5.9.0
// (https://www.wavpack.com, at the revision SDL_mixer's external/wavpack
// pins).
// Copyright (c) 1998 - 2013 Conifer Software.
// All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `WavpackUnpackSamples()`, without `ENABLE_THREADS` (so none of the
//! worker threads code) or `ENABLE_LEGACY`, with `ENABLE_DSD` (the
//! decimation of `OPEN_DSD_AS_PCM` never runs: SDL_mixer doesn't ask for
//! it).

// This module provides the high-level API for unpacking audio data from
// WavPack files. It manages the buffers used to interleave the data passed
// back to the application from the individual streams. The actual audio
// stream decompression is handled in the unpack.c module.

use super::common_utils::free_streams;
use super::open_utils::{
    read_next_header, read_wvc_block, try_buffer, unpack_init, wavpack_verify_single_block,
};
use super::unpack::unpack_samples;
use super::unpack_dsd::unpack_dsd_samples;
use super::*;

///////////////////////////// executable code ////////////////////////////////

// This function unpacks the specified number of samples from the given stream (which must be
// completely loaded and initialized). The samples are written (interleaved) into the given
// buffer at the specified offset. This function is threadsafe across streams, so it may be
// called directly from the main unpack code or from the worker threads.

/// Translation of `unpack_samples_interleave()`.
fn unpack_samples_interleave(
    wpc: &WavpackContext<'_>,
    wps: &mut WavpackStream,
    outbuf: &mut [i32],
    offset: usize,
    tmpbuf: &mut [i32],
    samcnt: u32,
) {
    let num_channels = wpc.config.num_channels as usize;

    // if the number of channels in the stream exactly matches the channels in the file, we
    // don't actually have to interleave and can render directly without the temp buffer

    if num_channels
        == if (wps.wphdr.flags & MONO_FLAG) != 0 {
            1
        } else {
            2
        }
    {
        if (wps.wphdr.flags & DSD_FLAG) != 0 {
            unpack_dsd_samples(wpc, wps, outbuf, samcnt);
        } else {
            unpack_samples(wpc, wps, outbuf, samcnt);
        }

        return;
    }

    // otherwise we do have to use the temp buffer

    if (wps.wphdr.flags & DSD_FLAG) != 0 {
        unpack_dsd_samples(wpc, wps, tmpbuf, samcnt);
    } else {
        unpack_samples(wpc, wps, tmpbuf, samcnt);
    }

    let samcnt = samcnt as usize;
    let dst = &mut outbuf[offset..];

    // if the block is mono, copy the samples from the single channel into the destination
    // using num_channels as the stride

    if (wps.wphdr.flags & MONO_FLAG) != 0 {
        for k in 0..samcnt {
            dst[k * num_channels] = tmpbuf[k];
        }
    }
    // if the block is stereo, and we don't have room for two more channels, just copy one
    else if offset == num_channels - 1 {
        for k in 0..samcnt {
            dst[k * num_channels] = tmpbuf[k * 2];
        }
    }
    // otherwise copy the stereo samples into the destination
    else {
        for k in 0..samcnt {
            dst[k * num_channels] = tmpbuf[k * 2];
            dst[k * num_channels + 1] = tmpbuf[k * 2 + 1];
        }
    }
}

/// Read the block of a new stream `wps` at the reader's position, as
/// upstream does in `WavpackUnpackSamples()`: its header and the rest of
/// it. `Err(true)` if the header can't be found or the block can't be read
/// (the file is done), `Err(false)` if it can't be allocated.
fn read_stream_block(wpc: &mut WavpackContext<'_>, wps: &mut WavpackStream) -> Result<(), bool> {
    let bcount = read_next_header(&mut *wpc.wv_in, &mut wps.wphdr);

    if bcount == u32::MAX {
        return Err(true);
    }

    let Some(mut blockbuff) = try_buffer(wps.wphdr.ck_size as usize + 8) else {
        return Err(false);
    };

    put_header(&mut blockbuff, &wps.wphdr);

    let want = wps.wphdr.ck_size as usize - 24;
    let got = wpc.wv_in.read_bytes(&mut blockbuff[32..32 + want]);
    wps.blockbuff = Some(blockbuff);
    if got != want as i32 {
        return Err(true);
    }

    Ok(())
}

// Unpack the specified number of samples from the current file position.
// Note that "samples" here refers to "complete" samples, which would be
// 2 longs for stereo files or even more for multichannel files, so the
// required memory at "buffer" is 4 * samples * num_channels bytes. The
// audio data is returned right-justified in 32-bit longs in the endian
// mode native to the executing processor. So, if the original data was
// 16-bit, then the values returned would be +/-32k. Floating point data
// can also be returned if the source was floating point data (and this
// can be optionally normalized to +/-1.0 by using the appropriate flag
// in the call to WavpackOpenFileInput ()). The actual number of samples
// unpacked is returned, which should be equal to the number requested unless
// the end of file is encountered or an error occurs. After all samples have
// been unpacked then 0 will be returned.

/// Translation of `WavpackUnpackSamples()`. `buffer` holds `samples`
/// times the channel count (fewer samples are unpacked if it's shorter).
pub(crate) fn wavpack_unpack_samples(
    wpc: &mut WavpackContext<'_>,
    buffer: &mut [i32],
    samples: u32,
) -> u32 {
    let mut streams = std::mem::take(&mut wpc.streams);
    let result = unpack_samples_streams(wpc, &mut streams, buffer, samples);
    wpc.streams = streams;
    result
}

fn unpack_samples_streams(
    wpc: &mut WavpackContext<'_>,
    streams: &mut Vec<Box<WavpackStream>>,
    buffer: &mut [i32],
    mut samples: u32,
) -> u32 {
    let num_channels = wpc.config.num_channels;
    let mut file_done = false;
    let mut samples_unpacked: u32 = 0;
    let mut samples_to_unpack: u32;
    let mut bptr = 0usize;
    let out_channels = if wpc.reduced_channels != 0 {
        wpc.reduced_channels
    } else {
        num_channels
    }
    .max(1) as usize;

    if streams.is_empty() {
        return 0;
    }

    samples = samples.min((buffer.len() / out_channels) as u32);
    buffer[..out_channels * samples as usize].fill(0);

    while samples != 0 {
        let mut stream_index = 0usize;

        // if the current block has no audio, or it's not the first block of a multichannel
        // sequence, or the sample we're on is past the last sample in this block...we need
        // to free up the streams and read the next block

        let wps = &streams[0];
        if wps.wphdr.block_samples == 0
            || (wps.wphdr.flags & INITIAL_BLOCK) == 0
            || wps.sample_index >= get_block_index(&wps.wphdr) + wps.wphdr.block_samples as i64
        {
            if wpc.wrapper_bytes >= MAX_WRAPPER_BYTES {
                break;
            }

            free_streams(streams);
            let wps = &mut streams[0];
            let nexthdrpos = wpc.wv_in.get_pos();
            let bcount = read_next_header(&mut *wpc.wv_in, &mut wps.wphdr);

            if bcount == u32::MAX {
                break;
            }

            wpc.filepos = nexthdrpos + bcount as i64;

            // allocate the memory for the entire raw block and read it in

            let Some(mut blockbuff) = try_buffer(wps.wphdr.ck_size as usize + 8) else {
                break;
            };

            put_header(&mut blockbuff, &wps.wphdr);

            let want = wps.wphdr.ck_size as usize - 24;
            let got = wpc.wv_in.read_bytes(&mut blockbuff[32..32 + want]);
            if got != want as i32 {
                wps.blockbuff = Some(blockbuff);
                wpc.error_message = "can't read all of last block!".into();
                wps.wphdr.block_samples = 0;
                wps.wphdr.ck_size = 24;
                break;
            }

            // render corrupt blocks harmless
            if wavpack_verify_single_block(&blockbuff, (wpc.open_flags & OPEN_NO_CHECKSUM) == 0)
                == 0
            {
                wps.wphdr.ck_size = 32 - 8;
                wps.wphdr.block_samples = 0;
                put_header(&mut blockbuff, &wps.wphdr);
            }

            // potentially adjusting block_index must be done AFTER verifying block

            if (wpc.open_flags & OPEN_STREAMING) != 0 {
                wps.sample_index = 0;
                set_block_index(&mut wps.wphdr, 0);
            } else {
                let index = get_block_index(&wps.wphdr) - wpc.initial_index;
                set_block_index(&mut wps.wphdr, index);
            }

            put_header(&mut blockbuff, &wps.wphdr);
            wps.blockbuff = Some(blockbuff);
            wps.init_done = FALSE; // we have not yet called unpack_init() for this block

            // if this block has audio, but not the sample index we were expecting, flag an error

            if wpc.reduced_channels == 0
                && wps.wphdr.block_samples != 0
                && wps.sample_index != get_block_index(&wps.wphdr)
            {
                wpc.crc_errors += 1;
            }

            // if this block has audio, and we're in hybrid lossless mode, read the matching wvc block

            if wps.wphdr.block_samples != 0 && wpc.wvc_flag != 0 {
                read_wvc_block(wpc, wps);
            }

            // if the block does NOT have any audio, call unpack_init() to process non-audio stuff

            if wps.wphdr.block_samples == 0 {
                if wps.init_done == 0 && unpack_init(wpc, wps) == 0 {
                    wpc.crc_errors += 1;
                }

                wps.init_done = TRUE;
            }
        }

        // if the current block has no audio, or it's not the first block of a multichannel
        // sequence, or the sample we're on is past the last sample in this block...we need
        // to loop back and read the next block

        let wps = &mut streams[0];
        let block_end = get_block_index(&wps.wphdr) + wps.wphdr.block_samples as i64;
        if wps.wphdr.block_samples == 0
            || (wps.wphdr.flags & INITIAL_BLOCK) == 0
            || wps.sample_index >= block_end
        {
            continue;
        }

        // There seems to be some missing data, like a block was corrupted or something.
        // If it's not too much data, just fill in with silence here and loop back.

        if wps.sample_index < get_block_index(&wps.wphdr) {
            let zvalue: i32 = if (wps.wphdr.flags & DSD_FLAG) != 0 {
                0x55
            } else {
                0
            };

            samples_to_unpack = (get_block_index(&wps.wphdr) - wps.sample_index) as u32;

            if samples_to_unpack == 0 || samples_to_unpack > 262144 {
                wpc.error_message = "discontinuity found, aborting file!".into();
                wps.wphdr.block_samples = 0;
                wps.wphdr.ck_size = 24;
                break;
            }

            if samples_to_unpack > samples {
                samples_to_unpack = samples;
            }

            wps.sample_index += samples_to_unpack as i64;
            samples_unpacked += samples_to_unpack;
            samples -= samples_to_unpack;

            let n = samples_to_unpack as usize * out_channels;
            buffer[bptr..bptr + n].fill(zvalue);
            bptr += n;

            continue;
        }

        // calculate number of samples to process from this block, then initialize the decoder for
        // this block if we haven't already

        samples_to_unpack = (block_end - wps.sample_index) as u32;

        if samples_to_unpack > samples {
            samples_to_unpack = samples;
        }

        if wps.init_done == 0 && unpack_init(wpc, wps) == 0 {
            wpc.crc_errors += 1;
        }

        wps.init_done = TRUE;

        // if this block is not the final block of a multichannel sequence (and we're not truncating
        // to stereo), then enter this conditional block...otherwise we just unpack the samples directly

        if wpc.reduced_channels == 0 && (wps.wphdr.flags & FINAL_BLOCK) == 0 {
            let mut temp_buffer = vec![0i32; samples_to_unpack as usize * 2];
            let mut offset: u32 = 0; // offset to next channel in sequence (0 to num_channels - 1)

            // loop through all the streams...

            loop {
                // if the stream has not been allocated and corresponding block read, do that here...

                if stream_index == streams.len() {
                    streams.push(Box::default());
                    let wps = &mut streams[stream_index];

                    wps.stream_index = stream_index as i32;

                    match read_stream_block(wpc, wps) {
                        Ok(()) => {}
                        Err(true) => {
                            streams[0].wphdr.block_samples = 0;
                            streams[0].wphdr.ck_size = 24;
                            file_done = true;
                            break;
                        }
                        Err(false) => break,
                    }

                    let wps = &mut streams[stream_index];
                    let Some(mut blockbuff) = wps.blockbuff.take() else {
                        break;
                    };

                    // render corrupt blocks harmless
                    if wavpack_verify_single_block(
                        &blockbuff,
                        (wpc.open_flags & OPEN_NO_CHECKSUM) == 0,
                    ) == 0
                    {
                        wps.wphdr.ck_size = 32 - 8;
                        wps.wphdr.block_samples = 0;
                        put_header(&mut blockbuff, &wps.wphdr);
                    }

                    // potentially adjusting block_index must be done AFTER verifying block

                    if (wpc.open_flags & OPEN_STREAMING) != 0 {
                        wps.sample_index = 0;
                        set_block_index(&mut wps.wphdr, 0);
                    } else {
                        let index = get_block_index(&wps.wphdr) - wpc.initial_index;
                        set_block_index(&mut wps.wphdr, index);
                    }

                    put_header(&mut blockbuff, &wps.wphdr);
                    wps.blockbuff = Some(blockbuff);

                    // if this block has audio, and we're in hybrid lossless mode, read the matching wvc block

                    if wpc.wvc_flag != 0 {
                        read_wvc_block(wpc, wps);
                    }

                    // initialize the unpacker for this block

                    if unpack_init(wpc, wps) == 0 {
                        wpc.crc_errors += 1;
                    }

                    wps.init_done = TRUE;
                }

                let wps = &mut streams[stream_index];

                unpack_samples_interleave(
                    wpc,
                    wps,
                    &mut buffer[bptr..],
                    offset as usize,
                    &mut temp_buffer,
                    samples_to_unpack,
                );

                if wps.sample_index == get_block_index(&wps.wphdr) + wps.wphdr.block_samples as i64
                    && wps.mute_error != 0
                {
                    wpc.crc_errors += 1;
                }

                if (wps.wphdr.flags & MONO_FLAG) != 0 {
                    offset += 1;
                } else if offset == (num_channels - 1) as u32 {
                    wpc.crc_errors += 1;
                    offset += 1;
                } else {
                    offset += 2;
                }

                // check several clues that we're done with this set of blocks and exit if we are; else do next stream

                if (wps.wphdr.flags & FINAL_BLOCK) != 0
                    || stream_index as i32 == wpc.max_streams - 1
                    || offset == num_channels as u32
                {
                    break;
                } else {
                    stream_index += 1;
                }
            }

            // if we didn't get all the channels we expected, mute the buffer and flag an error

            if offset != num_channels as u32 {
                let n = samples_to_unpack as usize * num_channels as usize;
                let wps = &streams[stream_index.min(streams.len() - 1)];
                if (wps.wphdr.flags & DSD_FLAG) != 0 {
                    buffer[bptr..bptr + n].fill(0x55);
                } else {
                    buffer[bptr..bptr + n].fill(0);
                }

                wpc.crc_errors += 1;
            }

            // go back to the first stream (we're going to leave them all loaded for now because they might have more samples)
        }
        // catch the error situation where we have only one channel but run into a stereo block
        // (this avoids overwriting the caller's buffer)
        else if (wps.wphdr.flags & MONO_FLAG) == 0
            && (num_channels == 1 || wpc.reduced_channels == 1)
        {
            buffer[bptr..bptr + samples_to_unpack as usize].fill(0);
            wps.sample_index += samples_to_unpack as i64;
            wpc.crc_errors += 1;
        } else {
            if (wps.wphdr.flags & DSD_FLAG) != 0 {
                unpack_dsd_samples(wpc, wps, &mut buffer[bptr..], samples_to_unpack);
            } else {
                unpack_samples(wpc, wps, &mut buffer[bptr..], samples_to_unpack);
            }

            if wps.sample_index == get_block_index(&wps.wphdr) + wps.wphdr.block_samples as i64
                && wps.mute_error != 0
            {
                wpc.crc_errors += 1;
            }
        }

        if file_done {
            wpc.error_message = "can't read all of last block!".into();
            break;
        }

        bptr += samples_to_unpack as usize * out_channels;

        samples_unpacked += samples_to_unpack;
        samples -= samples_to_unpack;

        if wpc.total_samples != -1 && streams[0].sample_index == wpc.total_samples {
            break;
        }
    }

    samples_unpacked
}
