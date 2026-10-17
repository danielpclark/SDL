// Rust translation of av1/common/entropymode.c, av1/common/entropymode.h and
// av1/common/entropymv.c from aom (https://aomedia.googlesource.com/aom, at
// the revision SDL_image's external/aom pins: libaom 3.6.1 with SDL's
// patches).
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The frame context (`FRAME_CONTEXT`) and its defaults. Only the CDFs of
//! intra frames are kept: the encoder codes still images as single key
//! frames without palettes, intra block copy, segmentation, delta loop
//! filter levels or loop restoration; the inter-frame CDFs (and the motion
//! vector contexts `av1_init_mv_probs()` sets) are never read.

use super::av1_common_int::Av1Common;
use super::entropy::*;
use super::enums::*;
use crate::aom::aom_dsp::prob::cdf_pad;

pub(crate) const BLOCK_SIZE_GROUPS: i32 = 4;

pub(crate) const TX_SIZE_CONTEXTS: i32 = 3;

pub(crate) const KF_MODE_CONTEXTS: i32 = 5;

/// Translation of `FRAME_CONTEXT` (`struct frame_contexts`), its intra
/// subset.
#[derive(Debug, Clone)]
pub(crate) struct FrameContext {
    pub(crate) txb_skip_cdf: [[[u16; 3]; TXB_SKIP_CONTEXTS as usize]; TX_SIZES as usize],
    pub(crate) eob_extra_cdf:
        [[[[u16; 3]; EOB_COEF_CONTEXTS as usize]; PLANE_TYPES as usize]; TX_SIZES as usize],
    pub(crate) dc_sign_cdf: [[[u16; 3]; DC_SIGN_CONTEXTS as usize]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf16: [[[u16; 6]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf32: [[[u16; 7]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf64: [[[u16; 8]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf128: [[[u16; 9]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf256: [[[u16; 10]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf512: [[[u16; 11]; 2]; PLANE_TYPES as usize],
    pub(crate) eob_flag_cdf1024: [[[u16; 12]; 2]; PLANE_TYPES as usize],
    pub(crate) coeff_base_eob_cdf:
        [[[[u16; 4]; SIG_COEF_CONTEXTS_EOB as usize]; PLANE_TYPES as usize]; TX_SIZES as usize],
    pub(crate) coeff_base_cdf:
        [[[[u16; 5]; SIG_COEF_CONTEXTS as usize]; PLANE_TYPES as usize]; TX_SIZES as usize],
    pub(crate) coeff_br_cdf: [[[[u16; BR_CDF_SIZE as usize + 1]; LEVEL_CONTEXTS as usize];
        PLANE_TYPES as usize]; TX_SIZES as usize],

    pub(crate) skip_txfm_cdfs: [[u16; 3]; SKIP_CONTEXTS as usize],
    pub(crate) intrabc_cdf: [u16; 3],
    pub(crate) filter_intra_cdfs: [[u16; 3]; BLOCK_SIZES_ALL as usize],
    pub(crate) filter_intra_mode_cdf: [u16; FILTER_INTRA_MODES as usize + 1],
    pub(crate) y_mode_cdf: [[u16; INTRA_MODES as usize + 1]; BLOCK_SIZE_GROUPS as usize],
    pub(crate) uv_mode_cdf:
        [[[u16; UV_INTRA_MODES as usize + 1]; INTRA_MODES as usize]; CFL_ALLOWED_TYPES as usize],
    pub(crate) partition_cdf:
        [[u16; EXT_PARTITION_TYPES as usize + 1]; PARTITION_CONTEXTS as usize],
    /* kf_y_cdf is discarded after use, so does not require persistent storage.
       However, we keep it with the other CDFs in this struct since it needs to
       be copied to each tile to support parallelism just like the others.
    */
    pub(crate) kf_y_cdf:
        [[[u16; INTRA_MODES as usize + 1]; KF_MODE_CONTEXTS as usize]; KF_MODE_CONTEXTS as usize],

    pub(crate) angle_delta_cdf:
        [[u16; 2 * MAX_ANGLE_DELTA as usize + 2]; DIRECTIONAL_MODES as usize],

    pub(crate) tx_size_cdf:
        [[[u16; MAX_TX_DEPTH as usize + 2]; TX_SIZE_CONTEXTS as usize]; MAX_TX_CATS as usize],
    pub(crate) delta_q_cdf: [u16; DELTA_Q_PROBS as usize + 2],
    pub(crate) delta_lf_multi_cdf: [[u16; DELTA_LF_PROBS as usize + 2]; FRAME_LF_COUNT as usize],
    pub(crate) delta_lf_cdf: [u16; DELTA_LF_PROBS as usize + 2],
    pub(crate) intra_ext_tx_cdf: [[[[u16; TX_TYPES as usize + 1]; INTRA_MODES as usize];
        EXT_TX_SIZES as usize]; EXT_TX_SETS_INTRA as usize],
    pub(crate) cfl_sign_cdf: [u16; CFL_JOINT_SIGNS as usize + 1],
    pub(crate) cfl_alpha_cdf: [[u16; CFL_ALPHABET_SIZE as usize + 1]; CFL_ALPHA_CONTEXTS as usize],
    pub(crate) initialized: i32,
}

impl Default for FrameContext {
    fn default() -> Self {
        FrameContext {
            txb_skip_cdf: [[[0; 3]; TXB_SKIP_CONTEXTS as usize]; TX_SIZES as usize],
            eob_extra_cdf: [[[[0; 3]; EOB_COEF_CONTEXTS as usize]; PLANE_TYPES as usize];
                TX_SIZES as usize],
            dc_sign_cdf: [[[0; 3]; DC_SIGN_CONTEXTS as usize]; PLANE_TYPES as usize],
            eob_flag_cdf16: [[[0; 6]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf32: [[[0; 7]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf64: [[[0; 8]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf128: [[[0; 9]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf256: [[[0; 10]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf512: [[[0; 11]; 2]; PLANE_TYPES as usize],
            eob_flag_cdf1024: [[[0; 12]; 2]; PLANE_TYPES as usize],
            coeff_base_eob_cdf: [[[[0; 4]; SIG_COEF_CONTEXTS_EOB as usize]; PLANE_TYPES as usize];
                TX_SIZES as usize],
            coeff_base_cdf: [[[[0; 5]; SIG_COEF_CONTEXTS as usize]; PLANE_TYPES as usize];
                TX_SIZES as usize],
            coeff_br_cdf: [[[[0; BR_CDF_SIZE as usize + 1]; LEVEL_CONTEXTS as usize];
                PLANE_TYPES as usize]; TX_SIZES as usize],
            skip_txfm_cdfs: [[0; 3]; SKIP_CONTEXTS as usize],
            intrabc_cdf: [0; 3],
            filter_intra_cdfs: [[0; 3]; BLOCK_SIZES_ALL as usize],
            filter_intra_mode_cdf: [0; FILTER_INTRA_MODES as usize + 1],
            y_mode_cdf: [[0; INTRA_MODES as usize + 1]; BLOCK_SIZE_GROUPS as usize],
            uv_mode_cdf: [[[0; UV_INTRA_MODES as usize + 1]; INTRA_MODES as usize];
                CFL_ALLOWED_TYPES as usize],
            partition_cdf: [[0; EXT_PARTITION_TYPES as usize + 1]; PARTITION_CONTEXTS as usize],
            kf_y_cdf: [[[0; INTRA_MODES as usize + 1]; KF_MODE_CONTEXTS as usize];
                KF_MODE_CONTEXTS as usize],
            angle_delta_cdf: [[0; 2 * MAX_ANGLE_DELTA as usize + 2]; DIRECTIONAL_MODES as usize],
            tx_size_cdf: [[[0; MAX_TX_DEPTH as usize + 2]; TX_SIZE_CONTEXTS as usize];
                MAX_TX_CATS as usize],
            delta_q_cdf: [0; DELTA_Q_PROBS as usize + 2],
            delta_lf_multi_cdf: [[0; DELTA_LF_PROBS as usize + 2]; FRAME_LF_COUNT as usize],
            delta_lf_cdf: [0; DELTA_LF_PROBS as usize + 2],
            intra_ext_tx_cdf: [[[[0; TX_TYPES as usize + 1]; INTRA_MODES as usize];
                EXT_TX_SIZES as usize]; EXT_TX_SETS_INTRA as usize],
            cfl_sign_cdf: [0; CFL_JOINT_SIGNS as usize + 1],
            cfl_alpha_cdf: [[0; CFL_ALPHABET_SIZE as usize + 1]; CFL_ALPHA_CONTEXTS as usize],
            initialized: 0,
        }
    }
}

/// Translation of `av1_init_mode_probs()`, for the intra CDFs.
pub(crate) fn av1_init_mode_probs(fc: &mut FrameContext) {
    fc.kf_y_cdf = DEFAULT_KF_Y_MODE_CDF;
    fc.angle_delta_cdf = DEFAULT_ANGLE_DELTA_CDF;
    fc.filter_intra_cdfs = DEFAULT_FILTER_INTRA_CDFS;
    fc.filter_intra_mode_cdf = DEFAULT_FILTER_INTRA_MODE_CDF;
    fc.y_mode_cdf = DEFAULT_IF_Y_MODE_CDF;
    fc.uv_mode_cdf = DEFAULT_UV_MODE_CDF;
    fc.partition_cdf = DEFAULT_PARTITION_CDF;
    fc.intra_ext_tx_cdf = DEFAULT_INTRA_EXT_TX_CDF;
    fc.skip_txfm_cdfs = DEFAULT_SKIP_TXFM_CDFS;
    fc.tx_size_cdf = DEFAULT_TX_SIZE_CDF;
    fc.delta_q_cdf = DEFAULT_DELTA_Q_CDF;
    fc.delta_lf_cdf = DEFAULT_DELTA_LF_CDF;
    fc.delta_lf_multi_cdf = DEFAULT_DELTA_LF_MULTI_CDF;
    fc.cfl_sign_cdf = DEFAULT_CFL_SIGN_CDF;
    fc.cfl_alpha_cdf = DEFAULT_CFL_ALPHA_CDF;
    fc.intrabc_cdf = DEFAULT_INTRABC_CDF;
}

pub(crate) fn av1_set_default_ref_deltas(ref_deltas: &mut [i8; REF_FRAMES as usize]) {
    ref_deltas[INTRA_FRAME as usize] = 1;
    ref_deltas[LAST_FRAME as usize] = 0;
    ref_deltas[LAST2_FRAME as usize] = ref_deltas[LAST_FRAME as usize];
    ref_deltas[LAST3_FRAME as usize] = ref_deltas[LAST_FRAME as usize];
    ref_deltas[BWDREF_FRAME as usize] = ref_deltas[LAST_FRAME as usize];
    ref_deltas[GOLDEN_FRAME as usize] = -1;
    ref_deltas[ALTREF2_FRAME as usize] = -1;
    ref_deltas[ALTREF_FRAME as usize] = -1;
}

pub(crate) fn av1_set_default_mode_deltas(mode_deltas: &mut [i8; MAX_MODE_LF_DELTAS as usize]) {
    mode_deltas[0] = 0;
    mode_deltas[1] = 0;
}

fn set_default_lf_deltas(lf: &mut super::av1_loopfilter::Loopfilter) {
    lf.mode_ref_delta_enabled = 1;
    lf.mode_ref_delta_update = 1;

    av1_set_default_ref_deltas(&mut lf.ref_deltas);
    av1_set_default_mode_deltas(&mut lf.mode_deltas);
}

/// Translation of `av1_setup_frame_contexts()` (no large scale tiles).
pub(crate) fn av1_setup_frame_contexts(cm: &mut Av1Common) {
    // Store the frame context into a special slot (not associated with any
    // reference buffer), so that we can set up cm->pre_fc correctly later
    // This function must ONLY be called when cm->fc has been initialized with
    // default probs, either by av1_setup_past_independence or after manually
    // initializing them
    cm.default_frame_context = cm.fc.clone();
}

/// Translation of `av1_setup_past_independence()` (no segmentation map).
pub(crate) fn av1_setup_past_independence(cm: &mut Av1Common) {
    // Reset the segment feature data to the default stats:
    // Features disabled, 0, with delta coding (Default state).
    super::seg_common::av1_clearall_segfeatures(&mut cm.seg);

    // reset mode ref deltas
    av1_set_default_ref_deltas(&mut cm.cur_frame_mut().ref_deltas);
    av1_set_default_mode_deltas(&mut cm.cur_frame_mut().mode_deltas);
    set_default_lf_deltas(&mut cm.lf);

    super::entropy::av1_default_coef_probs(cm);
    av1_init_mode_probs(&mut cm.fc);
    // (av1_init_mv_probs(): the motion vector contexts are not kept)
    cm.fc.initialized = 1;
    av1_setup_frame_contexts(cm);
}

// (the default CDFs)

pub(crate) static DEFAULT_KF_Y_MODE_CDF: [[[u16; (INTRA_MODES) as usize + 1];
    KF_MODE_CONTEXTS as usize];
    KF_MODE_CONTEXTS as usize] = [
    [
        cdf_pad(&[
            15588, 17027, 19338, 20218, 20682, 21110, 21825, 23244, 24189, 28165, 29093, 30466,
        ]),
        cdf_pad(&[
            12016, 18066, 19516, 20303, 20719, 21444, 21888, 23032, 24434, 28658, 30172, 31409,
        ]),
        cdf_pad(&[
            10052, 10771, 22296, 22788, 23055, 23239, 24133, 25620, 26160, 29336, 29929, 31567,
        ]),
        cdf_pad(&[
            14091, 15406, 16442, 18808, 19136, 19546, 19998, 22096, 24746, 29585, 30958, 32462,
        ]),
        cdf_pad(&[
            12122, 13265, 15603, 16501, 18609, 20033, 22391, 25583, 26437, 30261, 31073, 32475,
        ]),
    ],
    [
        cdf_pad(&[
            10023, 19585, 20848, 21440, 21832, 22760, 23089, 24023, 25381, 29014, 30482, 31436,
        ]),
        cdf_pad(&[
            5983, 24099, 24560, 24886, 25066, 25795, 25913, 26423, 27610, 29905, 31276, 31794,
        ]),
        cdf_pad(&[
            7444, 12781, 20177, 20728, 21077, 21607, 22170, 23405, 24469, 27915, 29090, 30492,
        ]),
        cdf_pad(&[
            8537, 14689, 15432, 17087, 17408, 18172, 18408, 19825, 24649, 29153, 31096, 32210,
        ]),
        cdf_pad(&[
            7543, 14231, 15496, 16195, 17905, 20717, 21984, 24516, 26001, 29675, 30981, 31994,
        ]),
    ],
    [
        cdf_pad(&[
            12613, 13591, 21383, 22004, 22312, 22577, 23401, 25055, 25729, 29538, 30305, 32077,
        ]),
        cdf_pad(&[
            9687, 13470, 18506, 19230, 19604, 20147, 20695, 22062, 23219, 27743, 29211, 30907,
        ]),
        cdf_pad(&[
            6183, 6505, 26024, 26252, 26366, 26434, 27082, 28354, 28555, 30467, 30794, 32086,
        ]),
        cdf_pad(&[
            10718, 11734, 14954, 17224, 17565, 17924, 18561, 21523, 23878, 28975, 30287, 32252,
        ]),
        cdf_pad(&[
            9194, 9858, 16501, 17263, 18424, 19171, 21563, 25961, 26561, 30072, 30737, 32463,
        ]),
    ],
    [
        cdf_pad(&[
            12602, 14399, 15488, 18381, 18778, 19315, 19724, 21419, 25060, 29696, 30917, 32409,
        ]),
        cdf_pad(&[
            8203, 13821, 14524, 17105, 17439, 18131, 18404, 19468, 25225, 29485, 31158, 32342,
        ]),
        cdf_pad(&[
            8451, 9731, 15004, 17643, 18012, 18425, 19070, 21538, 24605, 29118, 30078, 32018,
        ]),
        cdf_pad(&[
            7714, 9048, 9516, 16667, 16817, 16994, 17153, 18767, 26743, 30389, 31536, 32528,
        ]),
        cdf_pad(&[
            8843, 10280, 11496, 15317, 16652, 17943, 19108, 22718, 25769, 29953, 30983, 32485,
        ]),
    ],
    [
        cdf_pad(&[
            12578, 13671, 15979, 16834, 19075, 20913, 22989, 25449, 26219, 30214, 31150, 32477,
        ]),
        cdf_pad(&[
            9563, 13626, 15080, 15892, 17756, 20863, 22207, 24236, 25380, 29653, 31143, 32277,
        ]),
        cdf_pad(&[
            8356, 8901, 17616, 18256, 19350, 20106, 22598, 25947, 26466, 29900, 30523, 32261,
        ]),
        cdf_pad(&[
            10835, 11815, 13124, 16042, 17018, 18039, 18947, 22753, 24615, 29489, 30883, 32482,
        ]),
        cdf_pad(&[
            7618, 8288, 9859, 10509, 15386, 18657, 22903, 28776, 29180, 31355, 31802, 32593,
        ]),
    ],
];

pub(crate) static DEFAULT_ANGLE_DELTA_CDF: [[u16; (2 * MAX_ANGLE_DELTA + 1) as usize + 1];
    DIRECTIONAL_MODES as usize] = [
    cdf_pad(&[2180, 5032, 7567, 22776, 26989, 30217]),
    cdf_pad(&[2301, 5608, 8801, 23487, 26974, 30330]),
    cdf_pad(&[3780, 11018, 13699, 19354, 23083, 31286]),
    cdf_pad(&[4581, 11226, 15147, 17138, 21834, 28397]),
    cdf_pad(&[1737, 10927, 14509, 19588, 22745, 28823]),
    cdf_pad(&[2664, 10176, 12485, 17650, 21600, 30495]),
    cdf_pad(&[2240, 11096, 15453, 20341, 22561, 28917]),
    cdf_pad(&[3605, 10428, 12459, 17676, 21244, 30655]),
];

pub(crate) static DEFAULT_IF_Y_MODE_CDF: [[u16; (INTRA_MODES) as usize + 1];
    BLOCK_SIZE_GROUPS as usize] = [
    cdf_pad(&[
        22801, 23489, 24293, 24756, 25601, 26123, 26606, 27418, 27945, 29228, 29685, 30349,
    ]),
    cdf_pad(&[
        18673, 19845, 22631, 23318, 23950, 24649, 25527, 27364, 28152, 29701, 29984, 30852,
    ]),
    cdf_pad(&[
        19770, 20979, 23396, 23939, 24241, 24654, 25136, 27073, 27830, 29360, 29730, 30659,
    ]),
    cdf_pad(&[
        20155, 21301, 22838, 23178, 23261, 23533, 23703, 24804, 25352, 26575, 27016, 28049,
    ]),
];

pub(crate) static DEFAULT_UV_MODE_CDF: [[[u16; (UV_INTRA_MODES) as usize + 1];
    INTRA_MODES as usize]; CFL_ALLOWED_TYPES as usize] = [
    [
        cdf_pad(&[
            22631, 24152, 25378, 25661, 25986, 26520, 27055, 27923, 28244, 30059, 30941, 31961,
        ]),
        cdf_pad(&[
            9513, 26881, 26973, 27046, 27118, 27664, 27739, 27824, 28359, 29505, 29800, 31796,
        ]),
        cdf_pad(&[
            9845, 9915, 28663, 28704, 28757, 28780, 29198, 29822, 29854, 30764, 31777, 32029,
        ]),
        cdf_pad(&[
            13639, 13897, 14171, 25331, 25606, 25727, 25953, 27148, 28577, 30612, 31355, 32493,
        ]),
        cdf_pad(&[
            9764, 9835, 9930, 9954, 25386, 27053, 27958, 28148, 28243, 31101, 31744, 32363,
        ]),
        cdf_pad(&[
            11825, 13589, 13677, 13720, 15048, 29213, 29301, 29458, 29711, 31161, 31441, 32550,
        ]),
        cdf_pad(&[
            14175, 14399, 16608, 16821, 17718, 17775, 28551, 30200, 30245, 31837, 32342, 32667,
        ]),
        cdf_pad(&[
            12885, 13038, 14978, 15590, 15673, 15748, 16176, 29128, 29267, 30643, 31961, 32461,
        ]),
        cdf_pad(&[
            12026, 13661, 13874, 15305, 15490, 15726, 15995, 16273, 28443, 30388, 30767, 32416,
        ]),
        cdf_pad(&[
            19052, 19840, 20579, 20916, 21150, 21467, 21885, 22719, 23174, 28861, 30379, 32175,
        ]),
        cdf_pad(&[
            18627, 19649, 20974, 21219, 21492, 21816, 22199, 23119, 23527, 27053, 31397, 32148,
        ]),
        cdf_pad(&[
            17026, 19004, 19997, 20339, 20586, 21103, 21349, 21907, 22482, 25896, 26541, 31819,
        ]),
        cdf_pad(&[
            12124, 13759, 14959, 14992, 15007, 15051, 15078, 15166, 15255, 15753, 16039, 16606,
        ]),
    ],
    [
        cdf_pad(&[
            10407, 11208, 12900, 13181, 13823, 14175, 14899, 15656, 15986, 20086, 20995, 22455,
            24212,
        ]),
        cdf_pad(&[
            4532, 19780, 20057, 20215, 20428, 21071, 21199, 21451, 22099, 24228, 24693, 27032,
            29472,
        ]),
        cdf_pad(&[
            5273, 5379, 20177, 20270, 20385, 20439, 20949, 21695, 21774, 23138, 24256, 24703, 26679,
        ]),
        cdf_pad(&[
            6740, 7167, 7662, 14152, 14536, 14785, 15034, 16741, 18371, 21520, 22206, 23389, 24182,
        ]),
        cdf_pad(&[
            4987, 5368, 5928, 6068, 19114, 20315, 21857, 22253, 22411, 24911, 25380, 26027, 26376,
        ]),
        cdf_pad(&[
            5370, 6889, 7247, 7393, 9498, 21114, 21402, 21753, 21981, 24780, 25386, 26517, 27176,
        ]),
        cdf_pad(&[
            4816, 4961, 7204, 7326, 8765, 8930, 20169, 20682, 20803, 23188, 23763, 24455, 24940,
        ]),
        cdf_pad(&[
            6608, 6740, 8529, 9049, 9257, 9356, 9735, 18827, 19059, 22336, 23204, 23964, 24793,
        ]),
        cdf_pad(&[
            5998, 7419, 7781, 8933, 9255, 9549, 9753, 10417, 18898, 22494, 23139, 24764, 25989,
        ]),
        cdf_pad(&[
            10660, 11298, 12550, 12957, 13322, 13624, 14040, 15004, 15534, 20714, 21789, 23443,
            24861,
        ]),
        cdf_pad(&[
            10522, 11530, 12552, 12963, 13378, 13779, 14245, 15235, 15902, 20102, 22696, 23774,
            25838,
        ]),
        cdf_pad(&[
            10099, 10691, 12639, 13049, 13386, 13665, 14125, 15163, 15636, 19676, 20474, 23519,
            25208,
        ]),
        cdf_pad(&[
            3144, 5087, 7382, 7504, 7593, 7690, 7801, 8064, 8232, 9248, 9875, 10521, 29048,
        ]),
    ],
];

pub(crate) static DEFAULT_PARTITION_CDF: [[u16; (EXT_PARTITION_TYPES) as usize + 1];
    PARTITION_CONTEXTS as usize] = [
    cdf_pad(&[19132, 25510, 30392]),
    cdf_pad(&[13928, 19855, 28540]),
    cdf_pad(&[12522, 23679, 28629]),
    cdf_pad(&[9896, 18783, 25853]),
    cdf_pad(&[
        15597, 20929, 24571, 26706, 27664, 28821, 29601, 30571, 31902,
    ]),
    cdf_pad(&[7925, 11043, 16785, 22470, 23971, 25043, 26651, 28701, 29834]),
    cdf_pad(&[5414, 13269, 15111, 20488, 22360, 24500, 25537, 26336, 32117]),
    cdf_pad(&[2662, 6362, 8614, 20860, 23053, 24778, 26436, 27829, 31171]),
    cdf_pad(&[
        18462, 20920, 23124, 27647, 28227, 29049, 29519, 30178, 31544,
    ]),
    cdf_pad(&[7689, 9060, 12056, 24992, 25660, 26182, 26951, 28041, 29052]),
    cdf_pad(&[6015, 9009, 10062, 24544, 25409, 26545, 27071, 27526, 32047]),
    cdf_pad(&[1394, 2208, 2796, 28614, 29061, 29466, 29840, 30185, 31899]),
    cdf_pad(&[
        20137, 21547, 23078, 29566, 29837, 30261, 30524, 30892, 31724,
    ]),
    cdf_pad(&[6732, 7490, 9497, 27944, 28250, 28515, 28969, 29630, 30104]),
    cdf_pad(&[5945, 7663, 8348, 28683, 29117, 29749, 30064, 30298, 32238]),
    cdf_pad(&[870, 1212, 1487, 31198, 31394, 31574, 31743, 31881, 32332]),
    cdf_pad(&[27899, 28219, 28529, 32484, 32539, 32619, 32639]),
    cdf_pad(&[6607, 6990, 8268, 32060, 32219, 32338, 32371]),
    cdf_pad(&[5429, 6676, 7122, 32027, 32227, 32531, 32582]),
    cdf_pad(&[711, 966, 1172, 32448, 32538, 32617, 32664]),
];

pub(crate) static DEFAULT_INTRA_EXT_TX_CDF: [[[[u16; (TX_TYPES) as usize + 1];
    INTRA_MODES as usize]; EXT_TX_SIZES as usize];
    EXT_TX_SETS_INTRA as usize] = [
    [
        [
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
        ],
        [
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
        ],
        [
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
        ],
        [
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
            cdf_pad(&[]),
        ],
    ],
    [
        [
            cdf_pad(&[1535, 8035, 9461, 12751, 23467, 27825]),
            cdf_pad(&[564, 3335, 9709, 10870, 18143, 28094]),
            cdf_pad(&[672, 3247, 3676, 11982, 19415, 23127]),
            cdf_pad(&[5279, 13885, 15487, 18044, 23527, 30252]),
            cdf_pad(&[4423, 6074, 7985, 10416, 25693, 29298]),
            cdf_pad(&[1486, 4241, 9460, 10662, 16456, 27694]),
            cdf_pad(&[439, 2838, 3522, 6737, 18058, 23754]),
            cdf_pad(&[1190, 4233, 4855, 11670, 20281, 24377]),
            cdf_pad(&[1045, 4312, 8647, 10159, 18644, 29335]),
            cdf_pad(&[202, 3734, 4747, 7298, 17127, 24016]),
            cdf_pad(&[447, 4312, 6819, 8884, 16010, 23858]),
            cdf_pad(&[277, 4369, 5255, 8905, 16465, 22271]),
            cdf_pad(&[3409, 5436, 10599, 15599, 19687, 24040]),
        ],
        [
            cdf_pad(&[1870, 13742, 14530, 16498, 23770, 27698]),
            cdf_pad(&[326, 8796, 14632, 15079, 19272, 27486]),
            cdf_pad(&[484, 7576, 7712, 14443, 19159, 22591]),
            cdf_pad(&[1126, 15340, 15895, 17023, 20896, 30279]),
            cdf_pad(&[655, 4854, 5249, 5913, 22099, 27138]),
            cdf_pad(&[1299, 6458, 8885, 9290, 14851, 25497]),
            cdf_pad(&[311, 5295, 5552, 6885, 16107, 22672]),
            cdf_pad(&[883, 8059, 8270, 11258, 17289, 21549]),
            cdf_pad(&[741, 7580, 9318, 10345, 16688, 29046]),
            cdf_pad(&[110, 7406, 7915, 9195, 16041, 23329]),
            cdf_pad(&[363, 7974, 9357, 10673, 15629, 24474]),
            cdf_pad(&[153, 7647, 8112, 9936, 15307, 19996]),
            cdf_pad(&[3511, 6332, 11165, 15335, 19323, 23594]),
        ],
        [
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
        ],
        [
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
            cdf_pad(&[4681, 9362, 14043, 18725, 23406, 28087]),
        ],
    ],
    [
        [
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
        ],
        [
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
        ],
        [
            cdf_pad(&[1127, 12814, 22772, 27483]),
            cdf_pad(&[145, 6761, 11980, 26667]),
            cdf_pad(&[362, 5887, 11678, 16725]),
            cdf_pad(&[385, 15213, 18587, 30693]),
            cdf_pad(&[25, 2914, 23134, 27903]),
            cdf_pad(&[60, 4470, 11749, 23991]),
            cdf_pad(&[37, 3332, 14511, 21448]),
            cdf_pad(&[157, 6320, 13036, 17439]),
            cdf_pad(&[119, 6719, 12906, 29396]),
            cdf_pad(&[47, 5537, 12576, 21499]),
            cdf_pad(&[269, 6076, 11258, 23115]),
            cdf_pad(&[83, 5615, 12001, 17228]),
            cdf_pad(&[1968, 5556, 12023, 18547]),
        ],
        [
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
            cdf_pad(&[6554, 13107, 19661, 26214]),
        ],
    ],
];

pub(crate) static DEFAULT_FILTER_INTRA_MODE_CDF: [u16; (FILTER_INTRA_MODES) as usize + 1] =
    cdf_pad(&[8949, 12776, 17211, 29558]);

pub(crate) static DEFAULT_FILTER_INTRA_CDFS: [[u16; (2) as usize + 1]; BLOCK_SIZES_ALL as usize] = [
    cdf_pad(&[4621]),
    cdf_pad(&[6743]),
    cdf_pad(&[5893]),
    cdf_pad(&[7866]),
    cdf_pad(&[12551]),
    cdf_pad(&[9394]),
    cdf_pad(&[12408]),
    cdf_pad(&[14301]),
    cdf_pad(&[12756]),
    cdf_pad(&[22343]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
    cdf_pad(&[12770]),
    cdf_pad(&[10368]),
    cdf_pad(&[20229]),
    cdf_pad(&[18101]),
    cdf_pad(&[16384]),
    cdf_pad(&[16384]),
];

pub(crate) static DEFAULT_SKIP_TXFM_CDFS: [[u16; (2) as usize + 1]; SKIP_CONTEXTS as usize] =
    [cdf_pad(&[31671]), cdf_pad(&[16515]), cdf_pad(&[4576])];

pub(crate) static DEFAULT_TX_SIZE_CDF: [[[u16; (MAX_TX_DEPTH + 1) as usize + 1];
    TX_SIZE_CONTEXTS as usize]; MAX_TX_CATS as usize] = [
    [cdf_pad(&[19968]), cdf_pad(&[19968]), cdf_pad(&[24320])],
    [
        cdf_pad(&[12272, 30172]),
        cdf_pad(&[12272, 30172]),
        cdf_pad(&[18677, 30848]),
    ],
    [
        cdf_pad(&[12986, 15180]),
        cdf_pad(&[12986, 15180]),
        cdf_pad(&[24302, 25602]),
    ],
    [
        cdf_pad(&[5782, 11475]),
        cdf_pad(&[5782, 11475]),
        cdf_pad(&[16803, 22759]),
    ],
];

pub(crate) static DEFAULT_DELTA_Q_CDF: [u16; (DELTA_Q_PROBS + 1) as usize + 1] =
    cdf_pad(&[28160, 32120, 32677]);

pub(crate) static DEFAULT_DELTA_LF_MULTI_CDF: [[u16; (DELTA_LF_PROBS + 1) as usize + 1];
    FRAME_LF_COUNT as usize] = [
    cdf_pad(&[28160, 32120, 32677]),
    cdf_pad(&[28160, 32120, 32677]),
    cdf_pad(&[28160, 32120, 32677]),
    cdf_pad(&[28160, 32120, 32677]),
];

pub(crate) static DEFAULT_DELTA_LF_CDF: [u16; (DELTA_LF_PROBS + 1) as usize + 1] =
    cdf_pad(&[28160, 32120, 32677]);

pub(crate) static DEFAULT_CFL_SIGN_CDF: [u16; (CFL_JOINT_SIGNS) as usize + 1] =
    cdf_pad(&[1418, 2123, 13340, 18405, 26972, 28343, 32294]);

pub(crate) static DEFAULT_CFL_ALPHA_CDF: [[u16; (CFL_ALPHABET_SIZE) as usize + 1];
    CFL_ALPHA_CONTEXTS as usize] = [
    cdf_pad(&[
        7637, 20719, 31401, 32481, 32657, 32688, 32692, 32696, 32700, 32704, 32708, 32712, 32716,
        32720, 32724,
    ]),
    cdf_pad(&[
        14365, 23603, 28135, 31168, 32167, 32395, 32487, 32573, 32620, 32647, 32668, 32672, 32676,
        32680, 32684,
    ]),
    cdf_pad(&[
        11532, 22380, 28445, 31360, 32349, 32523, 32584, 32649, 32673, 32677, 32681, 32685, 32689,
        32693, 32697,
    ]),
    cdf_pad(&[
        26990, 31402, 32282, 32571, 32692, 32696, 32700, 32704, 32708, 32712, 32716, 32720, 32724,
        32728, 32732,
    ]),
    cdf_pad(&[
        17248, 26058, 28904, 30608, 31305, 31877, 32126, 32321, 32394, 32464, 32516, 32560, 32576,
        32593, 32622,
    ]),
    cdf_pad(&[
        14738, 21678, 25779, 27901, 29024, 30302, 30980, 31843, 32144, 32413, 32520, 32594, 32622,
        32656, 32660,
    ]),
];

pub(crate) static DEFAULT_INTRABC_CDF: [u16; (2) as usize + 1] = cdf_pad(&[30531]);
