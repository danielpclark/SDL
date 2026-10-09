// Rust translation of src/obu.c from libavif
// (https://github.com/AOMediaCodec/libavif, at the revision SDL_image's
// external/libavif pins: libavif 1.1.1 with SDL's patches).
//
// This file was originally written by dav1d's authors (see LICENSE.txt):
// Copyright © 2018-2019, VideoLAN and dav1d authors
// Copyright © 2018-2019, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause
//
// libavif's code around it:
// Copyright 2020 Joe Drago. All rights reserved.
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The AV1 sequence header parser libavif uses to harvest the CICP values
//! (and the codec configuration) of an item from its first sample:
//! originally dav1d's getbits.c and parse_seq_hdr(), heavily modified and
//! split, and pulling a few interesting pieces from it.

use super::avif::{
    AvifPixelFormat, AvifRange, AVIF_CHROMA_SAMPLE_POSITION_UNKNOWN, AVIF_COLOR_PRIMARIES_BT709,
    AVIF_COLOR_PRIMARIES_UNSPECIFIED, AVIF_MATRIX_COEFFICIENTS_IDENTITY,
    AVIF_MATRIX_COEFFICIENTS_UNSPECIFIED, AVIF_TRANSFER_CHARACTERISTICS_SRGB,
    AVIF_TRANSFER_CHARACTERISTICS_UNSPECIFIED,
};
use super::internal::{avif_check, AvifCodecType, AvifSequenceHeader};

// ---------------------------------------------------------------------------
// avifBits - Originally dav1d's GetBits struct (see dav1d's getbits.c)

/// Translation of `avifBits` (the pointers are offsets into `data`).
struct AvifBits<'a> {
    error: i32,
    eof: i32,
    state: u64,
    bits_left: u32,
    data: &'a [u8],
    ptr: usize,
}

impl<'a> AvifBits<'a> {
    /// Translation of `avifBitsReadPos()`.
    fn read_pos(&self) -> u32 {
        (self.ptr as u32)
            .wrapping_mul(8)
            .wrapping_sub(self.bits_left)
    }

    /// Translation of `avifBitsInit()`.
    fn init(data: &'a [u8]) -> AvifBits<'a> {
        AvifBits {
            error: 0,
            eof: (data.is_empty()) as i32,
            state: 0,
            bits_left: 0,
            data,
            ptr: 0,
        }
    }

    /// Translation of `avifBitsRefill()`.
    fn refill(&mut self, n: u32) {
        let mut state: u64 = 0;
        loop {
            state <<= 8;
            self.bits_left += 8;
            if self.eof == 0 {
                state |= self.data[self.ptr] as u64;
                self.ptr += 1;
            }
            if self.ptr >= self.data.len() {
                self.error = self.eof;
                self.eof = 1;
            }
            if n <= self.bits_left {
                break;
            }
        }
        self.state |= state << (64 - self.bits_left);
    }

    /// Translation of `avifBitsRead()`.
    fn read(&mut self, n: u32) -> u32 {
        if n > self.bits_left {
            self.refill(n);
        }

        let state = self.state;
        self.bits_left -= n;
        self.state = if n >= 64 { 0 } else { self.state << n };

        (state >> (64 - n)) as u32
    }

    /// Translation of `avifBitsReadUleb128()`.
    fn read_uleb128(&mut self) -> u32 {
        let mut val: u64 = 0;
        let mut more;
        let mut i: u32 = 0;

        loop {
            let v = self.read(8);
            more = v & 0x80;
            val |= ((v & 0x7F) as u64) << i;
            i += 7;
            if !(more != 0 && i < 56) {
                break;
            }
        }

        if val > u32::MAX as u64 || more != 0 {
            self.error = 1;
            return 0;
        }

        val as u32
    }

    /// Translation of `avifBitsReadVLC()`.
    fn read_vlc(&mut self) -> u32 {
        let mut num_bits = 0;
        while self.read(1) == 0 {
            num_bits += 1;
            if num_bits == 32 {
                return 0xFFFFFFFF;
            }
        }
        if num_bits != 0 {
            ((1u32 << num_bits) - 1).wrapping_add(self.read(num_bits))
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Variables in here use snake_case to self-document from the AV1 spec and the draft AV2 spec:
//
// https://aomediacodec.github.io/av1-spec/av1-spec.pdf
//
// Originally dav1d's parse_seq_hdr() function (heavily modified and split)

/// Translation of `parseSequenceHeaderProfile()`.
fn parse_sequence_header_profile(bits: &mut AvifBits<'_>, header: &mut AvifSequenceHeader) -> bool {
    let seq_profile = bits.read(3);
    if seq_profile > 2 {
        return false;
    }
    header.av1c.seq_profile = seq_profile as u8;

    let still_picture = bits.read(1);
    header.reduced_still_picture_header = bits.read(1) as u8;
    if header.reduced_still_picture_header != 0 && still_picture == 0 {
        return false;
    }

    if header.reduced_still_picture_header != 0 {
        header.av1c.seq_level_idx0 = bits.read(5) as u8;
        header.av1c.seq_tier0 = 0;
    } else {
        let timing_info_present_flag = bits.read(1);
        let mut decoder_model_info_present_flag = 0;
        let mut buffer_delay_length = 0;
        if timing_info_present_flag != 0 {
            // timing_info()
            bits.read(32); // num_units_in_display_tick
            bits.read(32); // time_scale
            let equal_picture_interval = bits.read(1);
            if equal_picture_interval != 0 {
                let num_ticks_per_picture_minus_1 = bits.read_vlc();
                if num_ticks_per_picture_minus_1 == 0xFFFFFFFF {
                    return false;
                }
            }

            decoder_model_info_present_flag = bits.read(1);
            if decoder_model_info_present_flag != 0 {
                // decoder_model_info()
                buffer_delay_length = bits.read(5) + 1;
                bits.read(32); // num_units_in_decoding_tick
                bits.read(10); // buffer_removal_time_length_minus_1, frame_presentation_time_length_minus_1
            }
        }

        let initial_display_delay_present_flag = bits.read(1);
        let operating_points_cnt = bits.read(5) + 1;
        for i in 0..operating_points_cnt {
            bits.read(12); // operating_point_idc
            let seq_level_idx = bits.read(5);
            if i == 0 {
                header.av1c.seq_level_idx0 = seq_level_idx as u8;
                header.av1c.seq_tier0 = 0;
            }
            if seq_level_idx > 7 {
                let seq_tier = bits.read(1);
                if i == 0 {
                    header.av1c.seq_tier0 = seq_tier as u8;
                }
            }
            if decoder_model_info_present_flag != 0 {
                let decoder_model_present_for_this_op = bits.read(1);
                if decoder_model_present_for_this_op != 0 {
                    // operating_parameters_info()
                    bits.read(buffer_delay_length); // decoder_buffer_delay
                    bits.read(buffer_delay_length); // encoder_buffer_delay
                    bits.read(1); // low_delay_mode_flag
                }
            }
            if initial_display_delay_present_flag != 0 {
                let initial_display_delay_present_for_this_op = bits.read(1);
                if initial_display_delay_present_for_this_op != 0 {
                    bits.read(4); // initial_display_delay_minus_1
                }
            }
        }
    }
    bits.error == 0
}

/// Translation of `parseSequenceHeaderFrameMaxDimensions()`.
fn parse_sequence_header_frame_max_dimensions(
    bits: &mut AvifBits<'_>,
    header: &mut AvifSequenceHeader,
) -> bool {
    let frame_width_bits = bits.read(4) + 1;
    let frame_height_bits = bits.read(4) + 1;
    header.max_width = bits.read(frame_width_bits).wrapping_add(1); // max_frame_width
    header.max_height = bits.read(frame_height_bits).wrapping_add(1); // max_frame_height
    let mut frame_id_numbers_present_flag = 0;
    if header.reduced_still_picture_header == 0 {
        frame_id_numbers_present_flag = bits.read(1);
    }
    if frame_id_numbers_present_flag != 0 {
        bits.read(7); // delta_frame_id_length_minus_2, additional_frame_id_length_minus_1
    }
    bits.error == 0
}

/// Translation of `parseSequenceHeaderEnabledFeatures()`.
fn parse_sequence_header_enabled_features(
    bits: &mut AvifBits<'_>,
    header: &mut AvifSequenceHeader,
) -> bool {
    bits.read(2); // enable_filter_intra, enable_intra_edge_filter

    if header.reduced_still_picture_header == 0 {
        bits.read(4); // enable_interintra_compound, enable_masked_compound, enable_warped_motion, enable_dual_filter
        let enable_order_hint = bits.read(1);
        if enable_order_hint != 0 {
            bits.read(2); // enable_jnt_comp, enable_ref_frame_mvs
        }

        let seq_force_screen_content_tools;
        let seq_choose_screen_content_tools = bits.read(1);
        if seq_choose_screen_content_tools != 0 {
            seq_force_screen_content_tools = 2;
        } else {
            seq_force_screen_content_tools = bits.read(1);
        }
        if seq_force_screen_content_tools > 0 {
            let seq_choose_integer_mv = bits.read(1);
            if seq_choose_integer_mv == 0 {
                bits.read(1); // seq_force_integer_mv
            }
        }
        if enable_order_hint != 0 {
            bits.read(3); // order_hint_bits_minus_1
        }
    }

    bits.error == 0
}

/// Translation of `parseSequenceHeaderColorConfig()`.
fn parse_sequence_header_color_config(
    bits: &mut AvifBits<'_>,
    header: &mut AvifSequenceHeader,
) -> bool {
    header.bit_depth = 8;
    header.chroma_sample_position = AVIF_CHROMA_SAMPLE_POSITION_UNKNOWN;
    header.av1c.chroma_sample_position = header.chroma_sample_position as u8;
    let high_bitdepth = bits.read(1);
    header.av1c.high_bitdepth = high_bitdepth as u8;
    if (header.av1c.seq_profile == 2) && high_bitdepth != 0 {
        let twelve_bit = bits.read(1);
        header.bit_depth = if twelve_bit != 0 { 12 } else { 10 };
        header.av1c.twelve_bit = twelve_bit as u8;
    } else
    /* if (seq_profile <= 2) */
    {
        header.bit_depth = if high_bitdepth != 0 { 10 } else { 8 };
        header.av1c.twelve_bit = 0;
    }
    let mut mono_chrome = 0;
    if header.av1c.seq_profile != 1 {
        mono_chrome = bits.read(1);
    }
    header.av1c.monochrome = mono_chrome as u8;
    let color_description_present_flag = bits.read(1);
    if color_description_present_flag != 0 {
        header.color_primaries = bits.read(8) as u16; // color_primaries
        header.transfer_characteristics = bits.read(8) as u16; // transfer_characteristics
        header.matrix_coefficients = bits.read(8) as u16; // matrix_coefficients
    } else {
        header.color_primaries = AVIF_COLOR_PRIMARIES_UNSPECIFIED;
        header.transfer_characteristics = AVIF_TRANSFER_CHARACTERISTICS_UNSPECIFIED;
        header.matrix_coefficients = AVIF_MATRIX_COEFFICIENTS_UNSPECIFIED;
    }
    if mono_chrome != 0 {
        header.range = if bits.read(1) != 0 {
            AvifRange::Full
        } else {
            AvifRange::Limited
        }; // color_range
        header.av1c.chroma_subsampling_x = 1;
        header.av1c.chroma_subsampling_y = 1;
        header.yuv_format = AvifPixelFormat::Yuv400;
    } else if header.color_primaries == AVIF_COLOR_PRIMARIES_BT709
        && header.transfer_characteristics == AVIF_TRANSFER_CHARACTERISTICS_SRGB
        && header.matrix_coefficients == AVIF_MATRIX_COEFFICIENTS_IDENTITY
    {
        header.range = AvifRange::Full;
        header.av1c.chroma_subsampling_x = 0;
        header.av1c.chroma_subsampling_y = 0;
        header.yuv_format = AvifPixelFormat::Yuv444;
    } else {
        #[allow(unused_assignments)]
        let mut subsampling_x = 0;
        #[allow(unused_assignments)]
        let mut subsampling_y = 0;
        header.range = if bits.read(1) != 0 {
            AvifRange::Full
        } else {
            AvifRange::Limited
        }; // color_range
        match header.av1c.seq_profile {
            0 => {
                subsampling_x = 1;
                subsampling_y = 1;
                header.yuv_format = AvifPixelFormat::Yuv420;
            }
            1 => {
                subsampling_x = 0;
                subsampling_y = 0;
                header.yuv_format = AvifPixelFormat::Yuv444;
            }
            2 => {
                if header.bit_depth == 12 {
                    subsampling_x = bits.read(1);
                    if subsampling_x != 0 {
                        subsampling_y = bits.read(1);
                    }
                } else {
                    subsampling_x = 1;
                    subsampling_y = 0;
                }
                if subsampling_x != 0 {
                    header.yuv_format = if subsampling_y != 0 {
                        AvifPixelFormat::Yuv420
                    } else {
                        AvifPixelFormat::Yuv422
                    };
                } else {
                    header.yuv_format = AvifPixelFormat::Yuv444;
                }
            }
            _ => return false,
        }

        if subsampling_x != 0 && subsampling_y != 0 {
            header.chroma_sample_position = bits.read(2); // chroma_sample_position
            header.av1c.chroma_sample_position = header.chroma_sample_position as u8;
        }
        header.av1c.chroma_subsampling_x = subsampling_x as u8;
        header.av1c.chroma_subsampling_y = subsampling_y as u8;
    }

    if mono_chrome == 0 {
        bits.read(1); // separate_uv_delta_q
    }

    bits.error == 0
}

/// Translation of `parseAV1SequenceHeader()`.
fn parse_av1_sequence_header(bits: &mut AvifBits<'_>, header: &mut AvifSequenceHeader) -> bool {
    avif_check!(parse_sequence_header_profile(bits, header));

    avif_check!(parse_sequence_header_frame_max_dimensions(bits, header));
    bits.read(1); // use_128x128_superblock
    avif_check!(parse_sequence_header_enabled_features(bits, header));

    bits.read(3); // enable_superres, enable_cdef, enable_restoration

    avif_check!(parse_sequence_header_color_config(bits, header));

    bits.read(1); // film_grain_params_present
    bits.error == 0
}

/// Translation of `avifSequenceHeaderParse()`.
pub(crate) fn avif_sequence_header_parse(
    header: &mut AvifSequenceHeader,
    sample: &[u8],
    codec_type: AvifCodecType,
) -> bool {
    let mut obus = sample;

    // Find the sequence header OBU
    while !obus.is_empty() {
        let mut bits = AvifBits::init(obus);

        // obu_header()
        let obu_forbidden_bit = bits.read(1);
        if obu_forbidden_bit != 0 {
            return false;
        }
        let obu_type = bits.read(4);
        let obu_extension_flag = bits.read(1);
        let obu_has_size_field = bits.read(1);
        bits.read(1); // obu_reserved_1bit

        if obu_extension_flag != 0 {
            // obu_extension_header()
            bits.read(8); // temporal_id, spatial_id, extension_header_reserved_3bits
        }

        let obu_size: u32 = if obu_has_size_field != 0 {
            bits.read_uleb128()
        } else {
            (obus.len() as i32)
                .wrapping_sub(1)
                .wrapping_sub(obu_extension_flag as i32) as u32
        };

        if bits.error != 0 {
            return false;
        }

        let init_bit_pos = bits.read_pos();
        let init_byte_pos = (init_bit_pos >> 3) as usize;
        if obu_size as u64 > (obus.len() as u64).wrapping_sub(init_byte_pos as u64) {
            return false;
        }

        if obu_type == 1 {
            // Sequence Header
            // (init_byte_pos is within the OBUs when no error was set)
            let Some(seq_hdr) = obus.get(init_byte_pos..init_byte_pos + obu_size as usize) else {
                return false;
            };
            let mut seq_hdr_bits = AvifBits::init(seq_hdr);
            match codec_type {
                AvifCodecType::Av1 => return parse_av1_sequence_header(&mut seq_hdr_bits, header),
                _ => return false,
            }
        }

        // Skip this OBU
        let Some(rest) = obus.get(obu_size as usize + init_byte_pos..) else {
            return false;
        };
        obus = rest;
    }
    false
}
