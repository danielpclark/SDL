// Rust translation of src/enc/picture_csp_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2014 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! WebPPicture utils for colorspace conversion: RGB(A) import, the
//! RGB(A) to YUV(A) conversion (with gamma-corrected chroma averaging)
//! and the YUV(A) to ARGB conversion.
//!
//! The samples are read from byte rows (`rgb`, with the channels at their
//! offsets) as upstream reads them through byte pointers; an ARGB picture
//! is read as its pixels' little-endian bytes (B, G, R, A), the order
//! upstream's `CHANNEL_OFFSET()` gives on a little-endian machine (on a
//! big-endian one it reads the same channels). The sharp (iterative)
//! conversion, the dithering and the 3-byte-per-pixel fast path are not
//! translated: SDL_image converts RGBA pixels without them.

use std::sync::OnceLock;

use crate::webp::decode::WebpCspMode;
use crate::webp::dsp::alpha_processing::{
    webp_extract_alpha, webp_has_alpha_32b, webp_has_alpha_8b,
};
use crate::webp::dsp::upsampling::{webp_upsample, LinePair};
use crate::webp::dsp::yuv::{vp8_rgb_to_y, webp_convert_rgba32_to_uv, YUV_HALF};
use crate::webp::enc::picture_enc::{
    webp_encoding_set_error, webp_picture_alloc, webp_picture_alloc_argb, webp_picture_alloc_yuva,
};
use crate::webp::encode::{
    WebPEncodingError, WebPPicture, WEBP_CSP_ALPHA_BIT, WEBP_CSP_UV_MASK, WEBP_YUV420, WEBP_YUV420A,
};

/// The bytes of an ARGB picture's pixels, as upstream reads them through a
/// byte pointer: B, G, R, A for each pixel.
fn argb_bytes(argb: &[u32]) -> Vec<u8> {
    argb.iter().flat_map(|p| p.to_le_bytes()).collect()
}

// uint32_t 0xff000000 is 0x00,00,00,ff in memory
/// Translation of `CHANNEL_OFFSET()` (on the little-endian bytes).
const fn channel_offset(i: usize) -> usize {
    3 - i
}

const ALPHA_OFFSET: usize = channel_offset(0);

//------------------------------------------------------------------------------
// Detection of non-trivial transparency

/// Returns true if alpha[] has non-0xff values. Translation of
/// `CheckNonOpaque()` (`alpha` from `off`, if any).
fn check_non_opaque(
    alpha: Option<(&[u8], usize)>,
    width: usize,
    height: usize,
    x_step: usize,
    y_step: usize,
) -> bool {
    let Some((alpha, mut off)) = alpha else {
        return false;
    };
    for _ in 0..height {
        if x_step == 1 {
            if webp_has_alpha_8b(&alpha[off..], width) {
                return true;
            }
        } else if webp_has_alpha_32b(&alpha[off..], width) {
            return true;
        }
        off += y_step;
    }
    false
}

/// Checking for the presence of non-opaque alpha. Translation of
/// `WebPPictureHasTransparency()`.
pub(crate) fn webp_picture_has_transparency(picture: &WebPPicture) -> bool {
    if picture.use_argb {
        if !picture.argb.is_empty() {
            let bytes = argb_bytes(&picture.argb);
            return check_non_opaque(
                Some((&bytes, ALPHA_OFFSET)),
                picture.width as usize,
                picture.height as usize,
                4,
                picture.argb_stride as usize * 4,
            );
        }
        return false;
    }
    check_non_opaque(
        (!picture.a.is_empty()).then_some((&picture.a[..], 0)),
        picture.width as usize,
        picture.height as usize,
        1,
        picture.a_stride as usize,
    )
}

//------------------------------------------------------------------------------
// Code for gamma correction

// Gamma correction compensates loss of resolution during chroma subsampling.
/// fixed-point precision for linear values
const GAMMA_FIX: i32 = 12;
/// fixed-point fractional bits precision
const GAMMA_TAB_FIX: i32 = 7;
const GAMMA_TAB_SIZE: usize = 1 << (GAMMA_FIX - GAMMA_TAB_FIX);
const K_GAMMA: f64 = 0.80;
const K_GAMMA_SCALE: i32 = (1 << GAMMA_FIX) - 1;
const K_GAMMA_TAB_SCALE: i32 = 1 << GAMMA_TAB_FIX;
const K_GAMMA_TAB_ROUNDER: i32 = (1 << GAMMA_TAB_FIX) >> 1;

/// Translation of `kLinearToGammaTab` and `kGammaToLinearTab`.
struct GammaTables {
    linear_to_gamma: [i32; GAMMA_TAB_SIZE + 1],
    gamma_to_linear: [u16; 256],
}

/// The tables, computed once. Translation of `InitGammaTables()`.
fn gamma_tables() -> &'static GammaTables {
    static TABLES: OnceLock<GammaTables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut t = GammaTables {
            linear_to_gamma: [0; GAMMA_TAB_SIZE + 1],
            gamma_to_linear: [0; 256],
        };
        let scale = (1 << GAMMA_TAB_FIX) as f64 / K_GAMMA_SCALE as f64;
        let norm = 1.0 / 255.0;
        for v in 0..=255 {
            t.gamma_to_linear[v] =
                ((norm * v as f64).powf(K_GAMMA) * K_GAMMA_SCALE as f64 + 0.5) as u16;
        }
        for v in 0..=GAMMA_TAB_SIZE {
            t.linear_to_gamma[v] = (255.0 * (scale * v as f64).powf(1.0 / K_GAMMA) + 0.5) as i32;
        }
        t
    })
}

/// Translation of `GammaToLinear()`.
fn gamma_to_linear(v: u8) -> u32 {
    gamma_tables().gamma_to_linear[v as usize] as u32
}

/// Translation of `Interpolate()`.
fn interpolate(v: i32) -> i32 {
    let tab = &gamma_tables().linear_to_gamma;
    let tab_pos = (v >> (GAMMA_TAB_FIX + 2)) as usize; // integer part
    let x = v & ((K_GAMMA_TAB_SCALE << 2) - 1); // fractional part
    let v0 = tab[tab_pos];
    let v1 = tab[tab_pos + 1];
    let y = v1 * x + v0 * ((K_GAMMA_TAB_SCALE << 2) - x); // interpolate
    debug_assert!(tab_pos + 1 < GAMMA_TAB_SIZE + 1);
    y
}

/// Convert a linear value 'v' to YUV_FIX+2 fixed-point precision
/// U/V value, suitable for RGBToU/V calls. Translation of
/// `LinearToGamma()`.
fn linear_to_gamma(base_value: u32, shift: i32) -> i32 {
    let y = interpolate((base_value << shift) as i32); // final uplifted value
    (y + K_GAMMA_TAB_ROUNDER) >> GAMMA_TAB_FIX // descale
}

//------------------------------------------------------------------------------
// RGB -> YUV conversion

/// Translation of `RGBToY()` (without the dithering generator).
fn rgb_to_y(r: i32, g: i32, b: i32) -> i32 {
    vp8_rgb_to_y(r, g, b, YUV_HALF)
}

//------------------------------------------------------------------------------
// "Fast" regular RGB->YUV

/// Translation of `SUM4()`.
fn sum4(rgb: &[u8], ptr: usize, step: usize, rgb_stride: usize) -> u16 {
    linear_to_gamma(
        gamma_to_linear(rgb[ptr])
            + gamma_to_linear(rgb[ptr + step])
            + gamma_to_linear(rgb[ptr + rgb_stride])
            + gamma_to_linear(rgb[ptr + rgb_stride + step]),
        0,
    ) as u16
}

/// Translation of `SUM2()`.
fn sum2(rgb: &[u8], ptr: usize, rgb_stride: usize) -> u16 {
    linear_to_gamma(
        gamma_to_linear(rgb[ptr]) + gamma_to_linear(rgb[ptr + rgb_stride]),
        1,
    ) as u16
}

/// Translation of `SUM2ALPHA()`.
fn sum2_alpha(rgb: &[u8], ptr: usize, rgb_stride: usize) -> u32 {
    rgb[ptr] as u32 + rgb[ptr + rgb_stride] as u32
}

/// Translation of `SUM4ALPHA()`.
fn sum4_alpha(rgb: &[u8], ptr: usize, rgb_stride: usize) -> u32 {
    sum2_alpha(rgb, ptr, rgb_stride) + sum2_alpha(rgb, ptr + 4, rgb_stride)
}

const K_ALPHA_FIX: i32 = 19;
/// Following table is (1 << kAlphaFix) / a. The (v * kInvAlpha[a]) >>
/// kAlphaFix formula is then equal to v / a in most (99.6%) cases. Note
/// that this table and constant are adjusted very tightly to fit 32b
/// arithmetic. In particular, they use the fact that the operands for
/// 'v / a' are actually derived as v = (a0.p0 + a1.p1 + a2.p2 + a3.p3) and
/// a = a0 + a1 + a2 + a3 with ai in [0..255] and pi in [0..1<<GAMMA_FIX).
/// The constraint to avoid overflow is: GAMMA_FIX + kAlphaFix <= 31.
const K_INV_ALPHA: [u32; 4 * 0xff + 1] = [
    0, 524288, 262144, 174762, 131072, 104857, 87381, 74898, 65536, 58254, 52428, 47662, 43690,
    40329, 37449, 34952, 32768, 30840, 29127, 27594, 26214, 24966, 23831, 22795, 21845, 20971,
    20164, 19418, 18724, 18078, 17476, 16912, 16384, 15887, 15420, 14979, 14563, 14169, 13797,
    13443, 13107, 12787, 12483, 12192, 11915, 11650, 11397, 11155, 10922, 10699, 10485, 10280,
    10082, 9892, 9709, 9532, 9362, 9198, 9039, 8886, 8738, 8594, 8456, 8322, 8192, 8065, 7943,
    7825, 7710, 7598, 7489, 7384, 7281, 7182, 7084, 6990, 6898, 6808, 6721, 6636, 6553, 6472, 6393,
    6316, 6241, 6168, 6096, 6026, 5957, 5890, 5825, 5761, 5698, 5637, 5577, 5518, 5461, 5405, 5349,
    5295, 5242, 5190, 5140, 5090, 5041, 4993, 4946, 4899, 4854, 4809, 4766, 4723, 4681, 4639, 4599,
    4559, 4519, 4481, 4443, 4405, 4369, 4332, 4297, 4262, 4228, 4194, 4161, 4128, 4096, 4064, 4032,
    4002, 3971, 3942, 3912, 3883, 3855, 3826, 3799, 3771, 3744, 3718, 3692, 3666, 3640, 3615, 3591,
    3566, 3542, 3518, 3495, 3472, 3449, 3426, 3404, 3382, 3360, 3339, 3318, 3297, 3276, 3256, 3236,
    3216, 3196, 3177, 3158, 3139, 3120, 3102, 3084, 3066, 3048, 3030, 3013, 2995, 2978, 2962, 2945,
    2928, 2912, 2896, 2880, 2864, 2849, 2833, 2818, 2803, 2788, 2774, 2759, 2744, 2730, 2716, 2702,
    2688, 2674, 2661, 2647, 2634, 2621, 2608, 2595, 2582, 2570, 2557, 2545, 2532, 2520, 2508, 2496,
    2484, 2473, 2461, 2449, 2438, 2427, 2416, 2404, 2394, 2383, 2372, 2361, 2351, 2340, 2330, 2319,
    2309, 2299, 2289, 2279, 2269, 2259, 2250, 2240, 2231, 2221, 2212, 2202, 2193, 2184, 2175, 2166,
    2157, 2148, 2139, 2131, 2122, 2114, 2105, 2097, 2088, 2080, 2072, 2064, 2056, 2048, 2040, 2032,
    2024, 2016, 2008, 2001, 1993, 1985, 1978, 1971, 1963, 1956, 1949, 1941, 1934, 1927, 1920, 1913,
    1906, 1899, 1892, 1885, 1879, 1872, 1865, 1859, 1852, 1846, 1839, 1833, 1826, 1820, 1814, 1807,
    1801, 1795, 1789, 1783, 1777, 1771, 1765, 1759, 1753, 1747, 1741, 1736, 1730, 1724, 1718, 1713,
    1707, 1702, 1696, 1691, 1685, 1680, 1675, 1669, 1664, 1659, 1653, 1648, 1643, 1638, 1633, 1628,
    1623, 1618, 1613, 1608, 1603, 1598, 1593, 1588, 1583, 1579, 1574, 1569, 1565, 1560, 1555, 1551,
    1546, 1542, 1537, 1533, 1528, 1524, 1519, 1515, 1510, 1506, 1502, 1497, 1493, 1489, 1485, 1481,
    1476, 1472, 1468, 1464, 1460, 1456, 1452, 1448, 1444, 1440, 1436, 1432, 1428, 1424, 1420, 1416,
    1413, 1409, 1405, 1401, 1398, 1394, 1390, 1387, 1383, 1379, 1376, 1372, 1368, 1365, 1361, 1358,
    1354, 1351, 1347, 1344, 1340, 1337, 1334, 1330, 1327, 1323, 1320, 1317, 1314, 1310, 1307, 1304,
    1300, 1297, 1294, 1291, 1288, 1285, 1281, 1278, 1275, 1272, 1269, 1266, 1263, 1260, 1257, 1254,
    1251, 1248, 1245, 1242, 1239, 1236, 1233, 1230, 1227, 1224, 1222, 1219, 1216, 1213, 1210, 1208,
    1205, 1202, 1199, 1197, 1194, 1191, 1188, 1186, 1183, 1180, 1178, 1175, 1172, 1170, 1167, 1165,
    1162, 1159, 1157, 1154, 1152, 1149, 1147, 1144, 1142, 1139, 1137, 1134, 1132, 1129, 1127, 1125,
    1122, 1120, 1117, 1115, 1113, 1110, 1108, 1106, 1103, 1101, 1099, 1096, 1094, 1092, 1089, 1087,
    1085, 1083, 1081, 1078, 1076, 1074, 1072, 1069, 1067, 1065, 1063, 1061, 1059, 1057, 1054, 1052,
    1050, 1048, 1046, 1044, 1042, 1040, 1038, 1036, 1034, 1032, 1030, 1028, 1026, 1024, 1022, 1020,
    1018, 1016, 1014, 1012, 1010, 1008, 1006, 1004, 1002, 1000, 998, 996, 994, 992, 991, 989, 987,
    985, 983, 981, 979, 978, 976, 974, 972, 970, 969, 967, 965, 963, 961, 960, 958, 956, 954, 953,
    951, 949, 948, 946, 944, 942, 941, 939, 937, 936, 934, 932, 931, 929, 927, 926, 924, 923, 921,
    919, 918, 916, 914, 913, 911, 910, 908, 907, 905, 903, 902, 900, 899, 897, 896, 894, 893, 891,
    890, 888, 887, 885, 884, 882, 881, 879, 878, 876, 875, 873, 872, 870, 869, 868, 866, 865, 863,
    862, 860, 859, 858, 856, 855, 853, 852, 851, 849, 848, 846, 845, 844, 842, 841, 840, 838, 837,
    836, 834, 833, 832, 830, 829, 828, 826, 825, 824, 823, 821, 820, 819, 817, 816, 815, 814, 812,
    811, 810, 809, 807, 806, 805, 804, 802, 801, 800, 799, 798, 796, 795, 794, 793, 791, 790, 789,
    788, 787, 786, 784, 783, 782, 781, 780, 779, 777, 776, 775, 774, 773, 772, 771, 769, 768, 767,
    766, 765, 764, 763, 762, 760, 759, 758, 757, 756, 755, 754, 753, 752, 751, 750, 748, 747, 746,
    745, 744, 743, 742, 741, 740, 739, 738, 737, 736, 735, 734, 733, 732, 731, 730, 729, 728, 727,
    726, 725, 724, 723, 722, 721, 720, 719, 718, 717, 716, 715, 714, 713, 712, 711, 710, 709, 708,
    707, 706, 705, 704, 703, 702, 701, 700, 699, 699, 698, 697, 696, 695, 694, 693, 692, 691, 690,
    689, 688, 688, 687, 686, 685, 684, 683, 682, 681, 680, 680, 679, 678, 677, 676, 675, 674, 673,
    673, 672, 671, 670, 669, 668, 667, 667, 666, 665, 664, 663, 662, 661, 661, 660, 659, 658, 657,
    657, 656, 655, 654, 653, 652, 652, 651, 650, 649, 648, 648, 647, 646, 645, 644, 644, 643, 642,
    641, 640, 640, 639, 638, 637, 637, 636, 635, 634, 633, 633, 632, 631, 630, 630, 629, 628, 627,
    627, 626, 625, 624, 624, 623, 622, 621, 621, 620, 619, 618, 618, 617, 616, 616, 615, 614, 613,
    613, 612, 611, 611, 610, 609, 608, 608, 607, 606, 606, 605, 604, 604, 603, 602, 601, 601, 600,
    599, 599, 598, 597, 597, 596, 595, 595, 594, 593, 593, 592, 591, 591, 590, 589, 589, 588, 587,
    587, 586, 585, 585, 584, 583, 583, 582, 581, 581, 580, 579, 579, 578, 578, 577, 576, 576, 575,
    574, 574, 573, 572, 572, 571, 571, 570, 569, 569, 568, 568, 567, 566, 566, 565, 564, 564, 563,
    563, 562, 561, 561, 560, 560, 559, 558, 558, 557, 557, 556, 555, 555, 554, 554, 553, 553, 552,
    551, 551, 550, 550, 549, 548, 548, 547, 547, 546, 546, 545, 544, 544, 543, 543, 542, 542, 541,
    541, 540, 539, 539, 538, 538, 537, 537, 536, 536, 535, 534, 534, 533, 533, 532, 532, 531, 531,
    530, 530, 529, 529, 528, 527, 527, 526, 526, 525, 525, 524, 524, 523, 523, 522, 522, 521, 521,
    520, 520, 519, 519, 518, 518, 517, 517, 516, 516, 515, 515, 514, 514,
];

/// Note that LinearToGamma() expects the values to be premultiplied by 4,
/// so we incorporate this factor 4 inside the DIVIDE_BY_ALPHA macro
/// directly. Translation of `DIVIDE_BY_ALPHA()`.
fn divide_by_alpha(sum: u32, a: u32) -> u32 {
    sum.wrapping_mul(K_INV_ALPHA[a as usize]) >> (K_ALPHA_FIX - 2)
}

/// Translation of `LinearToGammaWeighted()`: the samples at `src` and the
/// alpha values at `a_ptr` are both in `rgb`.
fn linear_to_gamma_weighted(
    rgb: &[u8],
    src: usize,
    a_ptr: usize,
    total_a: u32,
    step: usize,
    rgb_stride: usize,
) -> i32 {
    let sum = rgb[a_ptr] as u32 * gamma_to_linear(rgb[src])
        + rgb[a_ptr + step] as u32 * gamma_to_linear(rgb[src + step])
        + rgb[a_ptr + rgb_stride] as u32 * gamma_to_linear(rgb[src + rgb_stride])
        + rgb[a_ptr + rgb_stride + step] as u32 * gamma_to_linear(rgb[src + rgb_stride + step]);
    debug_assert!(total_a > 0 && total_a <= 4 * 0xff);
    debug_assert!((sum as u64 * K_INV_ALPHA[total_a as usize] as u64) < (1u64 << 32));
    linear_to_gamma(divide_by_alpha(sum, total_a), 0)
}

/// Translation of `ConvertRowToY()` (without the dithering generator).
fn convert_row_to_y(
    rgb: &[u8],
    r_ptr: usize,
    g_ptr: usize,
    b_ptr: usize,
    step: usize,
    dst_y: &mut [u8],
    width: usize,
) {
    let mut j = 0;
    for d in dst_y[..width].iter_mut() {
        *d = rgb_to_y(
            rgb[r_ptr + j] as i32,
            rgb[g_ptr + j] as i32,
            rgb[b_ptr + j] as i32,
        ) as u8;
        j += step;
    }
}

/// Translation of `AccumulateRGBA()`.
#[allow(clippy::too_many_arguments)]
fn accumulate_rgba(
    rgb: &[u8],
    r_ptr: usize,
    g_ptr: usize,
    b_ptr: usize,
    a_ptr: usize,
    rgb_stride: usize,
    dst: &mut [u16],
    width: usize,
) {
    let mut j = 0;
    let mut d = 0;
    // we loop over 2x2 blocks and produce one R/G/B/A value for each.
    for _ in 0..(width >> 1) {
        let a = sum4_alpha(rgb, a_ptr + j, rgb_stride);
        let (r, g, b);
        if a == 4 * 0xff || a == 0 {
            r = sum4(rgb, r_ptr + j, 4, rgb_stride) as i32;
            g = sum4(rgb, g_ptr + j, 4, rgb_stride) as i32;
            b = sum4(rgb, b_ptr + j, 4, rgb_stride) as i32;
        } else {
            r = linear_to_gamma_weighted(rgb, r_ptr + j, a_ptr + j, a, 4, rgb_stride);
            g = linear_to_gamma_weighted(rgb, g_ptr + j, a_ptr + j, a, 4, rgb_stride);
            b = linear_to_gamma_weighted(rgb, b_ptr + j, a_ptr + j, a, 4, rgb_stride);
        }
        dst[d] = r as u16;
        dst[d + 1] = g as u16;
        dst[d + 2] = b as u16;
        dst[d + 3] = a as u16;
        j += 2 * 4;
        d += 4;
    }
    if width & 1 != 0 {
        let a = 2 * sum2_alpha(rgb, a_ptr + j, rgb_stride);
        let (r, g, b);
        if a == 4 * 0xff || a == 0 {
            r = sum2(rgb, r_ptr + j, rgb_stride) as i32;
            g = sum2(rgb, g_ptr + j, rgb_stride) as i32;
            b = sum2(rgb, b_ptr + j, rgb_stride) as i32;
        } else {
            r = linear_to_gamma_weighted(rgb, r_ptr + j, a_ptr + j, a, 0, rgb_stride);
            g = linear_to_gamma_weighted(rgb, g_ptr + j, a_ptr + j, a, 0, rgb_stride);
            b = linear_to_gamma_weighted(rgb, b_ptr + j, a_ptr + j, a, 0, rgb_stride);
        }
        dst[d] = r as u16;
        dst[d + 1] = g as u16;
        dst[d + 2] = b as u16;
        dst[d + 3] = a as u16;
    }
}

/// Translation of `AccumulateRGB()`.
#[allow(clippy::too_many_arguments)]
fn accumulate_rgb(
    rgb: &[u8],
    r_ptr: usize,
    g_ptr: usize,
    b_ptr: usize,
    step: usize,
    rgb_stride: usize,
    dst: &mut [u16],
    width: usize,
) {
    let mut j = 0;
    let mut d = 0;
    for _ in 0..(width >> 1) {
        dst[d] = sum4(rgb, r_ptr + j, step, rgb_stride);
        dst[d + 1] = sum4(rgb, g_ptr + j, step, rgb_stride);
        dst[d + 2] = sum4(rgb, b_ptr + j, step, rgb_stride);
        j += 2 * step;
        d += 4;
    }
    if width & 1 != 0 {
        dst[d] = sum2(rgb, r_ptr + j, rgb_stride);
        dst[d + 1] = sum2(rgb, g_ptr + j, rgb_stride);
        dst[d + 2] = sum2(rgb, b_ptr + j, rgb_stride);
    }
}

/// Translation of `ImportYUVAFromRGBA()` (without the dithering, the
/// iterative conversion and the 3-byte fast path): the samples are the
/// bytes of `rgb` at the channels' offsets.
#[allow(clippy::too_many_arguments)]
fn import_yuva_from_rgba(
    rgb: &[u8],
    mut r_ptr: usize,
    mut g_ptr: usize,
    mut b_ptr: usize,
    mut a_ptr: Option<usize>,
    step: usize,       // bytes per pixel
    rgb_stride: usize, // bytes per scanline
    dithering: f32,
    use_iterative_conversion: bool,
    picture: &mut WebPPicture,
) -> bool {
    let width = picture.width as usize;
    let height = picture.height as usize;
    let has_alpha = check_non_opaque(a_ptr.map(|a| (rgb, a)), width, height, step, rgb_stride);
    debug_assert!(dithering <= 0.0, "the dithering is not translated");
    debug_assert!(step == 4, "the 3-byte fast path is not translated");
    debug_assert!(
        !use_iterative_conversion,
        "the sharp conversion is not translated"
    );

    picture.colorspace = if has_alpha { WEBP_YUV420A } else { WEBP_YUV420 };
    picture.use_argb = false;

    if !webp_picture_alloc_yuva(picture) {
        return false;
    }

    {
        let uv_width = (width + 1) >> 1;
        // temporary storage for accumulated R/G/B values during conversion to U/V
        let mut tmp_rgb = vec![0u16; 4 * uv_width];
        let mut dst_y = 0usize;
        let mut dst_u = 0usize;
        let mut dst_v = 0usize;
        let mut dst_a = 0usize;
        let y_stride = picture.y_stride as usize;
        let uv_stride = picture.uv_stride as usize;
        let a_stride = picture.a_stride as usize;

        // Downsample Y/U/V planes, two rows at a time
        for _ in 0..(height >> 1) {
            let mut rows_have_alpha = has_alpha;
            convert_row_to_y(
                rgb,
                r_ptr,
                g_ptr,
                b_ptr,
                step,
                &mut picture.y[dst_y..],
                width,
            );
            convert_row_to_y(
                rgb,
                r_ptr + rgb_stride,
                g_ptr + rgb_stride,
                b_ptr + rgb_stride,
                step,
                &mut picture.y[dst_y + y_stride..],
                width,
            );
            dst_y += 2 * y_stride;
            if has_alpha {
                rows_have_alpha &= !webp_extract_alpha(
                    rgb,
                    a_ptr.unwrap(),
                    rgb_stride,
                    width,
                    2,
                    &mut picture.a,
                    dst_a,
                    a_stride,
                );
                dst_a += 2 * a_stride;
            }
            // Collect averaged R/G/B(/A)
            if !rows_have_alpha {
                accumulate_rgb(
                    rgb,
                    r_ptr,
                    g_ptr,
                    b_ptr,
                    step,
                    rgb_stride,
                    &mut tmp_rgb,
                    width,
                );
            } else {
                accumulate_rgba(
                    rgb,
                    r_ptr,
                    g_ptr,
                    b_ptr,
                    a_ptr.unwrap(),
                    rgb_stride,
                    &mut tmp_rgb,
                    width,
                );
            }
            // Convert to U/V
            webp_convert_rgba32_to_uv(
                &tmp_rgb,
                &mut picture.u[dst_u..],
                &mut picture.v[dst_v..],
                uv_width,
            );
            dst_u += uv_stride;
            dst_v += uv_stride;
            r_ptr += 2 * rgb_stride;
            b_ptr += 2 * rgb_stride;
            g_ptr += 2 * rgb_stride;
            if has_alpha {
                a_ptr = a_ptr.map(|a| a + 2 * rgb_stride);
            }
        }
        if height & 1 != 0 {
            // extra last row
            let mut row_has_alpha = has_alpha;
            convert_row_to_y(
                rgb,
                r_ptr,
                g_ptr,
                b_ptr,
                step,
                &mut picture.y[dst_y..],
                width,
            );
            if row_has_alpha {
                row_has_alpha &=
                    !webp_extract_alpha(rgb, a_ptr.unwrap(), 0, width, 1, &mut picture.a, dst_a, 0);
            }
            // Collect averaged R/G/B(/A)
            if !row_has_alpha {
                // Collect averaged R/G/B
                accumulate_rgb(
                    rgb,
                    r_ptr,
                    g_ptr,
                    b_ptr,
                    step,
                    /* rgb_stride = */ 0,
                    &mut tmp_rgb,
                    width,
                );
            } else {
                accumulate_rgba(
                    rgb,
                    r_ptr,
                    g_ptr,
                    b_ptr,
                    a_ptr.unwrap(),
                    /* rgb_stride = */ 0,
                    &mut tmp_rgb,
                    width,
                );
            }
            webp_convert_rgba32_to_uv(
                &tmp_rgb,
                &mut picture.u[dst_u..],
                &mut picture.v[dst_v..],
                uv_width,
            );
        }
    }
    true
}

//------------------------------------------------------------------------------
// call for ARGB->YUVA conversion

/// Translation of `PictureARGBToYUVA()`.
fn picture_argb_to_yuva(
    picture: &mut WebPPicture,
    colorspace: i32,
    dithering: f32,
    use_iterative_conversion: bool,
) -> bool {
    if picture.argb.is_empty() {
        webp_encoding_set_error(picture, WebPEncodingError::NullParameter)
    } else if (colorspace & WEBP_CSP_UV_MASK) != WEBP_YUV420 {
        webp_encoding_set_error(picture, WebPEncodingError::InvalidConfiguration)
    } else {
        let argb = argb_bytes(&picture.argb);
        let a = channel_offset(0);
        let r = channel_offset(1);
        let g = channel_offset(2);
        let b = channel_offset(3);

        picture.colorspace = WEBP_YUV420;
        let stride = 4 * picture.argb_stride as usize;
        import_yuva_from_rgba(
            &argb,
            r,
            g,
            b,
            Some(a),
            4,
            stride,
            dithering,
            use_iterative_conversion,
            picture,
        )
    }
}

/// Same as WebPPictureARGBToYUVA(), but the conversion is done using
/// pseudo-random dithering with a strength 'dithering' between 0.0 (no
/// dithering) and 1.0 (maximum dithering). This is useful for
/// photographic picture. Translation of `WebPPictureARGBToYUVADithered()`.
pub(crate) fn webp_picture_argb_to_yuva_dithered(
    picture: &mut WebPPicture,
    colorspace: i32,
    dithering: f32,
) -> bool {
    picture_argb_to_yuva(picture, colorspace, dithering, false)
}

//------------------------------------------------------------------------------
// call for YUVA -> ARGB conversion

/// Converts picture->yuv to picture->argb and sets picture->use_argb to
/// true. The input format must be YUV_420 or YUV_420A. The conversion from
/// YUV420 to ARGB incurs a small loss too. Note that the use of this
/// colorspace is discouraged if one has access to the raw ARGB samples,
/// since using YUV420 is comparatively lossy. Returns false in case of
/// error. Translation of `WebPPictureYUVAToARGB()`.
pub(crate) fn webp_picture_yuva_to_argb(picture: &mut WebPPicture) -> bool {
    if picture.y.is_empty() || picture.u.is_empty() || picture.v.is_empty() {
        return webp_encoding_set_error(picture, WebPEncodingError::NullParameter);
    }
    if (picture.colorspace & WEBP_CSP_ALPHA_BIT) != 0 && picture.a.is_empty() {
        return webp_encoding_set_error(picture, WebPEncodingError::NullParameter);
    }
    if (picture.colorspace & WEBP_CSP_UV_MASK) != WEBP_YUV420 {
        return webp_encoding_set_error(picture, WebPEncodingError::InvalidConfiguration);
    }
    // Allocate a new argb buffer (discarding the previous one).
    if !webp_picture_alloc_argb(picture) {
        return false;
    }
    picture.use_argb = true;

    // Convert
    {
        let width = picture.width as usize;
        let height = picture.height as usize;
        let argb_stride = 4 * picture.argb_stride as usize;
        let y_stride = picture.y_stride as usize;
        let uv_stride = picture.uv_stride as usize;
        // (the BGRA bytes of the ARGB pixels, little-endian)
        let mut dst_bytes = vec![0u8; argb_stride * height];
        let mut dst = 0usize;
        let mut cur_u = 0usize;
        let mut cur_v = 0usize;
        let mut cur_y = 0usize;
        // WebPGetLinePairConverter(ALPHA_OFFSET > 0): the fancy BGRA upsampler.
        let mode = WebpCspMode::Bgra;
        let (yp, up, vp) = (&picture.y, &picture.u, &picture.v);

        // First row, with replicated top samples.
        webp_upsample(
            mode,
            &LinePair {
                top_y: &yp[cur_y..],
                bottom_y: None,
                top_u: &up[cur_u..],
                top_v: &vp[cur_v..],
                cur_u: &up[cur_u..],
                cur_v: &vp[cur_v..],
                top_dst: dst,
                bottom_dst: None,
            },
            &mut dst_bytes,
            width as i32,
        );
        cur_y += y_stride;
        dst += argb_stride;
        // Center rows.
        let mut y = 1;
        while y + 1 < height {
            let top_u = cur_u;
            let top_v = cur_v;
            cur_u += uv_stride;
            cur_v += uv_stride;
            webp_upsample(
                mode,
                &LinePair {
                    top_y: &yp[cur_y..],
                    bottom_y: Some(&yp[cur_y + y_stride..]),
                    top_u: &up[top_u..],
                    top_v: &vp[top_v..],
                    cur_u: &up[cur_u..],
                    cur_v: &vp[cur_v..],
                    top_dst: dst,
                    bottom_dst: Some(dst + argb_stride),
                },
                &mut dst_bytes,
                width as i32,
            );
            cur_y += 2 * y_stride;
            dst += 2 * argb_stride;
            y += 2;
        }
        // Last row (if needed), with replicated bottom samples.
        if height > 1 && (height & 1) == 0 {
            webp_upsample(
                mode,
                &LinePair {
                    top_y: &yp[cur_y..],
                    bottom_y: None,
                    top_u: &up[cur_u..],
                    top_v: &vp[cur_v..],
                    cur_u: &up[cur_u..],
                    cur_v: &vp[cur_v..],
                    top_dst: dst,
                    bottom_dst: None,
                },
                &mut dst_bytes,
                width as i32,
            );
        }
        for (p, b) in picture.argb.iter_mut().zip(dst_bytes.chunks_exact(4)) {
            *p = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        }
        // Insert alpha values if needed, in replacement for the default 0xff ones.
        if picture.colorspace & WEBP_CSP_ALPHA_BIT != 0 {
            let stride = picture.argb_stride as usize;
            let a_stride = picture.a_stride as usize;
            for y in 0..height {
                for x in 0..width {
                    let argb_dst = &mut picture.argb[y * stride + x];
                    *argb_dst =
                        (*argb_dst & 0x00ffffff) | ((picture.a[y * a_stride + x] as u32) << 24);
                }
            }
        }
    }
    true
}

//------------------------------------------------------------------------------
// automatic import / conversion

/// Translation of `Import()`.
fn import(
    picture: &mut WebPPicture,
    rgb: &[u8],
    rgb_stride: usize,
    step: usize,
    swap_rb: bool,
    import_alpha: bool,
) -> bool {
    // swap_rb -> b,g,r,a , !swap_rb -> r,g,b,a
    let r_ptr = if swap_rb { 2 } else { 0 };
    let g_ptr = 1;
    let b_ptr = if swap_rb { 0 } else { 2 };
    let width = picture.width as usize;
    let height = picture.height as usize;

    if rgb_stride < (if import_alpha { 4 } else { 3 }) * width {
        return false;
    }

    if !picture.use_argb {
        let a_ptr = import_alpha.then_some(3);
        return import_yuva_from_rgba(
            rgb, r_ptr, g_ptr, b_ptr, a_ptr, step, rgb_stride, 0.0, /* no dithering */
            false, picture,
        );
    }
    if !webp_picture_alloc(picture) {
        return false;
    }

    let dst_stride = picture.argb_stride as usize;
    for y in 0..height {
        let row = &rgb[y * rgb_stride..];
        let dst = &mut picture.argb[y * dst_stride..y * dst_stride + width];
        for (x, d) in dst.iter_mut().enumerate() {
            let p = &row[x * step..];
            // (VP8LConvertBGRAToRGBA() of the RGBA bytes, or WebPPackRGB())
            let a = if import_alpha { p[3] } else { 0xff };
            *d = ((a as u32) << 24)
                | ((p[r_ptr] as u32) << 16)
                | ((p[g_ptr] as u32) << 8)
                | p[b_ptr] as u32;
        }
    }
    true
}

/// Colorspace conversion function to import RGBA samples. Previous buffer
/// will be free'd, if any. *rgba buffer should have a size of at least
/// height * rgba_stride. Returns false in case of memory error.
/// Translation of `WebPPictureImportRGBA()`.
pub(crate) fn webp_picture_import_rgba(
    picture: &mut WebPPicture,
    rgba: &[u8],
    rgba_stride: usize,
) -> bool {
    import(picture, rgba, rgba_stride, 4, false, true)
}
