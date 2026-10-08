// Rust translation of src/dsp/lossless_enc.c and the encoder's parts of
// src/dsp/lossless.h and lossless_common.h from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2015 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Image transform methods for lossless encoder.
//!
//! The plain-C versions only (`VP8LEncDspInit()` installs them; the SIMD
//! ones give the same results). `VP8LHistogramAdd()` is in
//! `enc/histogram_enc.rs`, next to the histogram it works on.

#![allow(clippy::excessive_precision, clippy::approx_constant)]

use crate::webp::decode::ARGB_BLACK;
use crate::webp::dsp::lossless::vp8l_predictor;
use crate::webp::utils::bits_log2_floor;

/// lookup table for small values of log2(int). Translation of `kLog2Table`.
pub(crate) const K_LOG2_TABLE: [f32; LOG_LOOKUP_IDX_MAX] = [
    0.0000000000000000,
    0.0000000000000000,
    1.0000000000000000,
    1.5849625007211560,
    2.0000000000000000,
    2.3219280948873621,
    2.5849625007211560,
    2.8073549220576041,
    3.0000000000000000,
    3.1699250014423121,
    3.3219280948873621,
    3.4594316186372973,
    3.5849625007211560,
    3.7004397181410921,
    3.8073549220576041,
    3.9068905956085187,
    4.0000000000000000,
    4.0874628412503390,
    4.1699250014423121,
    4.2479275134435852,
    4.3219280948873626,
    4.3923174227787606,
    4.4594316186372973,
    4.5235619560570130,
    4.5849625007211560,
    4.6438561897747243,
    4.7004397181410917,
    4.7548875021634682,
    4.8073549220576037,
    4.8579809951275718,
    4.9068905956085187,
    4.9541963103868749,
    5.0000000000000000,
    5.0443941193584533,
    5.0874628412503390,
    5.1292830169449663,
    5.1699250014423121,
    5.2094533656289501,
    5.2479275134435852,
    5.2854022188622487,
    5.3219280948873626,
    5.3575520046180837,
    5.3923174227787606,
    5.4262647547020979,
    5.4594316186372973,
    5.4918530963296747,
    5.5235619560570130,
    5.5545888516776376,
    5.5849625007211560,
    5.6147098441152083,
    5.6438561897747243,
    5.6724253419714951,
    5.7004397181410917,
    5.7279204545631987,
    5.7548875021634682,
    5.7813597135246599,
    5.8073549220576037,
    5.8328900141647412,
    5.8579809951275718,
    5.8826430493618415,
    5.9068905956085187,
    5.9307373375628866,
    5.9541963103868749,
    5.9772799234999167,
    6.0000000000000000,
    6.0223678130284543,
    6.0443941193584533,
    6.0660891904577720,
    6.0874628412503390,
    6.1085244567781691,
    6.1292830169449663,
    6.1497471195046822,
    6.1699250014423121,
    6.1898245588800175,
    6.2094533656289501,
    6.2288186904958804,
    6.2479275134435852,
    6.2667865406949010,
    6.2854022188622487,
    6.3037807481771030,
    6.3219280948873626,
    6.3398500028846243,
    6.3575520046180837,
    6.3750394313469245,
    6.3923174227787606,
    6.4093909361377017,
    6.4262647547020979,
    6.4429434958487279,
    6.4594316186372973,
    6.4757334309663976,
    6.4918530963296747,
    6.5077946401986963,
    6.5235619560570130,
    6.5391588111080309,
    6.5545888516776376,
    6.5698556083309478,
    6.5849625007211560,
    6.5999128421871278,
    6.6147098441152083,
    6.6293566200796094,
    6.6438561897747243,
    6.6582114827517946,
    6.6724253419714951,
    6.6865005271832185,
    6.7004397181410917,
    6.7142455176661224,
    6.7279204545631987,
    6.7414669864011464,
    6.7548875021634682,
    6.7681843247769259,
    6.7813597135246599,
    6.7944158663501061,
    6.8073549220576037,
    6.8201789624151878,
    6.8328900141647412,
    6.8454900509443747,
    6.8579809951275718,
    6.8703647195834047,
    6.8826430493618415,
    6.8948177633079437,
    6.9068905956085187,
    6.9188632372745946,
    6.9307373375628866,
    6.9425145053392398,
    6.9541963103868749,
    6.9657842846620869,
    6.9772799234999167,
    6.9886846867721654,
    7.0000000000000000,
    7.0112272554232539,
    7.0223678130284543,
    7.0334230015374501,
    7.0443941193584533,
    7.0552824355011898,
    7.0660891904577720,
    7.0768155970508308,
    7.0874628412503390,
    7.0980320829605263,
    7.1085244567781691,
    7.1189410727235076,
    7.1292830169449663,
    7.1395513523987936,
    7.1497471195046822,
    7.1598713367783890,
    7.1699250014423121,
    7.1799090900149344,
    7.1898245588800175,
    7.1996723448363644,
    7.2094533656289501,
    7.2191685204621611,
    7.2288186904958804,
    7.2384047393250785,
    7.2479275134435852,
    7.2573878426926521,
    7.2667865406949010,
    7.2761244052742375,
    7.2854022188622487,
    7.2946207488916270,
    7.3037807481771030,
    7.3128829552843557,
    7.3219280948873626,
    7.3309168781146167,
    7.3398500028846243,
    7.3487281542310771,
    7.3575520046180837,
    7.3663222142458160,
    7.3750394313469245,
    7.3837042924740519,
    7.3923174227787606,
    7.4008794362821843,
    7.4093909361377017,
    7.4178525148858982,
    7.4262647547020979,
    7.4346282276367245,
    7.4429434958487279,
    7.4512111118323289,
    7.4594316186372973,
    7.4676055500829976,
    7.4757334309663976,
    7.4838157772642563,
    7.4918530963296747,
    7.4998458870832056,
    7.5077946401986963,
    7.5156998382840427,
    7.5235619560570130,
    7.5313814605163118,
    7.5391588111080309,
    7.5468944598876364,
    7.5545888516776376,
    7.5622424242210728,
    7.5698556083309478,
    7.5774288280357486,
    7.5849625007211560,
    7.5924570372680806,
    7.5999128421871278,
    7.6073303137496104,
    7.6147098441152083,
    7.6220518194563764,
    7.6293566200796094,
    7.6366246205436487,
    7.6438561897747243,
    7.6510516911789281,
    7.6582114827517946,
    7.6653359171851764,
    7.6724253419714951,
    7.6794800995054464,
    7.6865005271832185,
    7.6934869574993252,
    7.7004397181410917,
    7.7073591320808825,
    7.7142455176661224,
    7.7210991887071855,
    7.7279204545631987,
    7.7347096202258383,
    7.7414669864011464,
    7.7481928495894605,
    7.7548875021634682,
    7.7615512324444795,
    7.7681843247769259,
    7.7747870596011736,
    7.7813597135246599,
    7.7879025593914317,
    7.7944158663501061,
    7.8008998999203047,
    7.8073549220576037,
    7.8137811912170374,
    7.8201789624151878,
    7.8265484872909150,
    7.8328900141647412,
    7.8392037880969436,
    7.8454900509443747,
    7.8517490414160571,
    7.8579809951275718,
    7.8641861446542797,
    7.8703647195834047,
    7.8765169465649993,
    7.8826430493618415,
    7.8887432488982591,
    7.8948177633079437,
    7.9008668079807486,
    7.9068905956085187,
    7.9128893362299619,
    7.9188632372745946,
    7.9248125036057812,
    7.9307373375628866,
    7.9366379390025709,
    7.9425145053392398,
    7.9483672315846778,
    7.9541963103868749,
    7.9600019320680805,
    7.9657842846620869,
    7.9715435539507719,
    7.9772799234999167,
    7.9829935746943103,
    7.9886846867721654,
    7.9943534368588577,
];

/// lookup table for small values of int*log2(int). Translation of
/// `kSLog2Table`.
pub(crate) const K_SLOG2_TABLE: [f32; LOG_LOOKUP_IDX_MAX] = [
    0.00000000,
    0.00000000,
    2.00000000,
    4.75488750,
    8.00000000,
    11.60964047,
    15.50977500,
    19.65148445,
    24.00000000,
    28.52932501,
    33.21928095,
    38.05374781,
    43.01955001,
    48.10571634,
    53.30296891,
    58.60335893,
    64.00000000,
    69.48686830,
    75.05865003,
    80.71062276,
    86.43856190,
    92.23866588,
    98.10749561,
    104.04192499,
    110.03910002,
    116.09640474,
    122.21143267,
    128.38196256,
    134.60593782,
    140.88144886,
    147.20671787,
    153.58008562,
    160.00000000,
    166.46500594,
    172.97373660,
    179.52490559,
    186.11730005,
    192.74977453,
    199.42124551,
    206.13068654,
    212.87712380,
    219.65963219,
    226.47733176,
    233.32938445,
    240.21499122,
    247.13338933,
    254.08384998,
    261.06567603,
    268.07820003,
    275.12078236,
    282.19280949,
    289.29369244,
    296.42286534,
    303.57978409,
    310.76392512,
    317.97478424,
    325.21187564,
    332.47473081,
    339.76289772,
    347.07593991,
    354.41343574,
    361.77497759,
    369.16017124,
    376.56863518,
    384.00000000,
    391.45390785,
    398.93001188,
    406.42797576,
    413.94747321,
    421.48818752,
    429.04981119,
    436.63204548,
    444.23460010,
    451.85719280,
    459.49954906,
    467.16140179,
    474.84249102,
    482.54256363,
    490.26137307,
    497.99867911,
    505.75424759,
    513.52785023,
    521.31926438,
    529.12827280,
    536.95466351,
    544.79822957,
    552.65876890,
    560.53608414,
    568.42998244,
    576.34027536,
    584.26677867,
    592.20931226,
    600.16769996,
    608.14176943,
    616.13135206,
    624.13628279,
    632.15640007,
    640.19154569,
    648.24156472,
    656.30630539,
    664.38561898,
    672.47935976,
    680.58738488,
    688.70955430,
    696.84573069,
    704.99577935,
    713.15956818,
    721.33696754,
    729.52785023,
    737.73209140,
    745.94956849,
    754.18016116,
    762.42375127,
    770.68022275,
    778.94946161,
    787.23135586,
    795.52579543,
    803.83267219,
    812.15187982,
    820.48331383,
    828.82687147,
    837.18245171,
    845.54995518,
    853.92928416,
    862.32034249,
    870.72303558,
    879.13727036,
    887.56295522,
    896.00000000,
    904.44831595,
    912.90781569,
    921.37841320,
    929.86002376,
    938.35256392,
    946.85595152,
    955.37010560,
    963.89494641,
    972.43039537,
    980.97637504,
    989.53280911,
    998.09962237,
    1006.67674069,
    1015.26409097,
    1023.86160116,
    1032.46920021,
    1041.08681805,
    1049.71438560,
    1058.35183469,
    1066.99909811,
    1075.65610955,
    1084.32280357,
    1092.99911564,
    1101.68498204,
    1110.38033993,
    1119.08512727,
    1127.79928282,
    1136.52274614,
    1145.25545758,
    1153.99735821,
    1162.74838989,
    1171.50849518,
    1180.27761738,
    1189.05570047,
    1197.84268914,
    1206.63852876,
    1215.44316535,
    1224.25654560,
    1233.07861684,
    1241.90932703,
    1250.74862473,
    1259.59645914,
    1268.45278005,
    1277.31753781,
    1286.19068338,
    1295.07216828,
    1303.96194457,
    1312.85996488,
    1321.76618236,
    1330.68055071,
    1339.60302413,
    1348.53355734,
    1357.47210556,
    1366.41862452,
    1375.37307041,
    1384.33539991,
    1393.30557020,
    1402.28353887,
    1411.26926400,
    1420.26270412,
    1429.26381818,
    1438.27256558,
    1447.28890615,
    1456.31280014,
    1465.34420819,
    1474.38309138,
    1483.42941118,
    1492.48312945,
    1501.54420843,
    1510.61261078,
    1519.68829949,
    1528.77123795,
    1537.86138993,
    1546.95871952,
    1556.06319119,
    1565.17476976,
    1574.29342040,
    1583.41910860,
    1592.55180020,
    1601.69146137,
    1610.83805860,
    1619.99155871,
    1629.15192882,
    1638.31913637,
    1647.49314911,
    1656.67393509,
    1665.86146266,
    1675.05570047,
    1684.25661744,
    1693.46418280,
    1702.67836605,
    1711.89913698,
    1721.12646563,
    1730.36032233,
    1739.60067768,
    1748.84750254,
    1758.10076802,
    1767.36044551,
    1776.62650662,
    1785.89892323,
    1795.17766747,
    1804.46271172,
    1813.75402857,
    1823.05159087,
    1832.35537170,
    1841.66534438,
    1850.98148244,
    1860.30375965,
    1869.63214999,
    1878.96662767,
    1888.30716711,
    1897.65374295,
    1907.00633003,
    1916.36490342,
    1925.72943838,
    1935.09991037,
    1944.47629506,
    1953.85856831,
    1963.24670620,
    1972.64068498,
    1982.04048108,
    1991.44607117,
    2000.85743204,
    2010.27454072,
    2019.69737440,
    2029.12591044,
    2038.56012640,
];

/// Translation of `VP8LPrefixCode`.
#[derive(Clone, Copy)]
pub(crate) struct VP8LPrefixCode {
    code: i8,
    extra_bits: i8,
}

/// These tables are derived using VP8LPrefixEncodeNoLUT. Translation of
/// `kPrefixEncodeCode`.
const K_PREFIX_ENCODE_CODE: [VP8LPrefixCode; PREFIX_LOOKUP_IDX_MAX] = [
    VP8LPrefixCode {
        code: 0,
        extra_bits: 0,
    },
    VP8LPrefixCode {
        code: 0,
        extra_bits: 0,
    },
    VP8LPrefixCode {
        code: 1,
        extra_bits: 0,
    },
    VP8LPrefixCode {
        code: 2,
        extra_bits: 0,
    },
    VP8LPrefixCode {
        code: 3,
        extra_bits: 0,
    },
    VP8LPrefixCode {
        code: 4,
        extra_bits: 1,
    },
    VP8LPrefixCode {
        code: 4,
        extra_bits: 1,
    },
    VP8LPrefixCode {
        code: 5,
        extra_bits: 1,
    },
    VP8LPrefixCode {
        code: 5,
        extra_bits: 1,
    },
    VP8LPrefixCode {
        code: 6,
        extra_bits: 2,
    },
    VP8LPrefixCode {
        code: 6,
        extra_bits: 2,
    },
    VP8LPrefixCode {
        code: 6,
        extra_bits: 2,
    },
    VP8LPrefixCode {
        code: 6,
        extra_bits: 2,
    },
    VP8LPrefixCode {
        code: 7,
        extra_bits: 2,
    },
    VP8LPrefixCode {
        code: 7,
        extra_bits: 2,
    },
    VP8LPrefixCode {
        code: 7,
        extra_bits: 2,
    },
    VP8LPrefixCode {
        code: 7,
        extra_bits: 2,
    },
    VP8LPrefixCode {
        code: 8,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 8,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 8,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 8,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 8,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 8,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 8,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 8,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 9,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 9,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 9,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 9,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 9,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 9,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 9,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 9,
        extra_bits: 3,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 10,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 11,
        extra_bits: 4,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 12,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 13,
        extra_bits: 5,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 14,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 15,
        extra_bits: 6,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 16,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
    VP8LPrefixCode {
        code: 17,
        extra_bits: 7,
    },
];

/// Translation of `kPrefixEncodeExtraBitsValue`.
const K_PREFIX_ENCODE_EXTRA_BITS_VALUE: [u8; PREFIX_LOOKUP_IDX_MAX] = [
    0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6,
    7, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9,
    10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33,
    34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57,
    58, 59, 60, 61, 62, 63, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
    20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43,
    44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 0, 1, 2, 3, 4,
    5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53,
    54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77,
    78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100,
    101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119,
    120, 121, 122, 123, 124, 125, 126, 127, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39,
    40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63,
    64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87,
    88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108,
    109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126,
];

//------------------------------------------------------------------------------
// lossless_common.h: Misc methods.

/// Computes sampled size of 'size' when sampling using 'sampling bits'.
/// Translation of `VP8LSubSampleSize()`.
pub(crate) fn vp8l_sub_sample_size(size: u32, sampling_bits: u32) -> u32 {
    (size + (1 << sampling_bits) - 1) >> sampling_bits
}

// -----------------------------------------------------------------------------
// Faster logarithm for integers. Small values use a look-up table.

/// The threshold till approximate version of log_2 can be used.
/// Practically, we can get rid of the call to log() as the two values
/// match to very high degree (the ratio of these two is 0.99999x).
/// Keeping a high threshold for now.
const APPROX_LOG_WITH_CORRECTION_MAX: u32 = 65536;
const APPROX_LOG_MAX: u32 = 4096;
const LOG_2_RECIPROCAL: f64 = 1.44269504088896338700465094007086;
pub(crate) const LOG_LOOKUP_IDX_MAX: usize = 256;

/// Translation of `VP8LFastLog2()`.
pub(crate) fn vp8l_fast_log2(v: u32) -> f32 {
    if (v as usize) < LOG_LOOKUP_IDX_MAX {
        K_LOG2_TABLE[v as usize]
    } else {
        fast_log2_slow(v)
    }
}

/// Fast calculation of v * log2(v) for integer input. Translation of
/// `VP8LFastSLog2()`.
pub(crate) fn vp8l_fast_slog2(v: u32) -> f32 {
    if (v as usize) < LOG_LOOKUP_IDX_MAX {
        K_SLOG2_TABLE[v as usize]
    } else {
        fast_slog2_slow(v)
    }
}

// -----------------------------------------------------------------------------
// PrefixEncode()

/// Splitting of distance and length codes into prefixes and
/// extra bits. The prefixes are encoded with an entropy code
/// while the extra bits are stored just as normal bits.
/// Translation of `VP8LPrefixEncodeBitsNoLUT()`: (code, extra_bits).
fn vp8l_prefix_encode_bits_no_lut(mut distance: i32) -> (i32, i32) {
    distance -= 1;
    let highest_bit = bits_log2_floor(distance as u32);
    let second_highest_bit = (distance >> (highest_bit - 1)) & 1;
    (2 * highest_bit + second_highest_bit, highest_bit - 1)
}

/// Translation of `VP8LPrefixEncodeNoLUT()`: (code, extra_bits,
/// extra_bits_value).
fn vp8l_prefix_encode_no_lut(mut distance: i32) -> (i32, i32, i32) {
    distance -= 1;
    let highest_bit = bits_log2_floor(distance as u32);
    let second_highest_bit = (distance >> (highest_bit - 1)) & 1;
    let extra_bits = highest_bit - 1;
    let extra_bits_value = distance & ((1 << extra_bits) - 1);
    (
        2 * highest_bit + second_highest_bit,
        extra_bits,
        extra_bits_value,
    )
}

const PREFIX_LOOKUP_IDX_MAX: usize = 512;

/// Translation of `VP8LPrefixEncodeBits()`: (code, extra_bits).
pub(crate) fn vp8l_prefix_encode_bits(distance: i32) -> (i32, i32) {
    if (distance as usize) < PREFIX_LOOKUP_IDX_MAX {
        let prefix_code = K_PREFIX_ENCODE_CODE[distance as usize];
        (prefix_code.code as i32, prefix_code.extra_bits as i32)
    } else {
        vp8l_prefix_encode_bits_no_lut(distance)
    }
}

/// Translation of `VP8LPrefixEncode()`: (code, extra_bits,
/// extra_bits_value).
pub(crate) fn vp8l_prefix_encode(distance: i32) -> (i32, i32, i32) {
    if (distance as usize) < PREFIX_LOOKUP_IDX_MAX {
        let prefix_code = K_PREFIX_ENCODE_CODE[distance as usize];
        (
            prefix_code.code as i32,
            prefix_code.extra_bits as i32,
            K_PREFIX_ENCODE_EXTRA_BITS_VALUE[distance as usize] as i32,
        )
    } else {
        vp8l_prefix_encode_no_lut(distance)
    }
}

/// Difference of each component, mod 256. Translation of
/// `VP8LSubPixels()`.
pub(crate) fn vp8l_sub_pixels(a: u32, b: u32) -> u32 {
    let alpha_and_green = 0x00ff00ffu32
        .wrapping_add(a & 0xff00ff00)
        .wrapping_sub(b & 0xff00ff00);
    let red_and_blue = 0xff00ff00u32
        .wrapping_add(a & 0x00ff00ff)
        .wrapping_sub(b & 0x00ff00ff);
    (alpha_and_green & 0xff00ff00) | (red_and_blue & 0x00ff00ff)
}

//------------------------------------------------------------------------------

/// Translation of `FastSLog2Slow_C()`.
fn fast_slog2_slow(mut v: u32) -> f32 {
    debug_assert!(v as usize >= LOG_LOOKUP_IDX_MAX);
    if v < APPROX_LOG_WITH_CORRECTION_MAX {
        // use clz if available
        let log_cnt = bits_log2_floor(v) - 7;
        let y = 1u32 << log_cnt;
        let v_f = v as f32;
        let orig_v = v;
        v >>= log_cnt;
        // vf = (2^log_cnt) * Xf; where y = 2^log_cnt and Xf < 256
        // Xf = floor(Xf) * (1 + (v % y) / v)
        // log2(Xf) = log2(floor(Xf)) + log2(1 + (v % y) / v)
        // The correction factor: log(1 + d) ~ d; for very small d values, so
        // log2(1 + (v % y) / v) ~ LOG_2_RECIPROCAL * (v % y)/v
        // LOG_2_RECIPROCAL ~ 23/16
        let correction = ((23 * (orig_v & (y - 1))) >> 4) as i32;
        v_f * (K_LOG2_TABLE[v as usize] + log_cnt as f32) + correction as f32
    } else {
        (LOG_2_RECIPROCAL * v as f64 * (v as f64).ln()) as f32
    }
}

/// Translation of `FastLog2Slow_C()`.
fn fast_log2_slow(mut v: u32) -> f32 {
    debug_assert!(v as usize >= LOG_LOOKUP_IDX_MAX);
    if v < APPROX_LOG_WITH_CORRECTION_MAX {
        // use clz if available
        let log_cnt = bits_log2_floor(v) - 7;
        let y = 1u32 << log_cnt;
        let orig_v = v;
        v >>= log_cnt;
        let mut log_2 = (K_LOG2_TABLE[v as usize] + log_cnt as f32) as f64;
        if orig_v >= APPROX_LOG_MAX {
            // Since the division is still expensive, add this correction factor only
            // for large values of 'v'.
            let correction = ((23 * (orig_v & (y - 1))) >> 4) as i32;
            log_2 += correction as f64 / orig_v as f64;
        }
        log_2 as f32
    } else {
        (LOG_2_RECIPROCAL * (v as f64).ln()) as f32
    }
}

//------------------------------------------------------------------------------
// Methods to calculate Entropy (Shannon).

/// Compute the combined Shanon's entropy for distribution {X} and {X+Y}.
/// Translation of `CombinedShannonEntropy_C()`.
pub(crate) fn vp8l_combined_shannon_entropy(x_arr: &[i32; 256], y_arr: &[i32; 256]) -> f32 {
    let mut retval = 0.0f32;
    let mut sum_x = 0i32;
    let mut sum_xy = 0i32;
    for i in 0..256 {
        let x = x_arr[i];
        if x != 0 {
            let xy = x + y_arr[i];
            sum_x += x;
            retval -= vp8l_fast_slog2(x as u32);
            sum_xy += xy;
            retval -= vp8l_fast_slog2(xy as u32);
        } else if y_arr[i] != 0 {
            sum_xy += y_arr[i];
            retval -= vp8l_fast_slog2(y_arr[i] as u32);
        }
    }
    retval += vp8l_fast_slog2(sum_x as u32) + vp8l_fast_slog2(sum_xy as u32);
    retval
}

/// small struct to hold counters. Translation of `VP8LStreaks`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8LStreaks {
    /// index: 0=zero streak, 1=non-zero streak
    pub(crate) counts: [i32; 2],
    /// [zero/non-zero][streak<3 / streak>=3]
    pub(crate) streaks: [[i32; 2]; 2],
}

/// small struct to hold bit entropy results. Translation of
/// `VP8LBitEntropy`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8LBitEntropy {
    /// entropy
    pub(crate) entropy: f32,
    /// sum of the population
    pub(crate) sum: u32,
    /// number of non-zero elements in the population
    pub(crate) nonzeros: i32,
    /// maximum value in the population
    pub(crate) max_val: u32,
    /// index of the last non-zero in the population
    pub(crate) nonzero_code: u32,
}

/// Not a trivial literal symbol. Translation of `VP8L_NON_TRIVIAL_SYM`.
pub(crate) const VP8L_NON_TRIVIAL_SYM: u32 = 0xffffffff;

/// Translation of `VP8LBitEntropyInit()`.
pub(crate) fn vp8l_bit_entropy_init(entropy: &mut VP8LBitEntropy) {
    entropy.entropy = 0.0;
    entropy.sum = 0;
    entropy.nonzeros = 0;
    entropy.max_val = 0;
    entropy.nonzero_code = VP8L_NON_TRIVIAL_SYM;
}

/// Get the entropy for the distribution 'X'. Translation of
/// `VP8LBitsEntropyUnrefined()`.
pub(crate) fn vp8l_bits_entropy_unrefined(array: &[u32], n: usize, entropy: &mut VP8LBitEntropy) {
    vp8l_bit_entropy_init(entropy);

    for (i, &a) in array[..n].iter().enumerate() {
        if a != 0 {
            entropy.sum = entropy.sum.wrapping_add(a);
            entropy.nonzero_code = i as u32;
            entropy.nonzeros += 1;
            entropy.entropy -= vp8l_fast_slog2(a);
            if entropy.max_val < a {
                entropy.max_val = a;
            }
        }
    }
    entropy.entropy += vp8l_fast_slog2(entropy.sum);
}

/// Translation of `GetEntropyUnrefinedHelper()`.
fn get_entropy_unrefined_helper(
    val: u32,
    i: i32,
    val_prev: &mut u32,
    i_prev: &mut i32,
    bit_entropy: &mut VP8LBitEntropy,
    stats: &mut VP8LStreaks,
) {
    let streak = i - *i_prev;

    // Gather info for the bit entropy.
    if *val_prev != 0 {
        bit_entropy.sum = bit_entropy
            .sum
            .wrapping_add((*val_prev).wrapping_mul(streak as u32));
        bit_entropy.nonzeros += streak;
        bit_entropy.nonzero_code = *i_prev as u32;
        bit_entropy.entropy -= vp8l_fast_slog2(*val_prev) * streak as f32;
        if bit_entropy.max_val < *val_prev {
            bit_entropy.max_val = *val_prev;
        }
    }

    // Gather info for the Huffman cost.
    let nz = (*val_prev != 0) as usize;
    stats.counts[nz] += (streak > 3) as i32;
    stats.streaks[nz][(streak > 3) as usize] += streak;

    *val_prev = val;
    *i_prev = i;
}

/// Get the entropy for the distribution 'X'. Translation of
/// `GetEntropyUnrefined_C()`.
pub(crate) fn vp8l_get_entropy_unrefined(
    x_arr: &[u32],
    length: usize,
    bit_entropy: &mut VP8LBitEntropy,
    stats: &mut VP8LStreaks,
) {
    let mut i_prev = 0;
    let mut x_prev = x_arr[0];

    *stats = VP8LStreaks::default();
    vp8l_bit_entropy_init(bit_entropy);

    let mut i = 1;
    while i < length {
        let x = x_arr[i];
        if x != x_prev {
            get_entropy_unrefined_helper(x, i as i32, &mut x_prev, &mut i_prev, bit_entropy, stats);
        }
        i += 1;
    }
    get_entropy_unrefined_helper(0, i as i32, &mut x_prev, &mut i_prev, bit_entropy, stats);

    bit_entropy.entropy += vp8l_fast_slog2(bit_entropy.sum);
}

/// Get the combined symbol bit entropy and Huffman cost stats for the
/// distributions 'X' and 'Y'. Translation of
/// `GetCombinedEntropyUnrefined_C()`.
pub(crate) fn vp8l_get_combined_entropy_unrefined(
    x_arr: &[u32],
    y_arr: &[u32],
    length: usize,
    bit_entropy: &mut VP8LBitEntropy,
    stats: &mut VP8LStreaks,
) {
    let mut i_prev = 0;
    let mut xy_prev = x_arr[0].wrapping_add(y_arr[0]);

    *stats = VP8LStreaks::default();
    vp8l_bit_entropy_init(bit_entropy);

    let mut i = 1;
    while i < length {
        let xy = x_arr[i].wrapping_add(y_arr[i]);
        if xy != xy_prev {
            get_entropy_unrefined_helper(
                xy,
                i as i32,
                &mut xy_prev,
                &mut i_prev,
                bit_entropy,
                stats,
            );
        }
        i += 1;
    }
    get_entropy_unrefined_helper(0, i as i32, &mut xy_prev, &mut i_prev, bit_entropy, stats);

    bit_entropy.entropy += vp8l_fast_slog2(bit_entropy.sum);
}

//------------------------------------------------------------------------------

/// Translation of `VP8LSubtractGreenFromBlueAndRed_C()`.
pub(crate) fn vp8l_subtract_green_from_blue_and_red(argb_data: &mut [u32]) {
    for p in argb_data {
        let argb = *p as i32;
        let green = (argb >> 8) & 0xff;
        let new_r = ((((argb >> 16) & 0xff) - green) & 0xff) as u32;
        let new_b = (((argb & 0xff) - green) & 0xff) as u32;
        *p = (argb as u32 & 0xff00ff00) | (new_r << 16) | new_b;
    }
}

/// Translation of `ColorTransformDelta()`.
fn color_transform_delta(color_pred: i8, color: i8) -> i32 {
    (color_pred as i32 * color as i32) >> 5
}

/// Translation of `U32ToS8()`.
fn u32_to_s8(v: u32) -> i8 {
    (v & 0xff) as u8 as i8
}

/// Translation of `VP8LMultipliers`.
#[derive(Clone, Copy, Default)]
pub(crate) struct VP8LMultipliers {
    // Note: the members are uint8_t, so that any negative values are
    // automatically converted to "mod 256" values.
    pub(crate) green_to_red: u8,
    pub(crate) green_to_blue: u8,
    pub(crate) red_to_blue: u8,
}

/// Translation of `VP8LTransformColor_C()`.
pub(crate) fn vp8l_transform_color(m: &VP8LMultipliers, data: &mut [u32]) {
    for p in data {
        let argb = *p;
        let green = u32_to_s8(argb >> 8);
        let red = u32_to_s8(argb >> 16);
        let mut new_red = red as i32 & 0xff;
        let mut new_blue = (argb & 0xff) as i32;
        new_red -= color_transform_delta(m.green_to_red as i8, green);
        new_red &= 0xff;
        new_blue -= color_transform_delta(m.green_to_blue as i8, green);
        new_blue -= color_transform_delta(m.red_to_blue as i8, red);
        new_blue &= 0xff;
        *p = (argb & 0xff00ff00) | ((new_red as u32) << 16) | new_blue as u32;
    }
}

/// Translation of `TransformColorRed()`.
fn transform_color_red(green_to_red: u8, argb: u32) -> u8 {
    let green = u32_to_s8(argb >> 8);
    let mut new_red = (argb >> 16) as i32;
    new_red -= color_transform_delta(green_to_red as i8, green);
    (new_red & 0xff) as u8
}

/// Translation of `TransformColorBlue()`.
fn transform_color_blue(green_to_blue: u8, red_to_blue: u8, argb: u32) -> u8 {
    let green = u32_to_s8(argb >> 8);
    let red = u32_to_s8(argb >> 16);
    let mut new_blue = (argb & 0xff) as i32;
    new_blue -= color_transform_delta(green_to_blue as i8, green);
    new_blue -= color_transform_delta(red_to_blue as i8, red);
    (new_blue & 0xff) as u8
}

/// Translation of `VP8LCollectColorRedTransforms_C()`.
pub(crate) fn vp8l_collect_color_red_transforms(
    argb: &[u32],
    stride: usize,
    tile_width: usize,
    tile_height: usize,
    green_to_red: i32,
    histo: &mut [i32; 256],
) {
    for y in 0..tile_height {
        for &p in &argb[y * stride..y * stride + tile_width] {
            histo[transform_color_red(green_to_red as u8, p) as usize] += 1;
        }
    }
}

/// Translation of `VP8LCollectColorBlueTransforms_C()`.
pub(crate) fn vp8l_collect_color_blue_transforms(
    argb: &[u32],
    stride: usize,
    tile_width: usize,
    tile_height: usize,
    green_to_blue: i32,
    red_to_blue: i32,
    histo: &mut [i32; 256],
) {
    for y in 0..tile_height {
        for &p in &argb[y * stride..y * stride + tile_width] {
            histo[transform_color_blue(green_to_blue as u8, red_to_blue as u8, p) as usize] += 1;
        }
    }
}

//------------------------------------------------------------------------------

/// Translation of `VectorMismatch_C()`: the length of the common prefix of
/// `array1` and `array2`, up to `length`.
pub(crate) fn vp8l_vector_mismatch(array1: &[u32], array2: &[u32], length: usize) -> usize {
    let mut match_len = 0;

    while match_len < length && array1[match_len] == array2[match_len] {
        match_len += 1;
    }
    match_len
}

/// Bundles multiple (1, 2, 4 or 8) pixels into a single pixel.
/// Translation of `VP8LBundleColorMap_C()`.
pub(crate) fn vp8l_bundle_color_map(row: &[u8], width: usize, xbits: i32, dst: &mut [u32]) {
    if xbits > 0 {
        let bit_depth = 1 << (3 - xbits);
        let mask = (1 << xbits) - 1;
        let mut code = 0xff000000u32;
        for x in 0..width {
            let xsub = x as i32 & mask;
            if xsub == 0 {
                code = 0xff000000;
            }
            code |= (row[x] as u32) << (8 + bit_depth * xsub);
            dst[x >> xbits] = code;
        }
    } else {
        for x in 0..width {
            dst[x] = 0xff000000 | ((row[x] as u32) << 8);
        }
    }
}

//------------------------------------------------------------------------------

/// Translation of `ExtraCost_C()`.
pub(crate) fn vp8l_extra_cost(population: &[u32], length: usize) -> f32 {
    let mut cost = 0.0f32;
    for i in 2..length - 2 {
        cost += ((i >> 1) as u32).wrapping_mul(population[i + 2]) as f32;
    }
    cost
}

/// Translation of `ExtraCostCombined_C()`.
pub(crate) fn vp8l_extra_cost_combined(x_arr: &[u32], y_arr: &[u32], length: usize) -> f32 {
    let mut cost = 0.0f32;
    for i in 2..length - 2 {
        let xy = x_arr[i + 2].wrapping_add(y_arr[i + 2]) as i32;
        cost += ((i >> 1) as i32).wrapping_mul(xy) as f32;
    }
    cost
}

//------------------------------------------------------------------------------
// Image transforms.

/// The batch predictors `PredictorSub0_C()` to `PredictorSub13_C()`
/// (`VP8LPredictorsSub[mode]`): the residuals of `num_pixels` pixels at
/// `input` in `buf`, predicted from the row at `upper` in `buf`, to `out`.
/// It subtracts the prediction from the input pixel and stores the
/// residual in the output pixel.
pub(crate) fn vp8l_predictors_sub(
    mode: u32,
    buf: &[u32],
    input: usize,
    upper: usize,
    num_pixels: usize,
    out: &mut [u32],
) {
    for (x, o) in out[..num_pixels].iter_mut().enumerate() {
        let i = input + x;
        let pred = match mode {
            0 => ARGB_BLACK,
            1 => buf[i - 1],
            _ => {
                // (each predictor reads only the neighbours it uses: the
                // left and top-left ones may be before the buffer for the
                // top one)
                let u = upper + x;
                let left = if matches!(mode, 5 | 6 | 7 | 10..=13) {
                    buf[i - 1]
                } else {
                    0
                };
                let tl = if matches!(mode, 4 | 6 | 8 | 10..=13) {
                    buf[u - 1]
                } else {
                    0
                };
                let tr = if matches!(mode, 3 | 5 | 9 | 10) {
                    buf[u + 1]
                } else {
                    0
                };
                vp8l_predictor(mode, left, tl, buf[u], tr)
            }
        };
        *o = vp8l_sub_pixels(buf[i], pred);
    }
}

/// The predictors `VP8LPredictors[mode]` of the decoder: the prediction of
/// the pixel after `left`, from the row above at `top`.
pub(crate) fn vp8l_predictors(mode: u32, left: u32, upper: &[u32], top: usize) -> u32 {
    vp8l_predictor(mode, left, upper[top - 1], upper[top], upper[top + 1])
}
