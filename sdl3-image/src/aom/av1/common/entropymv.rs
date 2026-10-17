// Rust translation of av1/common/entropymv.c and av1/common/entropymv.h from
// aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches): the motion vector
// symbol contexts.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Motion vector entropy contexts.

use crate::aom::aom_dsp::prob::cdf_pad;

pub(crate) const MV_UPDATE_PROB: i32 = 252;

/* Symbols for coding which components are zero jointly */
pub(crate) const MV_JOINTS: i32 = 4;
/// `MV_JOINT_TYPE`
pub(crate) type MvJointType = i32;
pub(crate) const MV_JOINT_ZERO: MvJointType = 0; /* Zero vector */
pub(crate) const MV_JOINT_HNZVZ: MvJointType = 1; /* Vert zero, hor nonzero */
pub(crate) const MV_JOINT_HZVNZ: MvJointType = 2; /* Hor zero, vert nonzero */
pub(crate) const MV_JOINT_HNZVNZ: MvJointType = 3; /* Both components nonzero */

#[inline]
pub(crate) fn mv_joint_vertical(type_: MvJointType) -> bool {
    type_ == MV_JOINT_HZVNZ || type_ == MV_JOINT_HNZVNZ
}

#[inline]
pub(crate) fn mv_joint_horizontal(type_: MvJointType) -> bool {
    type_ == MV_JOINT_HNZVZ || type_ == MV_JOINT_HNZVNZ
}

/* Symbols for coding magnitude class of nonzero components */
pub(crate) const MV_CLASSES: i32 = 11;
pub(crate) const MV_CLASS_0: i32 = 0; /* (0, 2]     integer pel */
pub(crate) const MV_CLASS_10: i32 = 10; /* (1024,2048] integer pel */

pub(crate) const CLASS0_BITS: i32 = 1; /* bits at integer precision for class 0 */
pub(crate) const CLASS0_SIZE: i32 = 1 << CLASS0_BITS;
pub(crate) const MV_OFFSET_BITS: i32 = MV_CLASSES + CLASS0_BITS - 2;
pub(crate) const MV_BITS_CONTEXTS: i32 = 6;
pub(crate) const MV_FP_SIZE: i32 = 4;

pub(crate) const MV_MAX_BITS: i32 = MV_CLASSES + CLASS0_BITS + 2;
pub(crate) const MV_MAX: i32 = (1 << MV_MAX_BITS) - 1;
pub(crate) const MV_VALS: i32 = (MV_MAX << 1) + 1;

pub(crate) const MV_IN_USE_BITS: i32 = 14;
pub(crate) const MV_UPP: i32 = 1 << MV_IN_USE_BITS;
pub(crate) const MV_LOW: i32 = -(1 << MV_IN_USE_BITS);

/// `nmv_component`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NmvComponent {
    pub(crate) classes_cdf: [u16; MV_CLASSES as usize + 1],
    pub(crate) class0_fp_cdf: [[u16; MV_FP_SIZE as usize + 1]; CLASS0_SIZE as usize],
    pub(crate) fp_cdf: [u16; MV_FP_SIZE as usize + 1],
    pub(crate) sign_cdf: [u16; 2 + 1],
    pub(crate) class0_hp_cdf: [u16; 2 + 1],
    pub(crate) hp_cdf: [u16; 2 + 1],
    pub(crate) class0_cdf: [u16; CLASS0_SIZE as usize + 1],
    pub(crate) bits_cdf: [[u16; 2 + 1]; MV_OFFSET_BITS as usize],
}

/// `nmv_context`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NmvContext {
    pub(crate) joints_cdf: [u16; MV_JOINTS as usize + 1],
    pub(crate) comps: [NmvComponent; 2],
}

/// `MvSubpelPrecision`
pub(crate) const MV_SUBPEL_NONE: i32 = -1;
pub(crate) const MV_SUBPEL_LOW_PRECISION: i32 = 0;
pub(crate) const MV_SUBPEL_HIGH_PRECISION: i32 = 1;

const DEFAULT_NMV_COMPONENT: NmvComponent = NmvComponent {
    classes_cdf: cdf_pad(&[
        28672, 30976, 31858, 32320, 32551, 32656, 32740, 32757, 32762, 32767,
    ]), // class_cdf // fp
    class0_fp_cdf: [
        cdf_pad(&[16384, 24576, 26624]),
        cdf_pad(&[12288, 21248, 24128]),
    ], // class0_fp_cdf
    fp_cdf: cdf_pad(&[8192, 17408, 21248]), // fp_cdf
    sign_cdf: cdf_pad(&[128 * 128]),        // sign_cdf
    class0_hp_cdf: cdf_pad(&[160 * 128]),   // class0_hp_cdf
    hp_cdf: cdf_pad(&[128 * 128]),          // hp_cdf
    class0_cdf: cdf_pad(&[216 * 128]),      // class0_cdf
    bits_cdf: [
        cdf_pad(&[128 * 136]),
        cdf_pad(&[128 * 140]),
        cdf_pad(&[128 * 148]),
        cdf_pad(&[128 * 160]),
        cdf_pad(&[128 * 176]),
        cdf_pad(&[128 * 192]),
        cdf_pad(&[128 * 224]),
        cdf_pad(&[128 * 234]),
        cdf_pad(&[128 * 234]),
        cdf_pad(&[128 * 240]),
    ], // bits_cdf
};

pub(crate) const DEFAULT_NMV_CONTEXT: NmvContext = NmvContext {
    joints_cdf: cdf_pad(&[4096, 11264, 19328]), // joints_cdf
    comps: [
        // Vertical component
        DEFAULT_NMV_COMPONENT,
        // Horizontal component
        DEFAULT_NMV_COMPONENT,
    ],
};

impl Default for NmvContext {
    /// The zeroed context of a newly allocated frame context.
    fn default() -> Self {
        const C: NmvComponent = NmvComponent {
            classes_cdf: [0; MV_CLASSES as usize + 1],
            class0_fp_cdf: [[0; MV_FP_SIZE as usize + 1]; CLASS0_SIZE as usize],
            fp_cdf: [0; MV_FP_SIZE as usize + 1],
            sign_cdf: [0; 3],
            class0_hp_cdf: [0; 3],
            hp_cdf: [0; 3],
            class0_cdf: [0; CLASS0_SIZE as usize + 1],
            bits_cdf: [[0; 3]; MV_OFFSET_BITS as usize],
        };
        NmvContext {
            joints_cdf: [0; MV_JOINTS as usize + 1],
            comps: [C, C],
        }
    }
}
