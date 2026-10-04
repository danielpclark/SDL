// Rust translation of src/video/x11/edid-parse.c and edid.h from Simple
// DirectMedia Layer.
// Copyright 2007 Red Hat, Inc.
// Author: Soren Sandmann <sandmann@redhat.com>
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// on the rights to use, copy, modify, merge, publish, distribute, sub
// license, and/or sell copies of the Software, and to permit persons to whom
// the Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice (including the next
// paragraph) shall be included in all copies or substantial portions of the
// Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NON-INFRINGEMENT.  IN NO EVENT SHALL
// THE AUTHORS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
// IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Decoding monitor EDID blocks (for display names and HDR luminance).

#![allow(dead_code)] // (the decoded fields mirror edid.h; SDL reads a few)

use crate::stdlib::math::pow;

/// Translation of `Interface`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Interface {
    #[default]
    Undefined,
    Dvi,
    HdmiA,
    HdmiB,
    Mddi,
    DisplayPort,
}

/// Translation of `ColorType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum ColorType {
    #[default]
    UndefinedColor,
    Monochrome,
    Rgb,
    OtherColor,
}

/// Translation of `StereoType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum StereoType {
    #[default]
    NoStereo,
    FieldRight,
    FieldLeft,
    TwoWayRightOnEven,
    TwoWayLeftOnEven,
    FourWayInterleaved,
    SideBySide,
}

/// Translation of `Timing`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Timing {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) frequency: i32,
}

/// The sync description of a detailed timing (the `ad` union of
/// `DetailedTiming`, chosen by `digital_sync`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TimingSync {
    Analog {
        bipolar: i32,
        serrations: i32,
        sync_on_green: i32,
    },
    Digital {
        composite: i32,
        serrations: i32,
        negative_vsync: i32,
        negative_hsync: i32,
    },
}

impl Default for TimingSync {
    fn default() -> Self {
        TimingSync::Analog {
            bipolar: 0,
            serrations: 0,
            sync_on_green: 0,
        }
    }
}

/// Translation of `DetailedTiming`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct DetailedTiming {
    pub(crate) pixel_clock: i32,
    pub(crate) h_addr: i32,
    pub(crate) h_blank: i32,
    pub(crate) h_sync: i32,
    pub(crate) h_front_porch: i32,
    pub(crate) v_addr: i32,
    pub(crate) v_blank: i32,
    pub(crate) v_sync: i32,
    pub(crate) v_front_porch: i32,
    pub(crate) width_mm: i32,
    pub(crate) height_mm: i32,
    pub(crate) right_border: i32,
    pub(crate) top_border: i32,
    pub(crate) interlaced: i32,
    pub(crate) stereo: StereoType,

    pub(crate) digital_sync: i32,
    pub(crate) ad: TimingSync,
}

/// The display parameters (the `ad` union of `MonitorInfo`, chosen by
/// `is_digital`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum DisplayParameters {
    Digital {
        bits_per_primary: i32,
        interface: Interface,
        rgb444: i32,
        ycrcb444: i32,
        ycrcb422: i32,
    },
    Analog {
        video_signal_level: f64,
        sync_signal_level: f64,
        total_signal_level: f64,

        blank_to_black: i32,

        separate_hv_sync: i32,
        composite_sync_on_h: i32,
        composite_sync_on_green: i32,
        serration_on_vsync: i32,
        color_type: ColorType,
    },
}

/// Translation of `MonitorInfo`.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct MonitorInfo {
    pub(crate) checksum: i32,
    pub(crate) manufacturer_code: [u8; 4],
    pub(crate) product_code: i32,
    pub(crate) serial_number: u32,

    /// -1 if not specified
    pub(crate) production_week: i32,
    /// -1 if not specified
    pub(crate) production_year: i32,
    /// -1 if not specified
    pub(crate) model_year: i32,

    pub(crate) major_version: i32,
    pub(crate) minor_version: i32,

    pub(crate) is_digital: i32,

    pub(crate) ad: DisplayParameters,

    /// -1 if not specified
    pub(crate) width_mm: i32,
    /// -1 if not specified
    pub(crate) height_mm: i32,
    /// -1.0 if not specififed
    pub(crate) aspect_ratio: f64,

    /// -1.0 if not specified
    pub(crate) gamma: f64,

    pub(crate) standby: i32,
    pub(crate) suspend: i32,
    pub(crate) active_off: i32,

    pub(crate) srgb_is_standard: i32,
    pub(crate) preferred_timing_includes_native: i32,
    pub(crate) continuous_frequency: i32,

    pub(crate) red_x: f64,
    pub(crate) red_y: f64,
    pub(crate) green_x: f64,
    pub(crate) green_y: f64,
    pub(crate) blue_x: f64,
    pub(crate) blue_y: f64,
    pub(crate) white_x: f64,
    pub(crate) white_y: f64,

    pub(crate) min_luminance: f64,
    pub(crate) max_luminance: f64,
    pub(crate) max_frame_average_luminance: f64,

    /// Terminated by 0x0x0
    pub(crate) established: [Timing; 24],
    pub(crate) standard: [Timing; 8],

    pub(crate) n_detailed_timings: i32,
    /// If monitor has a preferred mode, it is the first one (whether it
    /// has, is determined by the preferred_timing_includes bit.
    pub(crate) detailed_timings: [DetailedTiming; 4],

    // Optional product description
    pub(crate) dsc_serial_number: [u8; 14],
    pub(crate) dsc_product_name: [u8; 14],
    /// Unspecified ASCII data
    pub(crate) dsc_string: [u8; 14],
}

impl MonitorInfo {
    /// A zeroed info (`SDL_calloc (1, sizeof (MonitorInfo))`).
    fn zeroed() -> MonitorInfo {
        MonitorInfo {
            checksum: 0,
            manufacturer_code: [0; 4],
            product_code: 0,
            serial_number: 0,
            production_week: 0,
            production_year: 0,
            model_year: 0,
            major_version: 0,
            minor_version: 0,
            is_digital: 0,
            ad: DisplayParameters::Digital {
                bits_per_primary: 0,
                interface: Interface::Undefined,
                rgb444: 0,
                ycrcb444: 0,
                ycrcb422: 0,
            },
            width_mm: 0,
            height_mm: 0,
            aspect_ratio: 0.0,
            gamma: 0.0,
            standby: 0,
            suspend: 0,
            active_off: 0,
            srgb_is_standard: 0,
            preferred_timing_includes_native: 0,
            continuous_frequency: 0,
            red_x: 0.0,
            red_y: 0.0,
            green_x: 0.0,
            green_y: 0.0,
            blue_x: 0.0,
            blue_y: 0.0,
            white_x: 0.0,
            white_y: 0.0,
            min_luminance: 0.0,
            max_luminance: 0.0,
            max_frame_average_luminance: 0.0,
            established: [Timing::default(); 24],
            standard: [Timing::default(); 8],
            n_detailed_timings: 0,
            detailed_timings: [DetailedTiming::default(); 4],
            dsc_serial_number: [0; 14],
            dsc_product_name: [0; 14],
            dsc_string: [0; 14],
        }
    }
}

/// A NUL-terminated byte field as a string.
pub(crate) fn c_field(field: &[u8]) -> String {
    let end = field.iter().position(|&c| c == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Translation of `get_bit()`.
fn get_bit(input: i32, bit: i32) -> i32 {
    (input & (1 << bit)) >> bit
}

/// Translation of `get_bits()`.
fn get_bits(input: i32, begin: i32, end: i32) -> i32 {
    let mask = (1 << (end - begin + 1)) - 1;

    (input >> begin) & mask
}

/// Translation of `decode_header()`.
fn decode_header(edid: &[u8]) -> bool {
    edid[..8] == *b"\x00\xff\xff\xff\xff\xff\xff\x00"
}

/// Translation of `decode_vendor_and_product_identification()`.
fn decode_vendor_and_product_identification(edid: &[u8], info: &mut MonitorInfo) -> bool {
    let e = |i: usize| edid[i] as i32;

    // Manufacturer Code
    let mut code = [0i32; 3];
    code[0] = get_bits(e(0x08), 2, 6);
    code[1] = get_bits(e(0x08), 0, 1) << 3;
    code[1] |= get_bits(e(0x09), 5, 7);
    code[2] = get_bits(e(0x09), 0, 4);
    info.manufacturer_code[3] = b'\0';

    for (out, c) in info.manufacturer_code.iter_mut().zip(code) {
        *out = (c + (b'A' as i32 - 1)) as u8;
    }

    // Product Code
    info.product_code = e(0x0b) << 8 | e(0x0a);

    // Serial Number
    info.serial_number = edid[0x0c] as u32
        | (edid[0x0d] as u32) << 8
        | (edid[0x0e] as u32) << 16
        | (edid[0x0f] as u32) << 24;

    // Week and Year
    let mut is_model_year = false;
    match edid[0x10] {
        0x00 => info.production_week = -1,
        0xff => {
            info.production_week = -1;
            is_model_year = true;
        }
        week => info.production_week = week as i32,
    }

    if is_model_year {
        info.production_year = -1;
        info.model_year = 1990 + e(0x11);
    } else {
        info.production_year = 1990 + e(0x11);
        info.model_year = -1;
    }

    true
}

/// Translation of `decode_edid_version()`.
fn decode_edid_version(edid: &[u8], info: &mut MonitorInfo) -> bool {
    info.major_version = edid[0x12] as i32;
    info.minor_version = edid[0x13] as i32;

    true
}

/// Translation of `decode_display_parameters()`.
fn decode_display_parameters(edid: &[u8], info: &mut MonitorInfo) -> bool {
    let e = |i: usize| edid[i] as i32;

    // Digital vs Analog
    info.is_digital = get_bit(e(0x14), 7);

    if info.is_digital != 0 {
        const BIT_DEPTH: [i32; 8] = [-1, 6, 8, 10, 12, 14, 16, -1];

        const INTERFACES: [Interface; 6] = [
            Interface::Undefined,
            Interface::Dvi,
            Interface::HdmiA,
            Interface::HdmiB,
            Interface::Mddi,
            Interface::DisplayPort,
        ];

        let bits = get_bits(e(0x14), 4, 6);
        let bits_per_primary = BIT_DEPTH[bits as usize];

        let bits = get_bits(e(0x14), 0, 3);

        let interface = if bits <= 5 {
            INTERFACES[bits as usize]
        } else {
            Interface::Undefined
        };
        info.ad = DisplayParameters::Digital {
            bits_per_primary,
            interface,
            rgb444: 0,
            ycrcb444: 0,
            ycrcb422: 0,
        };
    } else {
        let bits = get_bits(e(0x14), 5, 6);

        const LEVELS: [[f64; 3]; 4] = [
            [0.7, 0.3, 1.0],
            [0.714, 0.286, 1.0],
            [1.0, 0.4, 1.4],
            [0.7, 0.0, 0.7],
        ];

        info.ad = DisplayParameters::Analog {
            video_signal_level: LEVELS[bits as usize][0],
            sync_signal_level: LEVELS[bits as usize][1],
            total_signal_level: LEVELS[bits as usize][2],

            blank_to_black: get_bit(e(0x14), 4),

            separate_hv_sync: get_bit(e(0x14), 3),
            composite_sync_on_h: get_bit(e(0x14), 2),
            composite_sync_on_green: get_bit(e(0x14), 1),

            serration_on_vsync: get_bit(e(0x14), 0),
            color_type: ColorType::UndefinedColor,
        };
    }

    // Screen Size / Aspect Ratio
    if edid[0x15] == 0 && edid[0x16] == 0 {
        info.width_mm = -1;
        info.height_mm = -1;
        info.aspect_ratio = -1.0;
    } else if edid[0x16] == 0 {
        info.width_mm = -1;
        info.height_mm = -1;
        info.aspect_ratio = 100.0 / (e(0x15) + 99) as f64;
    } else if edid[0x15] == 0 {
        info.width_mm = -1;
        info.height_mm = -1;
        info.aspect_ratio = 100.0 / (e(0x16) + 99) as f64;
        info.aspect_ratio = 1.0 / info.aspect_ratio; // portrait
    } else {
        info.width_mm = 10 * e(0x15);
        info.height_mm = 10 * e(0x16);
    }

    // Gamma
    if edid[0x17] == 0xFF {
        info.gamma = -1.0;
    } else {
        info.gamma = (e(0x17) as f64 + 100.0) / 100.0;
    }

    // Features
    info.standby = get_bit(e(0x18), 7);
    info.suspend = get_bit(e(0x18), 6);
    info.active_off = get_bit(e(0x18), 5);

    match &mut info.ad {
        DisplayParameters::Digital {
            rgb444,
            ycrcb444,
            ycrcb422,
            ..
        } => {
            *rgb444 = 1;
            if get_bit(e(0x18), 3) != 0 {
                *ycrcb444 = 1;
            }
            if get_bit(e(0x18), 4) != 0 {
                *ycrcb422 = 1;
            }
        }
        DisplayParameters::Analog { color_type, .. } => {
            let bits = get_bits(e(0x18), 3, 4);
            const COLOR_TYPE: [ColorType; 4] = [
                ColorType::Monochrome,
                ColorType::Rgb,
                ColorType::OtherColor,
                ColorType::UndefinedColor,
            ];

            *color_type = COLOR_TYPE[bits as usize];
        }
    }

    info.srgb_is_standard = get_bit(e(0x18), 2);

    // In 1.3 this is called "has preferred timing"
    info.preferred_timing_includes_native = get_bit(e(0x18), 1);

    // FIXME: In 1.3 this indicates whether the monitor accepts GTF
    info.continuous_frequency = get_bit(e(0x18), 0);
    true
}

/// Translation of `decode_fraction()`.
fn decode_fraction(high: i32, low: i32) -> f64 {
    let mut result = 0.0;

    let high = (high << 2) | low;

    for i in 0..10 {
        result += get_bit(high, i) as f64 * pow(2.0, (i - 10) as f64);
    }

    result
}

/// Translation of `decode_color_characteristics()`.
fn decode_color_characteristics(edid: &[u8], info: &mut MonitorInfo) -> bool {
    let e = |i: usize| edid[i] as i32;
    info.red_x = decode_fraction(e(0x1b), get_bits(e(0x19), 6, 7));
    // FIXME (upstream): get_bits(x, 5, 4) is an empty range and always 0
    // (the red y low bits are bits 4..5).
    info.red_y = decode_fraction(e(0x1c), get_bits(e(0x19), 5, 4));
    info.green_x = decode_fraction(e(0x1d), get_bits(e(0x19), 2, 3));
    info.green_y = decode_fraction(e(0x1e), get_bits(e(0x19), 0, 1));
    info.blue_x = decode_fraction(e(0x1f), get_bits(e(0x1a), 6, 7));
    info.blue_y = decode_fraction(e(0x20), get_bits(e(0x1a), 4, 5));
    info.white_x = decode_fraction(e(0x21), get_bits(e(0x1a), 2, 3));
    info.white_y = decode_fraction(e(0x22), get_bits(e(0x1a), 0, 1));

    true
}

/// Translation of `decode_established_timings()`.
fn decode_established_timings(edid: &[u8], info: &mut MonitorInfo) -> bool {
    const fn t(width: i32, height: i32, frequency: i32) -> Timing {
        Timing {
            width,
            height,
            frequency,
        }
    }
    const ESTABLISHED: [[Timing; 8]; 3] = [
        [
            t(800, 600, 60),
            t(800, 600, 56),
            t(640, 480, 75),
            t(640, 480, 72),
            t(640, 480, 67),
            t(640, 480, 60),
            t(720, 400, 88),
            t(720, 400, 70),
        ],
        [
            t(1280, 1024, 75),
            t(1024, 768, 75),
            t(1024, 768, 70),
            t(1024, 768, 60),
            t(1024, 768, 87),
            t(832, 624, 75),
            t(800, 600, 75),
            t(800, 600, 72),
        ],
        [
            t(0, 0, 0),
            t(0, 0, 0),
            t(0, 0, 0),
            t(0, 0, 0),
            t(0, 0, 0),
            t(0, 0, 0),
            t(0, 0, 0),
            t(1152, 870, 75),
        ],
    ];

    let mut idx = 0;
    for (i, row) in ESTABLISHED.iter().enumerate() {
        for (j, timing) in row.iter().enumerate() {
            let byte = edid[0x23 + i] as i32;

            if get_bit(byte, j as i32) != 0 && timing.frequency != 0 {
                info.established[idx] = *timing;
                idx += 1;
            }
        }
    }
    true
}

/// Translation of `decode_standard_timings()`.
fn decode_standard_timings(edid: &[u8], info: &mut MonitorInfo) -> bool {
    for i in 0..8 {
        let first = edid[0x26 + 2 * i] as i32;
        let second = edid[0x27 + 2 * i] as i32;

        if first != 0x01 && second != 0x01 {
            let w = 8 * (first + 31);
            let h = match get_bits(second, 6, 7) {
                0x00 => (w / 16) * 10,
                0x01 => (w / 4) * 3,
                0x02 => (w / 5) * 4,
                _ => (w / 16) * 9,
            };

            info.standard[i].width = w;
            info.standard[i].height = h;
            info.standard[i].frequency = get_bits(second, 0, 5) + 60;
        }
    }

    true
}

/// Translation of `decode_lf_string()`.
fn decode_lf_string(s: &[u8], n_chars: usize, result: &mut [u8; 14]) {
    for i in 0..n_chars {
        if s[i] == 0x0a {
            result[i] = b'\0';
            break;
        } else if s[i] == 0x00 {
            // Convert embedded 0's to spaces
            result[i] = b' ';
        } else {
            result[i] = s[i];
        }
    }
}

/// Translation of `decode_display_descriptor()`.
fn decode_display_descriptor(desc: &[u8], info: &mut MonitorInfo) {
    match desc[0x03] {
        0xFC => decode_lf_string(&desc[5..], 13, &mut info.dsc_product_name),
        0xFF => decode_lf_string(&desc[5..], 13, &mut info.dsc_serial_number),
        0xFE => decode_lf_string(&desc[5..], 13, &mut info.dsc_string),
        0xFD => {} // Range Limits
        0xFB => {} // Color Point
        0xFA => {} // Timing Identifications
        0xF9 => {} // Color Management
        0xF8 => {} // Timing Codes
        0xF7 => {} // Established Timings
        0x10 => {}
        _ => {}
    }
}

/// Translation of `decode_detailed_timing()`.
fn decode_detailed_timing(timing: &[u8], detailed: &mut DetailedTiming) {
    let t = |i: usize| timing[i] as i32;
    const STEREO: [StereoType; 8] = [
        StereoType::NoStereo,
        StereoType::NoStereo,
        StereoType::FieldRight,
        StereoType::FieldLeft,
        StereoType::TwoWayRightOnEven,
        StereoType::TwoWayLeftOnEven,
        StereoType::FourWayInterleaved,
        StereoType::SideBySide,
    ];

    detailed.pixel_clock = (t(0x00) | t(0x01) << 8) * 10000;
    detailed.h_addr = t(0x02) | ((t(0x04) & 0xf0) << 4);
    detailed.h_blank = t(0x03) | ((t(0x04) & 0x0f) << 8);
    detailed.v_addr = t(0x05) | ((t(0x07) & 0xf0) << 4);
    detailed.v_blank = t(0x06) | ((t(0x07) & 0x0f) << 8);
    detailed.h_front_porch = t(0x08) | get_bits(t(0x0b), 6, 7) << 8;
    detailed.h_sync = t(0x09) | get_bits(t(0x0b), 4, 5) << 8;
    detailed.v_front_porch = get_bits(t(0x0a), 4, 7) | get_bits(t(0x0b), 2, 3) << 4;
    detailed.v_sync = get_bits(t(0x0a), 0, 3) | get_bits(t(0x0b), 0, 1) << 4;
    detailed.width_mm = t(0x0c) | get_bits(t(0x0e), 4, 7) << 8;
    detailed.height_mm = t(0x0d) | get_bits(t(0x0e), 0, 3) << 8;
    detailed.right_border = t(0x0f);
    detailed.top_border = t(0x10);

    detailed.interlaced = get_bit(t(0x11), 7);

    // Stereo
    let bits = get_bits(t(0x11), 5, 6) << 1 | get_bit(t(0x11), 0);
    detailed.stereo = STEREO[bits as usize];

    // Sync
    let bits = t(0x11);

    detailed.digital_sync = get_bit(bits, 4);
    if detailed.digital_sync != 0 {
        let composite = (get_bit(bits, 3) == 0) as i32;

        let (serrations, negative_vsync) = if composite != 0 {
            (get_bit(bits, 2), 0)
        } else {
            (0, (get_bit(bits, 2) == 0) as i32)
        };

        detailed.ad = TimingSync::Digital {
            composite,
            serrations,
            negative_vsync,
            negative_hsync: (get_bit(bits, 0) == 0) as i32,
        };
    } else {
        detailed.ad = TimingSync::Analog {
            bipolar: get_bit(bits, 3),
            serrations: get_bit(bits, 2),
            sync_on_green: (get_bit(bits, 1) == 0) as i32,
        };
    }
}

/// Translation of `decode_descriptors()`.
fn decode_descriptors(edid: &[u8], info: &mut MonitorInfo) -> bool {
    let mut timing_idx = 0;

    for i in 0..4 {
        let index = 0x36 + i * 18;

        if edid[index] == 0x00 && edid[index + 1] == 0x00 {
            decode_display_descriptor(&edid[index..], info);
        } else {
            decode_detailed_timing(&edid[index..], &mut info.detailed_timings[timing_idx]);
            timing_idx += 1;
        }
    }

    info.n_detailed_timings = timing_idx as i32;

    true
}

/// Translation of `decode_check_sum()`.
fn decode_check_sum(edid: &[u8], info: &mut MonitorInfo) {
    let mut check: u8 = 0;

    for &b in &edid[..128] {
        check = check.wrapping_add(b);
    }

    info.checksum = check as i32;
}

/// Translation of `decode_HDR_metadata_block()`.
fn decode_hdr_metadata_block(data: &[u8], length: i32, info: &mut MonitorInfo) {
    if length >= 3 {
        info.max_luminance = 50.0 * pow(2.0, data[2] as f64 / 32.0);
    }

    if length >= 4 {
        info.max_frame_average_luminance = 50.0 * pow(2.0, data[3] as f64 / 32.0);
    }

    if length >= 5 {
        info.min_luminance = 50.0 * pow(2.0, data[4] as f64 / 32.0);
    }
}

/// Translation of `decode_cta_block()`.
///
/// `edid` is the rest of the EDID from the block on.
fn decode_cta_block(edid: &[u8], info: &mut MonitorInfo) {
    let mut offset = 4;
    while offset < 128 {
        let length = (edid[offset] & 0x1f) as i32;
        let mut type_ = (edid[offset] >> 5) as i32;
        if length == 0 {
            break;
        }
        // FIXME (upstream): a data block is read without checking that it
        // fits in the 128-byte extension block (here: in the EDID).
        if type_ == 7 {
            type_ <<= 8;
            let Some(&ext) = edid.get(offset + 1) else {
                break;
            };
            type_ |= ext as i32;
        }
        if type_ == 0x706 {
            let start = offset + 2;
            let needed = start + (length - 1).max(0) as usize;
            if needed > edid.len() {
                break;
            }
            decode_hdr_metadata_block(&edid[start..], length - 1, info);
        }
        offset += 1 + length as usize;
    }
}

/// Decode an EDID (a whole number of 128-byte blocks). Translation of
/// `decode_edid()`.
pub(crate) fn decode_edid(edid: &[u8]) -> Option<MonitorInfo> {
    const EDID_PAGE_SIZE: usize = 128;
    let num_blocks = edid.len() / EDID_PAGE_SIZE;

    if !edid.len().is_multiple_of(EDID_PAGE_SIZE) {
        return None;
    }
    // FIXME (upstream): an empty EDID passes the size check and is then
    // read as a 128-byte block.
    if edid.is_empty() {
        return None;
    }

    let mut info = MonitorInfo::zeroed();

    decode_check_sum(edid, &mut info);

    if !decode_header(edid)
        || !decode_vendor_and_product_identification(edid, &mut info)
        || !decode_edid_version(edid, &mut info)
        || !decode_display_parameters(edid, &mut info)
        || !decode_color_characteristics(edid, &mut info)
        || !decode_established_timings(edid, &mut info)
        || !decode_standard_timings(edid, &mut info)
        || !decode_descriptors(edid, &mut info)
    {
        return None;
    }

    if num_blocks > 1 {
        for i in 1..num_blocks {
            let offset = i * EDID_PAGE_SIZE;
            let extension = edid[offset];
            if extension == 0x02 {
                // CTA-861 Extension Block
                decode_cta_block(&edid[offset..], &mut info);
            }
        }
    }

    Some(info)
}

/// Translation of `yesno()`.
fn yesno(v: i32) -> &'static str {
    if v != 0 {
        "yes"
    } else {
        "no"
    }
}

/// The text `dump_monitor_info()` prints (`X11MODES_DEBUG` output).
pub(crate) fn dump_monitor_info(info: &MonitorInfo) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let o = &mut out;

    let _ = writeln!(
        o,
        "Checksum: {} ({})",
        info.checksum,
        if info.checksum != 0 {
            "incorrect"
        } else {
            "correct"
        }
    );
    let _ = writeln!(o, "Manufacturer Code: {}", c_field(&info.manufacturer_code));
    let _ = writeln!(o, "Product Code: 0x{:x}", info.product_code);
    let _ = writeln!(o, "Serial Number: {}", info.serial_number);

    if info.production_week != -1 {
        let _ = writeln!(o, "Production Week: {}", info.production_week);
    } else {
        let _ = writeln!(o, "Production Week: unspecified");
    }

    if info.production_year != -1 {
        let _ = writeln!(o, "Production Year: {}", info.production_year);
    } else {
        let _ = writeln!(o, "Production Year: unspecified");
    }

    if info.model_year != -1 {
        let _ = writeln!(o, "Model Year: {}", info.model_year);
    } else {
        let _ = writeln!(o, "Model Year: unspecified");
    }

    let _ = writeln!(
        o,
        "EDID revision: {}.{}",
        info.major_version, info.minor_version
    );

    let _ = writeln!(
        o,
        "Display is {}",
        if info.is_digital != 0 {
            "digital"
        } else {
            "analog"
        }
    );
    match info.ad {
        DisplayParameters::Digital {
            bits_per_primary,
            interface,
            rgb444,
            ycrcb444,
            ycrcb422,
        } => {
            if bits_per_primary != -1 {
                let _ = writeln!(o, "Bits Per Primary: {bits_per_primary}");
            } else {
                let _ = writeln!(o, "Bits Per Primary: undefined");
            }

            let interface = match interface {
                Interface::Dvi => "DVI",
                Interface::HdmiA => "HDMI-a",
                Interface::HdmiB => "HDMI-b",
                Interface::Mddi => "MDDI",
                Interface::DisplayPort => "DisplayPort",
                Interface::Undefined => "undefined",
            };
            let _ = writeln!(o, "Interface: {interface}");

            let _ = writeln!(o, "RGB 4:4:4: {}", yesno(rgb444));
            let _ = writeln!(o, "YCrCb 4:4:4: {}", yesno(ycrcb444));
            let _ = writeln!(o, "YCrCb 4:2:2: {}", yesno(ycrcb422));
        }
        DisplayParameters::Analog {
            video_signal_level,
            sync_signal_level,
            total_signal_level,
            blank_to_black,
            separate_hv_sync,
            composite_sync_on_h,
            serration_on_vsync,
            color_type,
            ..
        } => {
            let _ = writeln!(o, "Video Signal Level: {video_signal_level:.6}");
            let _ = writeln!(o, "Sync Signal Level: {sync_signal_level:.6}");
            let _ = writeln!(o, "Total Signal Level: {total_signal_level:.6}");

            let _ = writeln!(o, "Blank to Black: {}", yesno(blank_to_black));
            let _ = writeln!(o, "Separate HV Sync: {}", yesno(separate_hv_sync));
            let _ = writeln!(o, "Composite Sync on H: {}", yesno(composite_sync_on_h));
            let _ = writeln!(o, "Serration on VSync: {}", yesno(serration_on_vsync));

            let s = match color_type {
                ColorType::UndefinedColor => "undefined",
                ColorType::Monochrome => "monochrome",
                ColorType::Rgb => "rgb",
                ColorType::OtherColor => "other color",
            };

            let _ = writeln!(o, "Color: {s}");
        }
    }

    if info.width_mm == -1 {
        let _ = writeln!(o, "Width: undefined");
    } else {
        let _ = writeln!(o, "Width: {} mm", info.width_mm);
    }

    if info.height_mm == -1 {
        let _ = writeln!(o, "Height: undefined");
    } else {
        let _ = writeln!(o, "Height: {} mm", info.height_mm);
    }

    if info.aspect_ratio > 0.0 {
        let _ = writeln!(o, "Aspect Ratio: {:.6}", info.aspect_ratio);
    } else {
        let _ = writeln!(o, "Aspect Ratio: undefined");
    }

    if info.gamma >= 0.0 {
        let _ = writeln!(o, "Gamma: {:.6}", info.gamma);
    } else {
        let _ = writeln!(o, "Gamma: undefined");
    }

    let _ = writeln!(o, "Standby: {}", yesno(info.standby));
    let _ = writeln!(o, "Suspend: {}", yesno(info.suspend));
    let _ = writeln!(o, "Active Off: {}", yesno(info.active_off));

    let _ = writeln!(o, "SRGB is Standard: {}", yesno(info.srgb_is_standard));
    let _ = writeln!(
        o,
        "Preferred Timing Includes Native: {}",
        yesno(info.preferred_timing_includes_native)
    );
    let _ = writeln!(
        o,
        "Continuous Frequency: {}",
        yesno(info.continuous_frequency)
    );

    let _ = writeln!(o, "Red X: {:.6}", info.red_x);
    let _ = writeln!(o, "Red Y: {:.6}", info.red_y);
    let _ = writeln!(o, "Green X: {:.6}", info.green_x);
    let _ = writeln!(o, "Green Y: {:.6}", info.green_y);
    let _ = writeln!(o, "Blue X: {:.6}", info.blue_x);
    let _ = writeln!(o, "Blue Y: {:.6}", info.blue_y);
    let _ = writeln!(o, "White X: {:.6}", info.white_x);
    let _ = writeln!(o, "White Y: {:.6}", info.white_y);
    let _ = writeln!(o, "Min luminance: {:.6}", info.min_luminance);
    let _ = writeln!(o, "Max luminance: {:.6}", info.max_luminance);
    let _ = writeln!(
        o,
        "Max frame average luminance {:.6}",
        info.max_frame_average_luminance
    );

    let _ = writeln!(o, "Established Timings:");

    for timing in &info.established {
        if timing.frequency == 0 {
            break;
        }

        let _ = writeln!(
            o,
            "  {} x {} @ {} Hz",
            timing.width, timing.height, timing.frequency
        );
    }

    let _ = writeln!(o, "Standard Timings:");
    for timing in &info.standard {
        if timing.frequency == 0 {
            break;
        }

        let _ = writeln!(
            o,
            "  {} x {} @ {} Hz",
            timing.width, timing.height, timing.frequency
        );
    }

    for (i, timing) in info.detailed_timings[..info.n_detailed_timings as usize]
        .iter()
        .enumerate()
    {
        let _ = writeln!(
            o,
            "Timing{}: ",
            if i == 0 && info.preferred_timing_includes_native != 0 {
                " (Preferred)"
            } else {
                ""
            }
        );
        let _ = writeln!(o, "  Pixel Clock: {}", timing.pixel_clock);
        let _ = writeln!(o, "  H Addressable: {}", timing.h_addr);
        let _ = writeln!(o, "  H Blank: {}", timing.h_blank);
        let _ = writeln!(o, "  H Front Porch: {}", timing.h_front_porch);
        let _ = writeln!(o, "  H Sync: {}", timing.h_sync);
        let _ = writeln!(o, "  V Addressable: {}", timing.v_addr);
        let _ = writeln!(o, "  V Blank: {}", timing.v_blank);
        let _ = writeln!(o, "  V Front Porch: {}", timing.v_front_porch);
        let _ = writeln!(o, "  V Sync: {}", timing.v_sync);
        let _ = writeln!(o, "  Width: {} mm", timing.width_mm);
        let _ = writeln!(o, "  Height: {} mm", timing.height_mm);
        let _ = writeln!(o, "  Right Border: {}", timing.right_border);
        let _ = writeln!(o, "  Top Border: {}", timing.top_border);
        let s = match timing.stereo {
            StereoType::NoStereo => "No Stereo",
            StereoType::FieldRight => "Field Sequential, Right on Sync",
            StereoType::FieldLeft => "Field Sequential, Left on Sync",
            StereoType::TwoWayRightOnEven => "Two-way, Right on Even",
            StereoType::TwoWayLeftOnEven => "Two-way, Left on Even",
            StereoType::FourWayInterleaved => "Four-way Interleaved",
            StereoType::SideBySide => "Side-by-Side",
        };
        let _ = writeln!(o, "  Stereo: {s}");

        match timing.ad {
            TimingSync::Digital {
                composite,
                serrations,
                negative_vsync,
                negative_hsync,
            } => {
                let _ = writeln!(o, "  Digital Sync:");
                let _ = writeln!(o, "    composite: {}", yesno(composite));
                let _ = writeln!(o, "    serrations: {}", yesno(serrations));
                let _ = writeln!(o, "    negative vsync: {}", yesno(negative_vsync));
                let _ = writeln!(o, "    negative hsync: {}", yesno(negative_hsync));
            }
            TimingSync::Analog {
                bipolar,
                serrations,
                sync_on_green,
            } => {
                let _ = writeln!(o, "  Analog Sync:");
                let _ = writeln!(o, "    bipolar: {}", yesno(bipolar));
                let _ = writeln!(o, "    serrations: {}", yesno(serrations));
                let _ = writeln!(o, "    sync on green: {}", yesno(sync_on_green));
            }
        }
    }

    let _ = writeln!(o, "Detailed Product information:");
    let _ = writeln!(o, "  Product Name: {}", c_field(&info.dsc_product_name));
    let _ = writeln!(o, "  Serial Number: {}", c_field(&info.dsc_serial_number));
    let _ = writeln!(o, "  Unspecified String: {}", c_field(&info.dsc_string));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 128-byte EDID 1.4 block of a "DELL U2415" (with a fixed checksum).
    fn sample_edid() -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(b"\x00\xff\xff\xff\xff\xff\xff\x00");
        // "DEL": D=4, E=5, L=12 -> 00100 00101 01100
        e[0x08] = 0x10;
        e[0x09] = 0xAC;
        e[0x0a] = 0x7C;
        e[0x0b] = 0xA0;
        e[0x0c..0x10].copy_from_slice(&[0x4C, 0x30, 0x31, 0x31]);
        e[0x10] = 0x1E; // week 30
        e[0x11] = 0x1A; // 2016
        e[0x12] = 1;
        e[0x13] = 4;
        e[0x14] = 0xA5; // digital, 8 bits, DisplayPort
        e[0x15] = 52;
        e[0x16] = 32;
        e[0x17] = 0x78; // gamma 2.2
        e[0x18] = 0x3A;
        e[0x23] = 0x21; // 800x600@60, 640x480@60
        for i in 0..8 {
            e[0x26 + 2 * i] = 0x01;
            e[0x27 + 2 * i] = 0x01;
        }
        e[0x26] = 0xD1; // 1920 wide
        e[0x27] = 0xC0; // 16:9, 60 Hz
                        // detailed timing 1: 1920x1200
        let dt = [
            0x28, 0x3C, 0x80, 0xA0, 0x70, 0xB0, 0x23, 0x40, 0x30, 0x20, 0x36, 0x00, 0x06, 0x44,
            0x21, 0x00, 0x00, 0x1A,
        ];
        e[0x36..0x36 + 18].copy_from_slice(&dt);
        // display descriptor: product name
        let name = [
            0x00, 0x00, 0x00, 0xFC, 0x00, b'D', b'E', b'L', b'L', b' ', b'U', b'2', b'4', b'1',
            b'5', 0x0A, 0x20, 0x20,
        ];
        e[0x48..0x48 + 18].copy_from_slice(&name);
        let sum: u8 = e[..127].iter().fold(0u8, |a, &b| a.wrapping_add(b));
        e[127] = 0u8.wrapping_sub(sum);
        e
    }

    #[test]
    fn decodes_a_monitor() {
        let info = decode_edid(&sample_edid()).unwrap();
        assert_eq!(info.checksum, 0);
        assert_eq!(c_field(&info.manufacturer_code), "DEL");
        assert_eq!(info.product_code, 0xA07C);
        assert_eq!(info.production_week, 30);
        assert_eq!(info.production_year, 2016);
        assert_eq!((info.major_version, info.minor_version), (1, 4));
        assert_eq!(info.width_mm, 520);
        assert_eq!(info.height_mm, 320);
        assert!((info.gamma - 2.2).abs() < 1e-9);
        assert_eq!(
            info.ad,
            DisplayParameters::Digital {
                bits_per_primary: 8,
                interface: Interface::DisplayPort,
                rgb444: 1,
                ycrcb444: 1,
                ycrcb422: 1,
            }
        );
        assert_eq!(
            info.established[0],
            Timing {
                width: 800,
                height: 600,
                frequency: 60
            }
        );
        assert_eq!(
            info.established[1],
            Timing {
                width: 640,
                height: 480,
                frequency: 60
            }
        );
        assert_eq!(
            info.standard[0],
            Timing {
                width: 1920,
                height: 1080,
                frequency: 60
            }
        );
        assert_eq!(info.n_detailed_timings, 1);
        assert_eq!(info.detailed_timings[0].h_addr, 1920);
        assert_eq!(info.detailed_timings[0].v_addr, 1200);
        assert_eq!(info.detailed_timings[0].pixel_clock, 154_000_000);
        assert_eq!(c_field(&info.dsc_product_name), "DELL U2415");
        let dump = dump_monitor_info(&info);
        assert!(dump.contains("Product Name: DELL U2415"), "{dump}");
        assert!(dump.contains("  1920 x 1080 @ 60 Hz"));
    }

    #[test]
    fn hdr_extension_and_bad_sizes() {
        let mut e = sample_edid();
        e[0x7e] = 1;
        let mut ext = vec![0u8; 128];
        ext[0] = 0x02;
        ext[1] = 0x03;
        // HDR static metadata data block: tag 7, length 6, extended tag 6
        ext[4..11].copy_from_slice(&[0xE6, 0x06, 0x05, 0x01, 0x74, 0x5C, 0x20]);
        e.extend_from_slice(&ext);
        let info = decode_edid(&e).unwrap();
        assert!((info.max_luminance - 50.0 * 2f64.powf(0x74 as f64 / 32.0)).abs() < 1e-9);
        assert!((info.min_luminance - 50.0 * 2f64.powf(0x20 as f64 / 32.0)).abs() < 1e-9);

        assert!(decode_edid(&e[..100]).is_none());
        assert!(decode_edid(&[]).is_none());
        let mut bad = sample_edid();
        bad[0] = 1;
        assert!(decode_edid(&bad).is_none());
    }
}
