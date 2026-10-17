// Rust translation of the parts of av1/encoder/pickcdef.c from aom
// (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches:
// the CDEF strengths predicted from the quantizer (`CDEF_PICK_FROM_Q`, the
// method of every real-time speed the saver uses).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CDEF parameter selection.

use super::encoder::*;
use super::speed_features::CDEF_PICK_FROM_Q;
use crate::aom::aom_dsp::aom_dsp_common::clamp;
use crate::aom::av1::common::av1_common_int::{frame_is_intra_only, Av1Common};
use crate::aom::av1::common::blockd::MI_NONE;
use crate::aom::av1::common::cdef::CDEF_SEC_STRENGTHS;
use crate::aom::av1::common::enums::MI_SIZE_64X64;
use crate::aom::av1::common::quant_common::av1_ac_quant_qtx;

/// `CDEF_REFERENCE`: disable CDEF on non reference frames.
pub(crate) const CDEF_REFERENCE: i32 = 2;

/// Translation of `pick_cdef_from_qp()`.
fn pick_cdef_from_qp(
    cm: &mut Av1Common,
    skip_cdef: bool,
    is_screen_content: bool,
) -> Result<(), ()> {
    let bd = cm.seq_params.bit_depth;
    let q = av1_ac_quant_qtx(cm.quant_params.base_qindex, 0, bd) as i32 >> (bd - 8);
    let intra_only = frame_is_intra_only(cm);
    let cdef_info = &mut cm.cdef_info;
    // Check the speed feature to avoid extra signaling.
    if skip_cdef {
        cdef_info.cdef_bits = 1;
        cdef_info.nb_cdef_strengths = 2;
    } else {
        cdef_info.cdef_bits = 0;
        cdef_info.nb_cdef_strengths = 1;
    }
    cdef_info.cdef_damping = 3 + (cm.quant_params.base_qindex >> 6);

    let predicted_y_f1;
    let predicted_y_f2;
    let predicted_uv_f1;
    let predicted_uv_f2;
    if is_screen_content {
        let qd = q as f64;
        predicted_y_f1 = clamp(
            (5.88217781e-06 * qd * qd + 6.10391455e-03 * qd + 9.95043102e-02) as i32,
            0,
            15,
        );
        predicted_y_f2 = clamp(
            (-7.79934857e-06 * qd * qd + 6.58957830e-03 * qd + 8.81045025e-01) as i32,
            0,
            3,
        );
        predicted_uv_f1 = clamp(
            (-6.79500136e-06 * qd * qd + 1.02695586e-02 * qd + 1.36126802e-01) as i32,
            0,
            15,
        );
        predicted_uv_f2 = clamp(
            (-9.99613695e-08 * qd * qd - 1.79361339e-05 * qd + 1.17022324e+0) as i32,
            0,
            3,
        );
    } else {
        // (the products are `int * int * float` in C: the integer square is
        // converted to float before the multiplication)
        let qq = (q * q) as f32;
        let qf = q as f32;
        if !intra_only {
            predicted_y_f1 = clamp(
                (qq * -0.0000023593946f32 + qf * 0.0068615186f32 + 0.02709886f32).round() as i32,
                0,
                15,
            );
            predicted_y_f2 = clamp(
                (qq * -0.00000057629734f32 + qf * 0.0013993345f32 + 0.03831067f32).round() as i32,
                0,
                3,
            );
            predicted_uv_f1 = clamp(
                (qq * -0.0000007095069f32 + qf * 0.0034628846f32 + 0.00887099f32).round() as i32,
                0,
                15,
            );
            predicted_uv_f2 = clamp(
                (qq * 0.00000023874085f32 + qf * 0.00028223585f32 + 0.05576307f32).round() as i32,
                0,
                3,
            );
        } else {
            predicted_y_f1 = clamp(
                (qq * 0.0000033731974f32 + qf * 0.008070594f32 + 0.0187634f32).round() as i32,
                0,
                15,
            );
            predicted_y_f2 = clamp(
                (qq * 0.0000029167343f32 + qf * 0.0027798624f32 + 0.0079405f32).round() as i32,
                0,
                3,
            );
            predicted_uv_f1 = clamp(
                (qq * -0.0000130790995f32 + qf * 0.012892405f32 - 0.00748388f32).round() as i32,
                0,
                15,
            );
            predicted_uv_f2 = clamp(
                (qq * 0.0000032651783f32 + qf * 0.00035520183f32 + 0.00228092f32).round() as i32,
                0,
                3,
            );
        }
    }
    cdef_info.cdef_strengths[0] = predicted_y_f1 * CDEF_SEC_STRENGTHS + predicted_y_f2;
    cdef_info.cdef_uv_strengths[0] = predicted_uv_f1 * CDEF_SEC_STRENGTHS + predicted_uv_f2;

    // mbmi->cdef_strength is already set in the encoding stage. We don't need to
    // set it again here.
    if skip_cdef {
        cdef_info.cdef_strengths[1] = 0;
        cdef_info.cdef_uv_strengths[1] = 0;
        return Ok(());
    }

    let mi_params = &mut cm.mi_params;
    let nvfb = (mi_params.mi_rows + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    let nhfb = (mi_params.mi_cols + MI_SIZE_64X64 - 1) / MI_SIZE_64X64;
    for r in 0..nvfb {
        for c in 0..nhfb {
            let idx = (MI_SIZE_64X64 * r * mi_params.mi_stride + MI_SIZE_64X64 * c) as usize;
            let i = *mi_params.mi_grid_base.get(idx).ok_or(())?;
            if i == MI_NONE {
                return Err(());
            }
            let current_mbmi = mi_params.mi_alloc.get_mut(i as usize).ok_or(())?;
            current_mbmi.cdef_strength = 0;
        }
    }
    Ok(())
}

/// Translation of `av1_cdef_search()` for the `CDEF_PICK_FROM_Q` method
/// (the other methods search the filtered frame; they belong to the good
/// quality speeds, which the saver never uses, and fail).
pub(crate) fn av1_cdef_search(cpi: &mut Av1Comp, is_screen_content: bool) -> Result<(), ()> {
    let cdef_control = cpi.oxcf.tool_cfg.cdef_control;
    let non_reference_frame = cpi.ppi.rtc_ref.non_reference_frame;
    let pick_method = cpi.sf.lpf_sf.cdef_pick_method;
    let skip_cdef_feature = cpi.sf.rt_sf.skip_cdef_sb != 0;
    let cm = &mut cpi.common;
    debug_assert!(cdef_control != CDEF_NONE);
    if cdef_control == CDEF_REFERENCE && non_reference_frame {
        let cdef_info = &mut cm.cdef_info;
        cdef_info.nb_cdef_strengths = 1;
        cdef_info.cdef_bits = 0;
        cdef_info.cdef_strengths[0] = 0;
        cdef_info.cdef_uv_strengths[0] = 0;
        return Ok(());
    }

    if pick_method == CDEF_PICK_FROM_Q {
        return pick_cdef_from_qp(cm, skip_cdef_feature, is_screen_content);
    }
    Err(())
}
