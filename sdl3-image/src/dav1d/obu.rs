// Rust translation of src/obu.c and src/obu.h from dav1d
// (https://code.videolan.org/videolan/dav1d, at the revision SDL_image's
// external/dav1d pins).
// Copyright © 2018-2021, VideoLAN and dav1d authors
// Copyright © 2018, Two Orioles, LLC
// SPDX-License-Identifier: BSD-2-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The OBU parser: sequence and frame headers, tile groups and metadata.

use std::sync::Arc;

use super::data::Dav1dData;
use super::decode::{dav1d_err, submit_frame, EINVAL, ENOENT, ERANGE};
use super::env::get_poc_diff;
use super::getbits::GetBits;
use super::headers::*;
use super::internal::*;
use super::intops::{iclip_u8, imax, imin, ulog2};
use super::log::dav1d_log;
use super::picture::*;
use super::tables::DEFAULT_WM_PARAMS;

fn check_trailing_bits(gb: &mut GetBits<'_>, strict_std_compliance: bool) -> Result<(), i32> {
    // trailing_one_bit + trailing_zero_bit
    if gb.get_bit() == 0 || gb.state != 0 || gb.error != 0 {
        return Err(dav1d_err(EINVAL));
    }

    if !strict_std_compliance {
        return Ok(());
    }

    let mut size = gb.ptr_end - gb.ptr;
    while size > 0 && gb.buf[gb.ptr + size - 1] == 0 {
        size -= 1;
    }

    if size != 0 {
        return Err(dav1d_err(EINVAL));
    }

    Ok(())
}

#[inline(never)]
fn parse_seq_hdr(
    hdr: &mut Dav1dSequenceHeader,
    gb: &mut GetBits<'_>,
    strict_std_compliance: bool,
) -> Result<(), i32> {
    let error = Err(dav1d_err(EINVAL));

    *hdr = Dav1dSequenceHeader::default();
    hdr.profile = gb.get_bits(3) as i32;
    if hdr.profile > 2 {
        return error;
    }

    hdr.still_picture = gb.get_bit() as i32;
    hdr.reduced_still_picture_header = gb.get_bit() as i32;
    if hdr.reduced_still_picture_header != 0 && hdr.still_picture == 0 {
        return error;
    }

    if hdr.reduced_still_picture_header != 0 {
        hdr.num_operating_points = 1;
        hdr.operating_points[0].major_level = gb.get_bits(3) as i32;
        hdr.operating_points[0].minor_level = gb.get_bits(2) as i32;
        hdr.operating_points[0].initial_display_delay = 10;
    } else {
        hdr.timing_info_present = gb.get_bit() as i32;
        if hdr.timing_info_present != 0 {
            hdr.num_units_in_tick = gb.get_bits(32) as i32;
            hdr.time_scale = gb.get_bits(32) as i32;
            if strict_std_compliance && (hdr.num_units_in_tick == 0 || hdr.time_scale == 0) {
                return error;
            }
            hdr.equal_picture_interval = gb.get_bit() as i32;
            if hdr.equal_picture_interval != 0 {
                let num_ticks_per_picture = gb.get_vlc();
                if num_ticks_per_picture == 0xFFFFFFFF {
                    return error;
                }
                hdr.num_ticks_per_picture = num_ticks_per_picture + 1;
            }

            hdr.decoder_model_info_present = gb.get_bit() as i32;
            if hdr.decoder_model_info_present != 0 {
                hdr.encoder_decoder_buffer_delay_length = gb.get_bits(5) as i32 + 1;
                hdr.num_units_in_decoding_tick = gb.get_bits(32) as i32;
                if strict_std_compliance && hdr.num_units_in_decoding_tick == 0 {
                    return error;
                }
                hdr.buffer_removal_delay_length = gb.get_bits(5) as i32 + 1;
                hdr.frame_presentation_delay_length = gb.get_bits(5) as i32 + 1;
            }
        }

        hdr.display_model_info_present = gb.get_bit() as i32;
        hdr.num_operating_points = gb.get_bits(5) as i32 + 1;
        for i in 0..hdr.num_operating_points as usize {
            let op = &mut hdr.operating_points[i];
            op.idc = gb.get_bits(12) as i32;
            if op.idc != 0 && (op.idc & 0xff == 0 || op.idc & 0xf00 == 0) {
                return error;
            }
            op.major_level = 2 + gb.get_bits(3) as i32;
            op.minor_level = gb.get_bits(2) as i32;
            if op.major_level > 3 {
                op.tier = gb.get_bit() as i32;
            }
            if hdr.decoder_model_info_present != 0 {
                op.decoder_model_param_present = gb.get_bit() as i32;
                if op.decoder_model_param_present != 0 {
                    let opi = &mut hdr.operating_parameter_info[i];
                    opi.decoder_buffer_delay =
                        gb.get_bits(hdr.encoder_decoder_buffer_delay_length) as i32;
                    opi.encoder_buffer_delay =
                        gb.get_bits(hdr.encoder_decoder_buffer_delay_length) as i32;
                    opi.low_delay_mode = gb.get_bit() as i32;
                }
            }
            let op = &mut hdr.operating_points[i];
            if hdr.display_model_info_present != 0 {
                op.display_model_param_present = gb.get_bit() as i32;
            }
            op.initial_display_delay = if op.display_model_param_present != 0 {
                gb.get_bits(4) as i32 + 1
            } else {
                10
            };
        }
    }

    hdr.width_n_bits = gb.get_bits(4) as i32 + 1;
    hdr.height_n_bits = gb.get_bits(4) as i32 + 1;
    hdr.max_width = gb.get_bits(hdr.width_n_bits) as i32 + 1;
    hdr.max_height = gb.get_bits(hdr.height_n_bits) as i32 + 1;
    if hdr.reduced_still_picture_header == 0 {
        hdr.frame_id_numbers_present = gb.get_bit() as i32;
        if hdr.frame_id_numbers_present != 0 {
            hdr.delta_frame_id_n_bits = gb.get_bits(4) as i32 + 2;
            hdr.frame_id_n_bits = gb.get_bits(3) as i32 + hdr.delta_frame_id_n_bits + 1;
        }
    }

    hdr.sb128 = gb.get_bit() as i32;
    hdr.filter_intra = gb.get_bit() as i32;
    hdr.intra_edge_filter = gb.get_bit() as i32;
    if hdr.reduced_still_picture_header != 0 {
        hdr.screen_content_tools = DAV1D_ADAPTIVE;
        hdr.force_integer_mv = DAV1D_ADAPTIVE;
    } else {
        hdr.inter_intra = gb.get_bit() as i32;
        hdr.masked_compound = gb.get_bit() as i32;
        hdr.warped_motion = gb.get_bit() as i32;
        hdr.dual_filter = gb.get_bit() as i32;
        hdr.order_hint = gb.get_bit() as i32;
        if hdr.order_hint != 0 {
            hdr.jnt_comp = gb.get_bit() as i32;
            hdr.ref_frame_mvs = gb.get_bit() as i32;
        }
        hdr.screen_content_tools = if gb.get_bit() != 0 {
            DAV1D_ADAPTIVE
        } else {
            gb.get_bit() as i32
        };
        hdr.force_integer_mv = if hdr.screen_content_tools != 0 {
            if gb.get_bit() != 0 {
                DAV1D_ADAPTIVE
            } else {
                gb.get_bit() as i32
            }
        } else {
            2
        };
        if hdr.order_hint != 0 {
            hdr.order_hint_n_bits = gb.get_bits(3) as i32 + 1;
        }
    }
    hdr.super_res = gb.get_bit() as i32;
    hdr.cdef = gb.get_bit() as i32;
    hdr.restoration = gb.get_bit() as i32;

    hdr.hbd = gb.get_bit() as i32;
    if hdr.profile == 2 && hdr.hbd != 0 {
        hdr.hbd += gb.get_bit() as i32;
    }
    if hdr.profile != 1 {
        hdr.monochrome = gb.get_bit() as i32;
    }
    hdr.color_description_present = gb.get_bit() as i32;
    if hdr.color_description_present != 0 {
        hdr.pri = gb.get_bits(8) as i32;
        hdr.trc = gb.get_bits(8) as i32;
        hdr.mtrx = gb.get_bits(8) as i32;
    } else {
        hdr.pri = DAV1D_COLOR_PRI_UNKNOWN;
        hdr.trc = DAV1D_TRC_UNKNOWN;
        hdr.mtrx = DAV1D_MC_UNKNOWN;
    }
    if hdr.monochrome != 0 {
        hdr.color_range = gb.get_bit() as i32;
        hdr.layout = DAV1D_PIXEL_LAYOUT_I400;
        hdr.ss_hor = 1;
        hdr.ss_ver = 1;
        hdr.chr = DAV1D_CHR_UNKNOWN;
    } else if hdr.pri == DAV1D_COLOR_PRI_BT709
        && hdr.trc == DAV1D_TRC_SRGB
        && hdr.mtrx == DAV1D_MC_IDENTITY
    {
        hdr.layout = DAV1D_PIXEL_LAYOUT_I444;
        hdr.color_range = 1;
        if hdr.profile != 1 && !(hdr.profile == 2 && hdr.hbd == 2) {
            return error;
        }
    } else {
        hdr.color_range = gb.get_bit() as i32;
        match hdr.profile {
            0 => {
                hdr.layout = DAV1D_PIXEL_LAYOUT_I420;
                hdr.ss_hor = 1;
                hdr.ss_ver = 1;
            }
            1 => {
                hdr.layout = DAV1D_PIXEL_LAYOUT_I444;
            }
            _ => {
                // 2
                if hdr.hbd == 2 {
                    hdr.ss_hor = gb.get_bit() as i32;
                    if hdr.ss_hor != 0 {
                        hdr.ss_ver = gb.get_bit() as i32;
                    }
                } else {
                    hdr.ss_hor = 1;
                }
                hdr.layout = if hdr.ss_hor != 0 {
                    if hdr.ss_ver != 0 {
                        DAV1D_PIXEL_LAYOUT_I420
                    } else {
                        DAV1D_PIXEL_LAYOUT_I422
                    }
                } else {
                    DAV1D_PIXEL_LAYOUT_I444
                };
            }
        }
        hdr.chr = if hdr.ss_hor & hdr.ss_ver != 0 {
            gb.get_bits(2) as i32
        } else {
            DAV1D_CHR_UNKNOWN
        };
    }
    if strict_std_compliance
        && hdr.mtrx == DAV1D_MC_IDENTITY
        && hdr.layout != DAV1D_PIXEL_LAYOUT_I444
    {
        return error;
    }
    if hdr.monochrome == 0 {
        hdr.separate_uv_delta_q = gb.get_bit() as i32;
    }

    hdr.film_grain_present = gb.get_bit() as i32;

    // We needn't bother flushing the OBU here: we'll check we didn't
    // overrun in the caller and will then discard gb, so there's no
    // point in setting its position properly.

    check_trailing_bits(gb, strict_std_compliance)
}

/// `dav1d_parse_sequence_header()`
pub(crate) fn dav1d_parse_sequence_header(
    out: &mut Dav1dSequenceHeader,
    data: &[u8],
) -> Result<(), i32> {
    if data.is_empty() {
        return Err(dav1d_err(EINVAL));
    }

    let mut gb = GetBits::new(data);
    let mut res = Err(dav1d_err(ENOENT));

    loop {
        gb.get_bit(); // obu_forbidden_bit
        let type_ = gb.get_bits(4);
        let has_extension = gb.get_bit() as i32;
        let has_length_field = gb.get_bit();
        gb.get_bits(1 + 8 * has_extension); // ignore

        let mut obu_end = gb.ptr_end;
        if has_length_field != 0 {
            let len = gb.get_uleb128() as usize;
            if len > obu_end - gb.ptr {
                return Err(dav1d_err(EINVAL));
            }
            obu_end = gb.ptr + len;
        }

        if type_ == DAV1D_OBU_SEQ_HDR {
            res = parse_seq_hdr(out, &mut gb, false);
            res?;
            if gb.ptr > obu_end {
                return Err(dav1d_err(EINVAL));
            }
            gb.bytealign();
        }

        if gb.error != 0 {
            return Err(dav1d_err(EINVAL));
        }
        debug_assert!(gb.state == 0 && gb.bits_left == 0);
        gb.ptr = obu_end;
        if gb.ptr >= gb.ptr_end {
            break;
        }
    }

    res
}

fn read_frame_size(
    c: &Dav1dContext,
    hdr: &mut Dav1dFrameHeader,
    gb: &mut GetBits<'_>,
    use_ref: bool,
) -> Result<(), ()> {
    let seqhdr = c.seq_hdr.as_deref().expect("sequence header");

    if use_ref {
        for i in 0..7 {
            if gb.get_bit() != 0 {
                let r = &c.refs[hdr.refidx[i] as usize].p;
                let Some(ref_hdr) = r.p.frame_hdr.as_deref() else {
                    return Err(());
                };
                hdr.width[1] = ref_hdr.width[1];
                hdr.height = ref_hdr.height;
                hdr.render_width = ref_hdr.render_width;
                hdr.render_height = ref_hdr.render_height;
                hdr.super_res.enabled = (seqhdr.super_res != 0 && gb.get_bit() != 0) as i32;
                if hdr.super_res.enabled != 0 {
                    let d = 9 + gb.get_bits(3) as i32;
                    hdr.super_res.width_scale_denominator = d;
                    hdr.width[0] =
                        imax((hdr.width[1] * 8 + (d >> 1)) / d, imin(16, hdr.width[1]));
                } else {
                    hdr.super_res.width_scale_denominator = 8;
                    hdr.width[0] = hdr.width[1];
                }
                return Ok(());
            }
        }
    }

    if hdr.frame_size_override != 0 {
        hdr.width[1] = gb.get_bits(seqhdr.width_n_bits) as i32 + 1;
        hdr.height = gb.get_bits(seqhdr.height_n_bits) as i32 + 1;
    } else {
        hdr.width[1] = seqhdr.max_width;
        hdr.height = seqhdr.max_height;
    }
    hdr.super_res.enabled = (seqhdr.super_res != 0 && gb.get_bit() != 0) as i32;
    if hdr.super_res.enabled != 0 {
        let d = 9 + gb.get_bits(3) as i32;
        hdr.super_res.width_scale_denominator = d;
        hdr.width[0] = imax((hdr.width[1] * 8 + (d >> 1)) / d, imin(16, hdr.width[1]));
    } else {
        hdr.super_res.width_scale_denominator = 8;
        hdr.width[0] = hdr.width[1];
    }
    hdr.have_render_size = gb.get_bit() as i32;
    if hdr.have_render_size != 0 {
        hdr.render_width = gb.get_bits(16) as i32 + 1;
        hdr.render_height = gb.get_bits(16) as i32 + 1;
    } else {
        hdr.render_width = hdr.width[1];
        hdr.render_height = hdr.height;
    }
    Ok(())
}

#[inline]
fn tile_log2(sz: i32, tgt: i32) -> i32 {
    let mut k = 0;
    while (sz << k) < tgt {
        k += 1;
    }
    k
}

const DEFAULT_MODE_REF_DELTAS: Dav1dLoopfilterModeRefDeltas = Dav1dLoopfilterModeRefDeltas {
    mode_delta: [0, 0],
    ref_delta: [1, 0, 0, 0, -1, 0, -1, -1],
};

/// The reference's frame header (`c->refs[i].p.p.frame_hdr`).
#[inline]
fn ref_hdr(c: &Dav1dContext, i: usize) -> Option<&Dav1dFrameHeader> {
    c.refs[i].p.p.frame_hdr.as_deref()
}

fn parse_frame_hdr(
    c: &Dav1dContext,
    hdr: &mut Dav1dFrameHeader,
    gb: &mut GetBits<'_>,
) -> Result<(), i32> {
    let seqhdr = c.seq_hdr.as_deref().expect("sequence header");
    let res = parse_frame_hdr_inner(c, seqhdr, hdr, gb);
    if res.is_err() {
        dav1d_log(c, "Error parsing frame header\n");
        return Err(dav1d_err(EINVAL));
    }
    Ok(())
}

fn parse_frame_hdr_inner(
    c: &Dav1dContext,
    seqhdr: &Dav1dSequenceHeader,
    hdr: &mut Dav1dFrameHeader,
    gb: &mut GetBits<'_>,
) -> Result<(), ()> {
    hdr.show_existing_frame =
        (seqhdr.reduced_still_picture_header == 0 && gb.get_bit() != 0) as i32;
    if hdr.show_existing_frame != 0 {
        hdr.existing_frame_idx = gb.get_bits(3) as i32;
        if seqhdr.decoder_model_info_present != 0 && seqhdr.equal_picture_interval == 0 {
            hdr.frame_presentation_delay =
                gb.get_bits(seqhdr.frame_presentation_delay_length) as i32;
        }
        if seqhdr.frame_id_numbers_present != 0 {
            hdr.frame_id = gb.get_bits(seqhdr.frame_id_n_bits) as i32;
            match ref_hdr(c, hdr.existing_frame_idx as usize) {
                Some(r) if r.frame_id == hdr.frame_id => {}
                _ => return Err(()),
            }
        }
        return Ok(());
    }

    hdr.frame_type = if seqhdr.reduced_still_picture_header != 0 {
        DAV1D_FRAME_TYPE_KEY
    } else {
        gb.get_bits(2) as i32
    };
    hdr.show_frame = (seqhdr.reduced_still_picture_header != 0 || gb.get_bit() != 0) as i32;
    if hdr.show_frame != 0 {
        if seqhdr.decoder_model_info_present != 0 && seqhdr.equal_picture_interval == 0 {
            hdr.frame_presentation_delay =
                gb.get_bits(seqhdr.frame_presentation_delay_length) as i32;
        }
        hdr.showable_frame = (hdr.frame_type != DAV1D_FRAME_TYPE_KEY) as i32;
    } else {
        hdr.showable_frame = gb.get_bit() as i32;
    }
    hdr.error_resilient_mode = ((hdr.frame_type == DAV1D_FRAME_TYPE_KEY && hdr.show_frame != 0)
        || hdr.frame_type == DAV1D_FRAME_TYPE_SWITCH
        || seqhdr.reduced_still_picture_header != 0
        || gb.get_bit() != 0) as i32;
    hdr.disable_cdf_update = gb.get_bit() as i32;
    hdr.allow_screen_content_tools = if seqhdr.screen_content_tools == DAV1D_ADAPTIVE {
        gb.get_bit() as i32
    } else {
        seqhdr.screen_content_tools
    };
    if hdr.allow_screen_content_tools != 0 {
        hdr.force_integer_mv = if seqhdr.force_integer_mv == DAV1D_ADAPTIVE {
            gb.get_bit() as i32
        } else {
            seqhdr.force_integer_mv
        };
    } else {
        hdr.force_integer_mv = 0;
    }

    if is_key_or_intra(hdr) {
        hdr.force_integer_mv = 1;
    }

    if seqhdr.frame_id_numbers_present != 0 {
        hdr.frame_id = gb.get_bits(seqhdr.frame_id_n_bits) as i32;
    }

    hdr.frame_size_override = if seqhdr.reduced_still_picture_header != 0 {
        0
    } else if hdr.frame_type == DAV1D_FRAME_TYPE_SWITCH {
        1
    } else {
        gb.get_bit() as i32
    };
    hdr.frame_offset = if seqhdr.order_hint != 0 {
        gb.get_bits(seqhdr.order_hint_n_bits) as i32
    } else {
        0
    };
    hdr.primary_ref_frame = if hdr.error_resilient_mode == 0 && is_inter_or_switch(hdr) {
        gb.get_bits(3) as i32
    } else {
        DAV1D_PRIMARY_REF_NONE
    };

    if seqhdr.decoder_model_info_present != 0 {
        hdr.buffer_removal_time_present = gb.get_bit() as i32;
        if hdr.buffer_removal_time_present != 0 {
            for i in 0..seqhdr.num_operating_points as usize {
                let seqop = &seqhdr.operating_points[i];
                if seqop.decoder_model_param_present != 0 {
                    let in_temporal_layer = (seqop.idc >> hdr.temporal_id) & 1;
                    let in_spatial_layer = (seqop.idc >> (hdr.spatial_id + 8)) & 1;
                    if seqop.idc == 0 || (in_temporal_layer != 0 && in_spatial_layer != 0) {
                        hdr.operating_points[i].buffer_removal_time =
                            gb.get_bits(seqhdr.buffer_removal_delay_length) as i32;
                    }
                }
            }
        }
    }

    if is_key_or_intra(hdr) {
        hdr.refresh_frame_flags = if hdr.frame_type == DAV1D_FRAME_TYPE_KEY && hdr.show_frame != 0
        {
            0xff
        } else {
            gb.get_bits(8) as i32
        };
        if hdr.refresh_frame_flags != 0xff
            && hdr.error_resilient_mode != 0
            && seqhdr.order_hint != 0
        {
            for _ in 0..8 {
                gb.get_bits(seqhdr.order_hint_n_bits);
            }
        }
        if c.strict_std_compliance
            && hdr.frame_type == DAV1D_FRAME_TYPE_INTRA
            && hdr.refresh_frame_flags == 0xff
        {
            return Err(());
        }
        read_frame_size(c, hdr, gb, false)?;
        hdr.allow_intrabc = (hdr.allow_screen_content_tools != 0
            && hdr.super_res.enabled == 0
            && gb.get_bit() != 0) as i32;
        hdr.use_ref_frame_mvs = 0;
    } else {
        hdr.allow_intrabc = 0;
        hdr.refresh_frame_flags = if hdr.frame_type == DAV1D_FRAME_TYPE_SWITCH {
            0xff
        } else {
            gb.get_bits(8) as i32
        };
        if hdr.error_resilient_mode != 0 && seqhdr.order_hint != 0 {
            for _ in 0..8 {
                gb.get_bits(seqhdr.order_hint_n_bits);
            }
        }
        hdr.frame_ref_short_signaling = (seqhdr.order_hint != 0 && gb.get_bit() != 0) as i32;
        if hdr.frame_ref_short_signaling != 0 {
            // FIXME: Nearly verbatim copy from section 7.8
            hdr.refidx[0] = gb.get_bits(3) as i32;
            hdr.refidx[1] = -1;
            hdr.refidx[2] = -1;
            hdr.refidx[3] = gb.get_bits(3) as i32;
            hdr.refidx[4] = -1;
            hdr.refidx[5] = -1;
            hdr.refidx[6] = -1;

            let mut shifted_frame_offset = [0i32; 8];
            let current_frame_offset = 1 << (seqhdr.order_hint_n_bits - 1);
            for i in 0..8 {
                let Some(r) = ref_hdr(c, i) else {
                    return Err(());
                };
                shifted_frame_offset[i] = current_frame_offset
                    + get_poc_diff(seqhdr.order_hint_n_bits, r.frame_offset, hdr.frame_offset);
            }

            let mut used_frame = [false; 8];
            used_frame[hdr.refidx[0] as usize] = true;
            used_frame[hdr.refidx[3] as usize] = true;

            let mut latest_frame_offset = -1;
            for i in 0..8 {
                let hint = shifted_frame_offset[i];
                if !used_frame[i] && hint >= current_frame_offset && hint >= latest_frame_offset {
                    hdr.refidx[6] = i as i32;
                    latest_frame_offset = hint;
                }
            }
            if latest_frame_offset != -1 {
                used_frame[hdr.refidx[6] as usize] = true;
            }

            let mut earliest_frame_offset = i32::MAX;
            for i in 0..8 {
                let hint = shifted_frame_offset[i];
                if !used_frame[i] && hint >= current_frame_offset && hint < earliest_frame_offset
                {
                    hdr.refidx[4] = i as i32;
                    earliest_frame_offset = hint;
                }
            }
            if earliest_frame_offset != i32::MAX {
                used_frame[hdr.refidx[4] as usize] = true;
            }

            earliest_frame_offset = i32::MAX;
            for i in 0..8 {
                let hint = shifted_frame_offset[i];
                if !used_frame[i] && hint >= current_frame_offset && hint < earliest_frame_offset
                {
                    hdr.refidx[5] = i as i32;
                    earliest_frame_offset = hint;
                }
            }
            if earliest_frame_offset != i32::MAX {
                used_frame[hdr.refidx[5] as usize] = true;
            }

            for i in 1..7 {
                if hdr.refidx[i] < 0 {
                    latest_frame_offset = -1;
                    for j in 0..8 {
                        let hint = shifted_frame_offset[j];
                        if !used_frame[j]
                            && hint < current_frame_offset
                            && hint >= latest_frame_offset
                        {
                            hdr.refidx[i] = j as i32;
                            latest_frame_offset = hint;
                        }
                    }
                    if latest_frame_offset != -1 {
                        used_frame[hdr.refidx[i] as usize] = true;
                    }
                }
            }

            earliest_frame_offset = i32::MAX;
            let mut r = -1;
            for i in 0..8 {
                let hint = shifted_frame_offset[i];
                if hint < earliest_frame_offset {
                    r = i as i32;
                    earliest_frame_offset = hint;
                }
            }
            for i in 0..7 {
                if hdr.refidx[i] < 0 {
                    hdr.refidx[i] = r;
                }
            }
        }
        for i in 0..7 {
            if hdr.frame_ref_short_signaling == 0 {
                hdr.refidx[i] = gb.get_bits(3) as i32;
            }
            if seqhdr.frame_id_numbers_present != 0 {
                let delta_ref_frame_id_minus_1 =
                    gb.get_bits(seqhdr.delta_frame_id_n_bits) as i32;
                let ref_frame_id = (hdr.frame_id + (1 << seqhdr.frame_id_n_bits)
                    - delta_ref_frame_id_minus_1
                    - 1)
                    & ((1 << seqhdr.frame_id_n_bits) - 1);
                match ref_hdr(c, hdr.refidx[i] as usize) {
                    Some(r) if r.frame_id == ref_frame_id => {}
                    _ => return Err(()),
                }
            }
        }
        let use_ref = hdr.error_resilient_mode == 0 && hdr.frame_size_override != 0;
        read_frame_size(c, hdr, gb, use_ref)?;
        hdr.hp = (hdr.force_integer_mv == 0 && gb.get_bit() != 0) as i32;
        hdr.subpel_filter_mode = if gb.get_bit() != 0 {
            DAV1D_FILTER_SWITCHABLE
        } else {
            gb.get_bits(2) as i32
        };
        hdr.switchable_motion_mode = gb.get_bit() as i32;
        hdr.use_ref_frame_mvs = (hdr.error_resilient_mode == 0
            && seqhdr.ref_frame_mvs != 0
            && seqhdr.order_hint != 0
            && is_inter_or_switch(hdr)
            && gb.get_bit() != 0) as i32;
    }

    hdr.refresh_context = (seqhdr.reduced_still_picture_header == 0
        && hdr.disable_cdf_update == 0
        && gb.get_bit() == 0) as i32;

    // tile data
    hdr.tiling.uniform = gb.get_bit() as i32;
    let sbsz_min1 = (64 << seqhdr.sb128) - 1;
    let sbsz_log2 = 6 + seqhdr.sb128;
    let sbw = (hdr.width[0] + sbsz_min1) >> sbsz_log2;
    let sbh = (hdr.height + sbsz_min1) >> sbsz_log2;
    let max_tile_width_sb = 4096 >> sbsz_log2;
    let max_tile_area_sb = 4096 * 2304 >> (2 * sbsz_log2);
    hdr.tiling.min_log2_cols = tile_log2(max_tile_width_sb, sbw);
    hdr.tiling.max_log2_cols = tile_log2(1, imin(sbw, DAV1D_MAX_TILE_COLS as i32));
    hdr.tiling.max_log2_rows = tile_log2(1, imin(sbh, DAV1D_MAX_TILE_ROWS as i32));
    let min_log2_tiles = imax(
        tile_log2(max_tile_area_sb, sbw * sbh),
        hdr.tiling.min_log2_cols,
    );
    if hdr.tiling.uniform != 0 {
        hdr.tiling.log2_cols = hdr.tiling.min_log2_cols;
        while hdr.tiling.log2_cols < hdr.tiling.max_log2_cols && gb.get_bit() != 0 {
            hdr.tiling.log2_cols += 1;
        }
        let tile_w = 1 + ((sbw - 1) >> hdr.tiling.log2_cols);
        hdr.tiling.cols = 0;
        let mut sbx = 0;
        while sbx < sbw {
            hdr.tiling.col_start_sb[hdr.tiling.cols as usize] = sbx as u16;
            sbx += tile_w;
            hdr.tiling.cols += 1;
        }
        hdr.tiling.min_log2_rows = imax(min_log2_tiles - hdr.tiling.log2_cols, 0);

        hdr.tiling.log2_rows = hdr.tiling.min_log2_rows;
        while hdr.tiling.log2_rows < hdr.tiling.max_log2_rows && gb.get_bit() != 0 {
            hdr.tiling.log2_rows += 1;
        }
        let tile_h = 1 + ((sbh - 1) >> hdr.tiling.log2_rows);
        hdr.tiling.rows = 0;
        let mut sby = 0;
        while sby < sbh {
            hdr.tiling.row_start_sb[hdr.tiling.rows as usize] = sby as u16;
            sby += tile_h;
            hdr.tiling.rows += 1;
        }
    } else {
        hdr.tiling.cols = 0;
        let mut widest_tile = 0;
        let mut max_tile_area_sb = sbw * sbh;
        let mut sbx = 0;
        while sbx < sbw && hdr.tiling.cols < DAV1D_MAX_TILE_COLS as i32 {
            let tile_width_sb = imin(sbw - sbx, max_tile_width_sb);
            let tile_w = if tile_width_sb > 1 {
                1 + gb.get_uniform(tile_width_sb as u32) as i32
            } else {
                1
            };
            hdr.tiling.col_start_sb[hdr.tiling.cols as usize] = sbx as u16;
            sbx += tile_w;
            widest_tile = imax(widest_tile, tile_w);
            hdr.tiling.cols += 1;
        }
        hdr.tiling.log2_cols = tile_log2(1, hdr.tiling.cols);
        if min_log2_tiles != 0 {
            max_tile_area_sb >>= min_log2_tiles + 1;
        }
        let max_tile_height_sb = imax(max_tile_area_sb / widest_tile, 1);

        hdr.tiling.rows = 0;
        let mut sby = 0;
        while sby < sbh && hdr.tiling.rows < DAV1D_MAX_TILE_ROWS as i32 {
            let tile_height_sb = imin(sbh - sby, max_tile_height_sb);
            let tile_h = if tile_height_sb > 1 {
                1 + gb.get_uniform(tile_height_sb as u32) as i32
            } else {
                1
            };
            hdr.tiling.row_start_sb[hdr.tiling.rows as usize] = sby as u16;
            sby += tile_h;
            hdr.tiling.rows += 1;
        }
        hdr.tiling.log2_rows = tile_log2(1, hdr.tiling.rows);
    }
    hdr.tiling.col_start_sb[hdr.tiling.cols as usize] = sbw as u16;
    hdr.tiling.row_start_sb[hdr.tiling.rows as usize] = sbh as u16;
    if hdr.tiling.log2_cols != 0 || hdr.tiling.log2_rows != 0 {
        hdr.tiling.update = gb.get_bits(hdr.tiling.log2_cols + hdr.tiling.log2_rows) as i32;
        if hdr.tiling.update >= hdr.tiling.cols * hdr.tiling.rows {
            return Err(());
        }
        hdr.tiling.n_bytes = gb.get_bits(2) + 1;
    } else {
        hdr.tiling.n_bytes = 0;
        hdr.tiling.update = 0;
    }

    // quant data
    hdr.quant.yac = gb.get_bits(8) as i32;
    hdr.quant.ydc_delta = if gb.get_bit() != 0 {
        gb.get_sbits(7)
    } else {
        0
    };
    if seqhdr.monochrome == 0 {
        // If the sequence header says that delta_q might be different
        // for U, V, we must check whether it actually is for this
        // frame.
        let diff_uv_delta = if seqhdr.separate_uv_delta_q != 0 {
            gb.get_bit()
        } else {
            0
        };
        hdr.quant.udc_delta = if gb.get_bit() != 0 {
            gb.get_sbits(7)
        } else {
            0
        };
        hdr.quant.uac_delta = if gb.get_bit() != 0 {
            gb.get_sbits(7)
        } else {
            0
        };
        if diff_uv_delta != 0 {
            hdr.quant.vdc_delta = if gb.get_bit() != 0 {
                gb.get_sbits(7)
            } else {
                0
            };
            hdr.quant.vac_delta = if gb.get_bit() != 0 {
                gb.get_sbits(7)
            } else {
                0
            };
        } else {
            hdr.quant.vdc_delta = hdr.quant.udc_delta;
            hdr.quant.vac_delta = hdr.quant.uac_delta;
        }
    }
    hdr.quant.qm = gb.get_bit() as i32;
    if hdr.quant.qm != 0 {
        hdr.quant.qm_y = gb.get_bits(4) as i32;
        hdr.quant.qm_u = gb.get_bits(4) as i32;
        hdr.quant.qm_v = if seqhdr.separate_uv_delta_q != 0 {
            gb.get_bits(4) as i32
        } else {
            hdr.quant.qm_u
        };
    }

    // segmentation data
    hdr.segmentation.enabled = gb.get_bit() as i32;
    if hdr.segmentation.enabled != 0 {
        if hdr.primary_ref_frame == DAV1D_PRIMARY_REF_NONE {
            hdr.segmentation.update_map = 1;
            hdr.segmentation.temporal = 0;
            hdr.segmentation.update_data = 1;
        } else {
            hdr.segmentation.update_map = gb.get_bit() as i32;
            hdr.segmentation.temporal = if hdr.segmentation.update_map != 0 {
                gb.get_bit() as i32
            } else {
                0
            };
            hdr.segmentation.update_data = gb.get_bit() as i32;
        }

        if hdr.segmentation.update_data != 0 {
            hdr.segmentation.seg_data.preskip = 0;
            hdr.segmentation.seg_data.last_active_segid = -1;
            for i in 0..DAV1D_MAX_SEGMENTS {
                let sd = &mut hdr.segmentation.seg_data;
                let seg = &mut sd.d[i];
                if gb.get_bit() != 0 {
                    seg.delta_q = gb.get_sbits(9);
                    sd.last_active_segid = i as i32;
                } else {
                    seg.delta_q = 0;
                }
                if gb.get_bit() != 0 {
                    seg.delta_lf_y_v = gb.get_sbits(7);
                    sd.last_active_segid = i as i32;
                } else {
                    seg.delta_lf_y_v = 0;
                }
                if gb.get_bit() != 0 {
                    seg.delta_lf_y_h = gb.get_sbits(7);
                    sd.last_active_segid = i as i32;
                } else {
                    seg.delta_lf_y_h = 0;
                }
                if gb.get_bit() != 0 {
                    seg.delta_lf_u = gb.get_sbits(7);
                    sd.last_active_segid = i as i32;
                } else {
                    seg.delta_lf_u = 0;
                }
                if gb.get_bit() != 0 {
                    seg.delta_lf_v = gb.get_sbits(7);
                    sd.last_active_segid = i as i32;
                } else {
                    seg.delta_lf_v = 0;
                }
                if gb.get_bit() != 0 {
                    seg.ref_ = gb.get_bits(3) as i32;
                    sd.last_active_segid = i as i32;
                    sd.preskip = 1;
                } else {
                    seg.ref_ = -1;
                }
                seg.skip = gb.get_bit() as i32;
                if seg.skip != 0 {
                    sd.last_active_segid = i as i32;
                    sd.preskip = 1;
                }
                let seg = &mut sd.d[i];
                seg.globalmv = gb.get_bit() as i32;
                if seg.globalmv != 0 {
                    sd.last_active_segid = i as i32;
                    sd.preskip = 1;
                }
            }
        } else {
            // segmentation.update_data was false so we should copy
            // segmentation data from the reference frame.
            debug_assert!(hdr.primary_ref_frame != DAV1D_PRIMARY_REF_NONE);
            let pri_ref = hdr.refidx[hdr.primary_ref_frame as usize] as usize;
            let Some(r) = ref_hdr(c, pri_ref) else {
                return Err(());
            };
            hdr.segmentation.seg_data = r.segmentation.seg_data;
        }
    } else {
        hdr.segmentation.seg_data = Dav1dSegmentationDataSet::default();
        for i in 0..DAV1D_MAX_SEGMENTS {
            hdr.segmentation.seg_data.d[i].ref_ = -1;
        }
    }

    // delta q
    hdr.delta.q.present = if hdr.quant.yac != 0 {
        gb.get_bit() as i32
    } else {
        0
    };
    hdr.delta.q.res_log2 = if hdr.delta.q.present != 0 {
        gb.get_bits(2) as i32
    } else {
        0
    };
    hdr.delta.lf.present =
        (hdr.delta.q.present != 0 && hdr.allow_intrabc == 0 && gb.get_bit() != 0) as i32;
    hdr.delta.lf.res_log2 = if hdr.delta.lf.present != 0 {
        gb.get_bits(2) as i32
    } else {
        0
    };
    hdr.delta.lf.multi = if hdr.delta.lf.present != 0 {
        gb.get_bit() as i32
    } else {
        0
    };

    // derive lossless flags
    let delta_lossless = hdr.quant.ydc_delta == 0
        && hdr.quant.udc_delta == 0
        && hdr.quant.uac_delta == 0
        && hdr.quant.vdc_delta == 0
        && hdr.quant.vac_delta == 0;
    hdr.all_lossless = 1;
    for i in 0..DAV1D_MAX_SEGMENTS {
        hdr.segmentation.qidx[i] = if hdr.segmentation.enabled != 0 {
            iclip_u8(hdr.quant.yac + hdr.segmentation.seg_data.d[i].delta_q)
        } else {
            hdr.quant.yac
        };
        hdr.segmentation.lossless[i] = (hdr.segmentation.qidx[i] == 0 && delta_lossless) as i32;
        hdr.all_lossless &= hdr.segmentation.lossless[i];
    }

    // loopfilter
    if hdr.all_lossless != 0 || hdr.allow_intrabc != 0 {
        hdr.loopfilter.level_y = [0, 0];
        hdr.loopfilter.level_u = 0;
        hdr.loopfilter.level_v = 0;
        hdr.loopfilter.sharpness = 0;
        hdr.loopfilter.mode_ref_delta_enabled = 1;
        hdr.loopfilter.mode_ref_delta_update = 1;
        hdr.loopfilter.mode_ref_deltas = DEFAULT_MODE_REF_DELTAS;
    } else {
        hdr.loopfilter.level_y[0] = gb.get_bits(6) as i32;
        hdr.loopfilter.level_y[1] = gb.get_bits(6) as i32;
        if seqhdr.monochrome == 0
            && (hdr.loopfilter.level_y[0] != 0 || hdr.loopfilter.level_y[1] != 0)
        {
            hdr.loopfilter.level_u = gb.get_bits(6) as i32;
            hdr.loopfilter.level_v = gb.get_bits(6) as i32;
        }
        hdr.loopfilter.sharpness = gb.get_bits(3) as i32;

        if hdr.primary_ref_frame == DAV1D_PRIMARY_REF_NONE {
            hdr.loopfilter.mode_ref_deltas = DEFAULT_MODE_REF_DELTAS;
        } else {
            let r = hdr.refidx[hdr.primary_ref_frame as usize] as usize;
            let Some(rh) = ref_hdr(c, r) else {
                return Err(());
            };
            hdr.loopfilter.mode_ref_deltas = rh.loopfilter.mode_ref_deltas;
        }
        hdr.loopfilter.mode_ref_delta_enabled = gb.get_bit() as i32;
        if hdr.loopfilter.mode_ref_delta_enabled != 0 {
            hdr.loopfilter.mode_ref_delta_update = gb.get_bit() as i32;
            if hdr.loopfilter.mode_ref_delta_update != 0 {
                for i in 0..8 {
                    if gb.get_bit() != 0 {
                        hdr.loopfilter.mode_ref_deltas.ref_delta[i] = gb.get_sbits(7);
                    }
                }
                for i in 0..2 {
                    if gb.get_bit() != 0 {
                        hdr.loopfilter.mode_ref_deltas.mode_delta[i] = gb.get_sbits(7);
                    }
                }
            }
        }
    }

    // cdef
    if hdr.all_lossless == 0 && seqhdr.cdef != 0 && hdr.allow_intrabc == 0 {
        hdr.cdef.damping = gb.get_bits(2) as i32 + 3;
        hdr.cdef.n_bits = gb.get_bits(2) as i32;
        for i in 0..(1 << hdr.cdef.n_bits) as usize {
            hdr.cdef.y_strength[i] = gb.get_bits(6) as i32;
            if seqhdr.monochrome == 0 {
                hdr.cdef.uv_strength[i] = gb.get_bits(6) as i32;
            }
        }
    } else {
        hdr.cdef.n_bits = 0;
        hdr.cdef.y_strength[0] = 0;
        hdr.cdef.uv_strength[0] = 0;
    }

    // restoration
    if (hdr.all_lossless == 0 || hdr.super_res.enabled != 0)
        && seqhdr.restoration != 0
        && hdr.allow_intrabc == 0
    {
        hdr.restoration.type_[0] = gb.get_bits(2) as u8;
        if seqhdr.monochrome == 0 {
            hdr.restoration.type_[1] = gb.get_bits(2) as u8;
            hdr.restoration.type_[2] = gb.get_bits(2) as u8;
        } else {
            hdr.restoration.type_[1] = DAV1D_RESTORATION_NONE;
            hdr.restoration.type_[2] = DAV1D_RESTORATION_NONE;
        }

        if hdr.restoration.type_[0] != 0
            || hdr.restoration.type_[1] != 0
            || hdr.restoration.type_[2] != 0
        {
            // Log2 of the restoration unit size.
            hdr.restoration.unit_size[0] = 6 + seqhdr.sb128;
            if gb.get_bit() != 0 {
                hdr.restoration.unit_size[0] += 1;
                if seqhdr.sb128 == 0 {
                    hdr.restoration.unit_size[0] += gb.get_bit() as i32;
                }
            }
            hdr.restoration.unit_size[1] = hdr.restoration.unit_size[0];
            if (hdr.restoration.type_[1] != 0 || hdr.restoration.type_[2] != 0)
                && seqhdr.ss_hor == 1
                && seqhdr.ss_ver == 1
            {
                hdr.restoration.unit_size[1] -= gb.get_bit() as i32;
            }
        } else {
            hdr.restoration.unit_size[0] = 8;
        }
    } else {
        hdr.restoration.type_ = [DAV1D_RESTORATION_NONE; 3];
    }

    hdr.txfm_mode = if hdr.all_lossless != 0 {
        DAV1D_TX_4X4_ONLY
    } else if gb.get_bit() != 0 {
        DAV1D_TX_SWITCHABLE
    } else {
        DAV1D_TX_LARGEST
    };
    hdr.switchable_comp_refs = if is_inter_or_switch(hdr) {
        gb.get_bit() as i32
    } else {
        0
    };
    hdr.skip_mode_allowed = 0;
    if hdr.switchable_comp_refs != 0 && is_inter_or_switch(hdr) && seqhdr.order_hint != 0 {
        let poc = hdr.frame_offset as u32;
        let mut off_before = 0xFFFFFFFFu32;
        let mut off_after = -1i32;
        let mut off_before_idx = 0;
        let mut off_after_idx = 0;
        for i in 0..7 {
            let Some(r) = ref_hdr(c, hdr.refidx[i] as usize) else {
                return Err(());
            };
            let refpoc = r.frame_offset as u32;

            let diff = get_poc_diff(seqhdr.order_hint_n_bits, refpoc as i32, poc as i32);
            if diff > 0 {
                if off_after == -1
                    || get_poc_diff(seqhdr.order_hint_n_bits, off_after, refpoc as i32) > 0
                {
                    off_after = refpoc as i32;
                    off_after_idx = i as i32;
                }
            } else if diff < 0
                && (off_before == 0xFFFFFFFF
                    || get_poc_diff(seqhdr.order_hint_n_bits, refpoc as i32, off_before as i32)
                        > 0)
            {
                off_before = refpoc;
                off_before_idx = i as i32;
            }
        }

        if off_before != 0xFFFFFFFF && off_after != -1 {
            hdr.skip_mode_refs[0] = imin(off_before_idx, off_after_idx);
            hdr.skip_mode_refs[1] = imax(off_before_idx, off_after_idx);
            hdr.skip_mode_allowed = 1;
        } else if off_before != 0xFFFFFFFF {
            let mut off_before2 = 0xFFFFFFFFu32;
            let mut off_before2_idx = 0;
            for i in 0..7 {
                let Some(r) = ref_hdr(c, hdr.refidx[i] as usize) else {
                    return Err(());
                };
                let refpoc = r.frame_offset as u32;
                if get_poc_diff(seqhdr.order_hint_n_bits, refpoc as i32, off_before as i32) < 0
                    && (off_before2 == 0xFFFFFFFF
                        || get_poc_diff(
                            seqhdr.order_hint_n_bits,
                            refpoc as i32,
                            off_before2 as i32,
                        ) > 0)
                {
                    off_before2 = refpoc;
                    off_before2_idx = i as i32;
                }
            }

            if off_before2 != 0xFFFFFFFF {
                hdr.skip_mode_refs[0] = imin(off_before_idx, off_before2_idx);
                hdr.skip_mode_refs[1] = imax(off_before_idx, off_before2_idx);
                hdr.skip_mode_allowed = 1;
            }
        }
    }
    hdr.skip_mode_enabled = if hdr.skip_mode_allowed != 0 {
        gb.get_bit() as i32
    } else {
        0
    };
    hdr.warp_motion = (hdr.error_resilient_mode == 0
        && is_inter_or_switch(hdr)
        && seqhdr.warped_motion != 0
        && gb.get_bit() != 0) as i32;
    hdr.reduced_txtp_set = gb.get_bit() as i32;

    for i in 0..7 {
        hdr.gmv[i] = DEFAULT_WM_PARAMS;
    }

    if is_inter_or_switch(hdr) {
        for i in 0..7 {
            hdr.gmv[i].type_ = if gb.get_bit() == 0 {
                DAV1D_WM_TYPE_IDENTITY
            } else if gb.get_bit() != 0 {
                DAV1D_WM_TYPE_ROT_ZOOM
            } else if gb.get_bit() != 0 {
                DAV1D_WM_TYPE_TRANSLATION
            } else {
                DAV1D_WM_TYPE_AFFINE
            };

            if hdr.gmv[i].type_ == DAV1D_WM_TYPE_IDENTITY {
                continue;
            }

            let ref_gmv = if hdr.primary_ref_frame == DAV1D_PRIMARY_REF_NONE {
                DEFAULT_WM_PARAMS
            } else {
                let pri_ref = hdr.refidx[hdr.primary_ref_frame as usize] as usize;
                let Some(r) = ref_hdr(c, pri_ref) else {
                    return Err(());
                };
                r.gmv[i]
            };
            let ref_mat = &ref_gmv.matrix;
            let bits;
            let shift;
            let hp = hdr.hp;
            let gtype = hdr.gmv[i].type_;
            let mat = &mut hdr.gmv[i].matrix;

            if gtype >= DAV1D_WM_TYPE_ROT_ZOOM {
                mat[2] = (1 << 16) + 2 * gb.get_bits_subexp((ref_mat[2] - (1 << 16)) >> 1, 12);
                mat[3] = 2 * gb.get_bits_subexp(ref_mat[3] >> 1, 12);

                bits = 12;
                shift = 10;
            } else {
                bits = 9 - (hp == 0) as u32;
                shift = 13 + (hp == 0) as i32;
            }

            if gtype == DAV1D_WM_TYPE_AFFINE {
                mat[4] = 2 * gb.get_bits_subexp(ref_mat[4] >> 1, 12);
                mat[5] = (1 << 16) + 2 * gb.get_bits_subexp((ref_mat[5] - (1 << 16)) >> 1, 12);
            } else {
                mat[4] = -mat[3];
                mat[5] = mat[2];
            }

            mat[0] = gb.get_bits_subexp(ref_mat[0] >> shift, bits) * (1 << shift);
            mat[1] = gb.get_bits_subexp(ref_mat[1] >> shift, bits) * (1 << shift);
        }
    }

    hdr.film_grain.present = (seqhdr.film_grain_present != 0
        && (hdr.show_frame != 0 || hdr.showable_frame != 0)
        && gb.get_bit() != 0) as i32;
    if hdr.film_grain.present != 0 {
        let seed = gb.get_bits(16);
        hdr.film_grain.update =
            (hdr.frame_type != DAV1D_FRAME_TYPE_INTER || gb.get_bit() != 0) as i32;
        if hdr.film_grain.update == 0 {
            let refidx = gb.get_bits(3) as i32;
            let mut i = 0;
            while i < 7 {
                if hdr.refidx[i] == refidx {
                    break;
                }
                i += 1;
            }
            let r = ref_hdr(c, refidx as usize);
            if i == 7 || r.is_none() {
                return Err(());
            }
            hdr.film_grain.data = r.expect("reference").film_grain.data;
            hdr.film_grain.data.seed = seed;
        } else {
            let fgd = &mut hdr.film_grain.data;
            fgd.seed = seed;

            fgd.num_y_points = gb.get_bits(4) as i32;
            if fgd.num_y_points > 14 {
                return Err(());
            }
            for i in 0..fgd.num_y_points as usize {
                fgd.y_points[i][0] = gb.get_bits(8) as u8;
                if i != 0 && fgd.y_points[i - 1][0] >= fgd.y_points[i][0] {
                    return Err(());
                }
                fgd.y_points[i][1] = gb.get_bits(8) as u8;
            }

            fgd.chroma_scaling_from_luma = (seqhdr.monochrome == 0 && gb.get_bit() != 0) as i32;
            if seqhdr.monochrome != 0
                || fgd.chroma_scaling_from_luma != 0
                || (seqhdr.ss_ver == 1 && seqhdr.ss_hor == 1 && fgd.num_y_points == 0)
            {
                fgd.num_uv_points = [0, 0];
            } else {
                for pl in 0..2 {
                    fgd.num_uv_points[pl] = gb.get_bits(4) as i32;
                    if fgd.num_uv_points[pl] > 10 {
                        return Err(());
                    }
                    for i in 0..fgd.num_uv_points[pl] as usize {
                        fgd.uv_points[pl][i][0] = gb.get_bits(8) as u8;
                        if i != 0 && fgd.uv_points[pl][i - 1][0] >= fgd.uv_points[pl][i][0] {
                            return Err(());
                        }
                        fgd.uv_points[pl][i][1] = gb.get_bits(8) as u8;
                    }
                }
            }

            if seqhdr.ss_hor == 1
                && seqhdr.ss_ver == 1
                && (fgd.num_uv_points[0] != 0) != (fgd.num_uv_points[1] != 0)
            {
                return Err(());
            }

            fgd.scaling_shift = gb.get_bits(2) as i32 + 8;
            fgd.ar_coeff_lag = gb.get_bits(2) as i32;
            let num_y_pos = (2 * fgd.ar_coeff_lag * (fgd.ar_coeff_lag + 1)) as usize;
            if fgd.num_y_points != 0 {
                for i in 0..num_y_pos {
                    fgd.ar_coeffs_y[i] = (gb.get_bits(8) as i32 - 128) as i8;
                }
            }
            for pl in 0..2 {
                if fgd.num_uv_points[pl] != 0 || fgd.chroma_scaling_from_luma != 0 {
                    let num_uv_pos = num_y_pos + (fgd.num_y_points != 0) as usize;
                    for i in 0..num_uv_pos {
                        fgd.ar_coeffs_uv[pl][i] = (gb.get_bits(8) as i32 - 128) as i8;
                    }
                    if fgd.num_y_points == 0 {
                        fgd.ar_coeffs_uv[pl][num_uv_pos] = 0;
                    }
                }
            }
            fgd.ar_coeff_shift = gb.get_bits(2) as u64 + 6;
            fgd.grain_scale_shift = gb.get_bits(2) as i32;
            for pl in 0..2 {
                if fgd.num_uv_points[pl] != 0 {
                    fgd.uv_mult[pl] = gb.get_bits(8) as i32 - 128;
                    fgd.uv_luma_mult[pl] = gb.get_bits(8) as i32 - 128;
                    fgd.uv_offset[pl] = gb.get_bits(9) as i32 - 256;
                }
            }
            fgd.overlap_flag = gb.get_bit() as i32;
            fgd.clip_to_restricted_range = gb.get_bit() as i32;
        }
    } else {
        hdr.film_grain.data = Dav1dFilmGrainData::default();
    }

    Ok(())
}

fn parse_tile_hdr(hdr: &Dav1dFrameHeader, tg: &mut Dav1dTileGroup, gb: &mut GetBits<'_>) {
    let n_tiles = hdr.tiling.cols * hdr.tiling.rows;
    let have_tile_pos = if n_tiles > 1 { gb.get_bit() } else { 0 };

    if have_tile_pos != 0 {
        let n_bits = hdr.tiling.log2_cols + hdr.tiling.log2_rows;
        tg.start = gb.get_bits(n_bits) as i32;
        tg.end = gb.get_bits(n_bits) as i32;
    } else {
        tg.start = 0;
        tg.end = n_tiles - 1;
    }
}

// enum ObuMetaType
const OBU_META_HDR_CLL: u32 = 1;
const OBU_META_HDR_MDCV: u32 = 2;
const OBU_META_SCALABILITY: u32 = 3;
const OBU_META_ITUT_T35: u32 = 4;
const OBU_META_TIMECODE: u32 = 5;

/// The outcome of `dav1d_parse_obus()` before its common tail.
enum ObuEnd {
    Done,
    Skip,
    Error,
}

/// `dav1d_parse_obus()`: returns the number of bytes of `in_` consumed.
pub(crate) fn dav1d_parse_obus(c: &mut Dav1dContext, in_: &Dav1dData) -> Result<usize, i32> {
    let data = in_.data();
    let mut gb = GetBits::new(data);

    let end = parse_obus_body(c, in_, &mut gb)?;
    match end {
        ObuEnd::Done => Ok(gb.ptr_end),
        ObuEnd::Skip => {
            // update refs with only the headers in case we skip the frame
            let frame_hdr = c.frame_hdr.clone().expect("frame header");
            for i in 0..8 {
                if frame_hdr.refresh_frame_flags & (1 << i) != 0 {
                    c.refs[i].p.unref();
                    c.refs[i].p.p.frame_hdr = Some(frame_hdr.clone());
                    c.refs[i].p.p.seq_hdr = c.seq_hdr.clone();
                }
            }

            c.frame_hdr = None;
            c.n_tiles = 0;

            Ok(gb.ptr_end)
        }
        ObuEnd::Error => {
            c.cached_error_props = in_.m.clone();
            dav1d_log(
                c,
                if gb.error != 0 {
                    "Overrun in OBU bit buffer\n"
                } else {
                    "Error parsing OBU data\n"
                },
            );
            Err(dav1d_err(EINVAL))
        }
    }
}

fn parse_obus_body(
    c: &mut Dav1dContext,
    in_: &Dav1dData,
    gb: &mut GetBits<'_>,
) -> Result<ObuEnd, i32> {
    // obu header
    gb.get_bit(); // obu_forbidden_bit
    let type_ = gb.get_bits(4);
    let has_extension = gb.get_bit();
    let has_length_field = gb.get_bit();
    gb.get_bit(); // reserved

    let mut temporal_id = 0;
    let mut spatial_id = 0;
    if has_extension != 0 {
        temporal_id = gb.get_bits(3) as i32;
        spatial_id = gb.get_bits(2) as i32;
        gb.get_bits(3); // reserved
    }

    if has_length_field != 0 {
        let len = gb.get_uleb128() as usize;
        if len > gb.ptr_end - gb.ptr {
            return Ok(ObuEnd::Error);
        }
        gb.ptr_end = gb.ptr + len;
    }
    if gb.error != 0 {
        return Ok(ObuEnd::Error);
    }

    // We must have read a whole number of bytes at this point (1 byte
    // for the header and whole bytes at a time when reading the
    // leb128 length field).
    debug_assert!(gb.bits_left == 0);

    // skip obu not belonging to the selected temporal/spatial layer
    if type_ != DAV1D_OBU_SEQ_HDR
        && type_ != DAV1D_OBU_TD
        && has_extension != 0
        && c.operating_point_idc != 0
    {
        let in_temporal_layer = (c.operating_point_idc >> temporal_id) & 1;
        let in_spatial_layer = (c.operating_point_idc >> (spatial_id + 8)) & 1;
        if in_temporal_layer == 0 || in_spatial_layer == 0 {
            return Ok(ObuEnd::Done);
        }
    }

    let mut parse_frame = false;
    let mut parse_tiles = false;
    match type_ {
        DAV1D_OBU_SEQ_HDR => {
            let mut seq_hdr = Dav1dSequenceHeader::default();
            if parse_seq_hdr(&mut seq_hdr, gb, c.strict_std_compliance).is_err() {
                dav1d_log(c, "Error parsing sequence header\n");
                return Ok(ObuEnd::Error);
            }
            if gb.error != 0 {
                return Ok(ObuEnd::Error);
            }

            let op_idx = if c.operating_point < seq_hdr.num_operating_points {
                c.operating_point
            } else {
                0
            };
            c.operating_point_idc = seq_hdr.operating_points[op_idx as usize].idc as u32;
            let spatial_mask = c.operating_point_idc >> 8;
            c.max_spatial_id = if spatial_mask != 0 {
                ulog2(spatial_mask)
            } else {
                0
            };

            // If we have read a sequence header which is different from
            // the old one, this is a new video sequence and can't use any
            // previous state. Free that state.

            match &c.seq_hdr {
                None => {
                    c.frame_hdr = None;
                    c.frame_flags |= PICTURE_FLAG_NEW_SEQUENCE;
                }
                // see 7.5, operating_parameter_info is allowed to change in
                // sequence headers of a single sequence
                Some(old) if seq_hdr.differs_before_op_params_info(old) => {
                    c.frame_hdr = None;
                    c.mastering_display = None;
                    c.content_light = None;
                    for i in 0..8 {
                        c.refs[i].p.unref();
                        c.refs[i].segmap = None;
                        c.refs[i].refmvs = None;
                        c.cdf[i] = Default::default();
                    }
                    c.frame_flags |= PICTURE_FLAG_NEW_SEQUENCE;
                }
                // If operating_parameter_info changed, signal it
                Some(old) if seq_hdr.operating_parameter_info != old.operating_parameter_info => {
                    c.frame_flags |= PICTURE_FLAG_NEW_OP_PARAMS_INFO;
                }
                _ => {}
            }
            c.seq_hdr = Some(Arc::new(seq_hdr));
        }
        DAV1D_OBU_REDUNDANT_FRAME_HDR | DAV1D_OBU_FRAME | DAV1D_OBU_FRAME_HDR => {
            if !(type_ == DAV1D_OBU_REDUNDANT_FRAME_HDR && c.frame_hdr.is_some()) {
                parse_frame = true;
            }
        }
        DAV1D_OBU_TILE_GRP => {
            parse_tiles = true;
        }
        DAV1D_OBU_METADATA => {
            // obu metadta type field
            let meta_type = gb.get_uleb128();
            if gb.error != 0 {
                return Ok(ObuEnd::Error);
            }

            match meta_type {
                OBU_META_HDR_CLL => {
                    let content_light = Dav1dContentLightLevel {
                        max_content_light_level: gb.get_bits(16) as i32,
                        max_frame_average_light_level: gb.get_bits(16) as i32,
                    };

                    if check_trailing_bits(gb, c.strict_std_compliance).is_err() {
                        return Ok(ObuEnd::Error);
                    }

                    c.content_light = Some(Arc::new(content_light));
                }
                OBU_META_HDR_MDCV => {
                    let mut mastering_display = Dav1dMasteringDisplay::default();

                    for i in 0..3 {
                        mastering_display.primaries[i][0] = gb.get_bits(16) as u16;
                        mastering_display.primaries[i][1] = gb.get_bits(16) as u16;
                    }
                    mastering_display.white_point[0] = gb.get_bits(16) as u16;
                    mastering_display.white_point[1] = gb.get_bits(16) as u16;
                    mastering_display.max_luminance = gb.get_bits(32);
                    mastering_display.min_luminance = gb.get_bits(32);
                    if check_trailing_bits(gb, c.strict_std_compliance).is_err() {
                        return Ok(ObuEnd::Error);
                    }

                    c.mastering_display = Some(Arc::new(mastering_display));
                }
                OBU_META_ITUT_T35 => {
                    let mut payload_size = gb.ptr_end as isize - gb.ptr as isize;
                    // Don't take into account all the trailing bits for payload_size
                    while payload_size > 0 && gb.buf[gb.ptr + payload_size as usize - 1] == 0 {
                        payload_size -= 1; // trailing_zero_bit x 8
                    }
                    payload_size -= 1; // trailing_one_bit + trailing_zero_bit x 7

                    let mut country_code_extension_byte = 0;
                    let country_code = gb.get_bits(8) as u8;
                    payload_size -= 1;
                    if country_code == 0xFF {
                        country_code_extension_byte = gb.get_bits(8) as u8;
                        payload_size -= 1;
                    }

                    if payload_size <= 0 || gb.buf[gb.ptr + payload_size as usize] != 0x80 {
                        dav1d_log(c, "Malformed ITU-T T.35 metadata message format\n");
                    } else {
                        // We know that we've read a whole number of bytes and that the
                        // payload is within the OBU boundaries, so just copy it
                        debug_assert!(gb.bits_left == 0);
                        let payload = gb.buf[gb.ptr..gb.ptr + payload_size as usize].to_vec();
                        let mut v: Vec<Dav1dITUTT35> =
                            c.itut_t35.as_deref().cloned().unwrap_or_default();
                        v.push(Dav1dITUTT35 {
                            country_code,
                            country_code_extension_byte,
                            payload,
                        });
                        c.itut_t35 = Some(Arc::new(v));
                    }
                }
                OBU_META_SCALABILITY | OBU_META_TIMECODE => {
                    // ignore metadata OBUs we don't care about
                }
                _ => {
                    // print a warning but don't fail for unknown types
                    dav1d_log(c, &format!("Unknown Metadata OBU type {meta_type}\n"));
                }
            }
        }
        DAV1D_OBU_TD => {
            c.frame_flags |= PICTURE_FLAG_NEW_TEMPORAL_UNIT;
        }
        DAV1D_OBU_PADDING => {
            // ignore OBUs we don't care about
        }
        _ => {
            // print a warning but don't fail for unknown types
            dav1d_log(
                c,
                &format!("Unknown OBU type {} of size {}\n", type_, gb.ptr_end - gb.ptr),
            );
        }
    }

    if parse_frame {
        if c.seq_hdr.is_none() {
            return Ok(ObuEnd::Error);
        }
        let mut hdr = Dav1dFrameHeader {
            temporal_id,
            spatial_id,
            ..Default::default()
        };
        c.frame_hdr = None;
        if parse_frame_hdr(c, &mut hdr, gb).is_err() {
            return Ok(ObuEnd::Error);
        }
        for n in 0..c.n_tile_data as usize {
            c.tile[n].data.unref();
        }
        c.n_tile_data = 0;
        c.n_tiles = 0;
        if type_ != DAV1D_OBU_FRAME {
            // This is actually a frame header OBU so read the
            // trailing bit and check for overrun.
            if check_trailing_bits(gb, c.strict_std_compliance).is_err() {
                return Ok(ObuEnd::Error);
            }
        }

        if c.frame_size_limit != 0
            && hdr.width[1] as i64 * hdr.height as i64 > c.frame_size_limit as i64
        {
            dav1d_log(
                c,
                &format!(
                    "Frame size {}x{} exceeds limit {}\n",
                    hdr.width[1], hdr.height, c.frame_size_limit
                ),
            );
            return Err(dav1d_err(ERANGE));
        }

        if type_ == DAV1D_OBU_FRAME {
            // OBU_FRAMEs shouldn't be signaled with show_existing_frame
            if hdr.show_existing_frame != 0 {
                return Ok(ObuEnd::Error);
            }

            // This is the frame header at the start of a frame OBU.
            // There's no trailing bit at the end to skip, but we do need
            // to align to the next byte.
            gb.bytealign();
            parse_tiles = true;
        }
        c.frame_hdr = Some(Arc::new(hdr));
    }

    if parse_tiles {
        let Some(frame_hdr) = c.frame_hdr.clone() else {
            return Ok(ObuEnd::Error);
        };
        let n = c.n_tile_data as usize;
        if c.tile.len() < n + 1 {
            if c.tile.try_reserve(1).is_err() {
                return Ok(ObuEnd::Error);
            }
            c.tile.push(Dav1dTileGroup::default());
        }
        parse_tile_hdr(&frame_hdr, &mut c.tile[n], gb);
        // Align to the next byte boundary and check for overrun.
        gb.bytealign();
        if gb.error != 0 {
            return Ok(ObuEnd::Error);
        }

        c.tile[n].data = Dav1dData {
            buf: in_.buf.clone(),
            offset: in_.offset + gb.ptr,
            sz: gb.ptr_end - gb.ptr,
            m: in_.m.clone(),
        };
        // ensure tile groups are in order and sane, see 6.10.1
        if c.tile[n].start > c.tile[n].end || c.tile[n].start != c.n_tiles {
            for i in 0..=n {
                c.tile[i].data.unref();
            }
            c.n_tile_data = 0;
            c.n_tiles = 0;
            return Ok(ObuEnd::Error);
        }
        c.n_tiles += 1 + c.tile[n].end - c.tile[n].start;
        c.n_tile_data += 1;
    }

    if let (Some(_), Some(frame_hdr)) = (&c.seq_hdr, c.frame_hdr.clone()) {
        if frame_hdr.show_existing_frame != 0 {
            let idx = frame_hdr.existing_frame_idx as usize;
            let Some(ref_frame_type) = ref_hdr(c, idx).map(|h| h.frame_type) else {
                return Ok(ObuEnd::Error);
            };
            match ref_frame_type {
                DAV1D_FRAME_TYPE_INTER | DAV1D_FRAME_TYPE_SWITCH => {
                    if c.decode_frame_type > DAV1D_DECODEFRAMETYPE_REFERENCE {
                        return Ok(ObuEnd::Skip);
                    }
                }
                DAV1D_FRAME_TYPE_INTRA => {
                    if c.decode_frame_type > DAV1D_DECODEFRAMETYPE_INTRA {
                        return Ok(ObuEnd::Skip);
                    }
                }
                _ => {}
            }
            if c.refs[idx].p.p.data.is_none() {
                return Ok(ObuEnd::Error);
            }
            if c.strict_std_compliance && c.refs[idx].p.showable == 0 {
                return Ok(ObuEnd::Error);
            }
            c.out = c.refs[idx].p.clone();
            let (cl, md, it) = (
                c.content_light.clone(),
                c.mastering_display.clone(),
                c.itut_t35.clone(),
            );
            c.out.p.copy_props(&cl, &md, &it, &in_.m);
            // Must be removed from the context after being attached to the frame
            c.itut_t35 = None;
            c.event_flags |= picture_get_event_flags(&c.refs[idx].p);
            if ref_frame_type == DAV1D_FRAME_TYPE_KEY {
                let r = idx;
                c.refs[r].p.showable = 0;
                for i in 0..8 {
                    if i == r {
                        continue;
                    }

                    c.refs[i].p = c.refs[r].p.clone();

                    c.cdf[i] = c.cdf[r].clone();

                    c.refs[i].segmap = c.refs[r].segmap.clone();
                    c.refs[i].refmvs = None;
                }
            }
            c.frame_hdr = None;
        } else if c.n_tiles == frame_hdr.tiling.cols * frame_hdr.tiling.rows {
            match frame_hdr.frame_type {
                DAV1D_FRAME_TYPE_INTER | DAV1D_FRAME_TYPE_SWITCH => {
                    if c.decode_frame_type > DAV1D_DECODEFRAMETYPE_REFERENCE
                        || (c.decode_frame_type == DAV1D_DECODEFRAMETYPE_REFERENCE
                            && frame_hdr.refresh_frame_flags == 0)
                    {
                        return Ok(ObuEnd::Skip);
                    }
                }
                DAV1D_FRAME_TYPE_INTRA => {
                    if c.decode_frame_type > DAV1D_DECODEFRAMETYPE_INTRA
                        || (c.decode_frame_type == DAV1D_DECODEFRAMETYPE_REFERENCE
                            && frame_hdr.refresh_frame_flags == 0)
                    {
                        return Ok(ObuEnd::Skip);
                    }
                }
                _ => {}
            }
            if c.n_tile_data == 0 {
                return Ok(ObuEnd::Error);
            }
            submit_frame(c)?;
            debug_assert!(c.n_tile_data == 0);
            c.frame_hdr = None;
            c.n_tiles = 0;
        }
    }

    Ok(ObuEnd::Done)
}
