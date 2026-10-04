// Tests for the report descriptor parser.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The expected values come from upstream's SDL_report_descriptor.c,
//! compiled from C with stubs for the SDL functions it uses.

use super::*;

/// FNV-1a over the fields (report ID, usage, bit offset, bit size).
fn fields_hash(descriptor: &ReportDescriptor) -> (usize, u32) {
    let mut h: u32 = 2166136261;
    for field in &descriptor.fields {
        for v in [
            u32::from(field.report_id),
            field.usage,
            field.bit_offset as u32,
            field.bit_size as u32,
        ] {
            h ^= v;
            h = h.wrapping_mul(16777619);
        }
    }
    (descriptor.fields.len(), h)
}

macro_rules! descriptor {
    ($name:literal) => {
        include_bytes!(concat!("../../../hidapi/testdata/", $name))
    };
}

#[test]
fn xbox_one_descriptor_fields() {
    // (the reconstructed descriptor of an Xbox One controller on Windows)
    let descriptor = parse_report_descriptor(descriptor!("045E_02FF_0005_0001.rpt_desc")).unwrap();
    let expected: Option<&[(u8, u32, i32, i32)]> = Some(&[
        (0, 0x00010030, 0, 16),
        (0, 0x00010031, 16, 16),
        (0, 0x00010033, 32, 16),
        (0, 0x00010034, 48, 16),
        (0, 0x00010032, 64, 16),
        (0, 0x00090001, 80, 1),
        (0, 0x00090002, 81, 1),
        (0, 0x00090003, 82, 1),
        (0, 0x00090004, 83, 1),
        (0, 0x00090005, 84, 1),
        (0, 0x00090006, 85, 1),
        (0, 0x00090007, 86, 1),
        (0, 0x00090008, 87, 1),
        (0, 0x00090009, 88, 1),
        (0, 0x0009000a, 89, 1),
        (0, 0x0009000b, 90, 1),
        (0, 0x0009000c, 91, 1),
        (0, 0x0009000d, 92, 1),
        (0, 0x0009000e, 93, 1),
        (0, 0x0009000f, 94, 1),
        (0, 0x00090010, 95, 1),
        (0, 0x00010039, 96, 4),
    ]);
    let fields: Vec<(u8, u32, i32, i32)> = descriptor
        .fields
        .iter()
        .map(|f| (f.report_id, f.usage, f.bit_offset, f.bit_size))
        .collect();
    assert_eq!(Some(fields.as_slice()), expected);
    assert!(descriptor.has_usage(0x0001, 0x0030));
    assert!(descriptor.has_usage(0x0009, 0x0010));
    assert!(!descriptor.has_usage(0x0009, 0x0011));
}

#[test]
fn real_descriptors() {
    type Case = (&'static str, &'static [u8], Option<(usize, u32)>);
    const CASES: [Case; 24] = [
        (
            "045E_02FF_0005_0001.rpt_desc",
            descriptor!("045E_02FF_0005_0001.rpt_desc"),
            Some((22, 0x876bc9e2)),
        ),
        (
            "046A_0011_0006_0001.rpt_desc",
            descriptor!("046A_0011_0006_0001.rpt_desc"),
            Some((8, 0x473ac84d)),
        ),
        (
            "046D_0A37_0001_000C.rpt_desc",
            descriptor!("046D_0A37_0001_000C.rpt_desc"),
            Some((84, 0x2d2ba2a2)),
        ),
        (
            "046D_B010_0001_000C.rpt_desc",
            descriptor!("046D_B010_0001_000C.rpt_desc"),
            Some((1, 0x6055d75e)),
        ),
        (
            "046D_B010_0001_FF00.rpt_desc",
            descriptor!("046D_B010_0001_FF00.rpt_desc"),
            Some((6, 0xeb2ab5dd)),
        ),
        (
            "046D_B010_0002_0001.rpt_desc",
            descriptor!("046D_B010_0002_0001.rpt_desc"),
            Some((12, 0x3a7d9f2e)),
        ),
        (
            "046D_B010_0002_FF00.rpt_desc",
            descriptor!("046D_B010_0002_FF00.rpt_desc"),
            Some((19, 0x96777856)),
        ),
        (
            "046D_B010_0006_0001.rpt_desc",
            descriptor!("046D_B010_0006_0001.rpt_desc"),
            Some((8, 0x1181ba6d)),
        ),
        (
            "046D_C077_0002_0001.rpt_desc",
            descriptor!("046D_C077_0002_0001.rpt_desc"),
            Some((11, 0x48699cb7)),
        ),
        (
            "046D_C283_0004_0001.rpt_desc",
            descriptor!("046D_C283_0004_0001.rpt_desc"),
            Some((14, 0x67309f40)),
        ),
        (
            "046D_C52F_0001_000C.rpt_desc",
            descriptor!("046D_C52F_0001_000C.rpt_desc"),
            Some((2, 0xf74b0f8c)),
        ),
        (
            "046D_C52F_0001_FF00.rpt_desc",
            descriptor!("046D_C52F_0001_FF00.rpt_desc"),
            Some((6, 0xeb2ab5dd)),
        ),
        (
            "046D_C52F_0002_0001.rpt_desc",
            descriptor!("046D_C52F_0002_0001.rpt_desc"),
            Some((20, 0x72441022)),
        ),
        (
            "046D_C52F_0002_FF00.rpt_desc",
            descriptor!("046D_C52F_0002_FF00.rpt_desc"),
            Some((19, 0x96777856)),
        ),
        (
            "046D_C534_0001_000C.rpt_desc",
            descriptor!("046D_C534_0001_000C.rpt_desc"),
            Some((2, 0xf74b0f8c)),
        ),
        (
            "046D_C534_0001_FF00.rpt_desc",
            descriptor!("046D_C534_0001_FF00.rpt_desc"),
            Some((6, 0xeb2ab5dd)),
        ),
        (
            "046D_C534_0002_0001.rpt_desc",
            descriptor!("046D_C534_0002_0001.rpt_desc"),
            Some((20, 0x8b5490ae)),
        ),
        (
            "046D_C534_0002_FF00.rpt_desc",
            descriptor!("046D_C534_0002_FF00.rpt_desc"),
            Some((19, 0x96777856)),
        ),
        (
            "046D_C534_0006_0001.rpt_desc",
            descriptor!("046D_C534_0006_0001.rpt_desc"),
            Some((8, 0x473ac84d)),
        ),
        (
            "046D_C534_0080_0001.rpt_desc",
            descriptor!("046D_C534_0080_0001.rpt_desc"),
            Some((1, 0xcf08efe1)),
        ),
        (
            "047F_C056_0001_000C.rpt_desc",
            descriptor!("047F_C056_0001_000C.rpt_desc"),
            Some((82, 0x89f57801)),
        ),
        (
            "047F_C056_0003_FFA0.rpt_desc",
            descriptor!("047F_C056_0003_FFA0.rpt_desc"),
            Some((39, 0x6af0b785)),
        ),
        (
            "047F_C056_0005_000B.rpt_desc",
            descriptor!("047F_C056_0005_000B.rpt_desc"),
            Some((3, 0xb46de32b)),
        ),
        (
            "17CC_1130_0000_FF01.rpt_desc",
            descriptor!("17CC_1130_0000_FF01.rpt_desc"),
            Some((86, 0xda88cbdd)),
        ),
    ];
    for (name, bytes, expected) in CASES {
        let parsed = parse_report_descriptor(bytes).ok();
        assert_eq!(parsed.as_ref().map(fields_hash), expected, "{name}");
    }
}

#[test]
fn random_descriptors() {
    type Case = (u32, usize, Option<(usize, u32)>);
    const CASES: [Case; 64] = [
        (34, 85, Some((0, 0x811c9dc5))),
        (9, 18, Some((0, 0x811c9dc5))),
        (8, 16, Some((0, 0x811c9dc5))),
        (10, 23, Some((0, 0x811c9dc5))),
        (18, 38, Some((0, 0x811c9dc5))),
        (2, 4, Some((0, 0x811c9dc5))),
        (8, 16, Some((0, 0x811c9dc5))),
        (3, 6, Some((0, 0x811c9dc5))),
        (12, 25, Some((0, 0x811c9dc5))),
        (26, 60, Some((0, 0x811c9dc5))),
        (25, 58, Some((0, 0x811c9dc5))),
        (30, 69, Some((0, 0x811c9dc5))),
        (1, 2, Some((0, 0x811c9dc5))),
        (3, 5, None),
        (11, 27, Some((0, 0x811c9dc5))),
        (6, 14, None),
        (40, 82, Some((6, 0x70ad0e04))),
        (20, 40, Some((17, 0x1eca88ad))),
        (36, 81, Some((4, 0x6ab7ebec))),
        (20, 40, None),
        (30, 63, Some((6, 0x581813be))),
        (2, 4, Some((0, 0x811c9dc5))),
        (27, 64, Some((1, 0x90314a32))),
        (19, 41, Some((0, 0x811c9dc5))),
        (16, 33, None),
        (37, 83, Some((10, 0x95bf832c))),
        (39, 81, Some((0, 0x811c9dc5))),
        (9, 18, Some((0, 0x811c9dc5))),
        (18, 48, Some((0, 0x811c9dc5))),
        (20, 50, Some((15, 0x5d30e779))),
        (32, 71, Some((22, 0xad7f230b))),
        (7, 16, Some((0, 0x811c9dc5))),
        (11, 21, Some((0, 0x811c9dc5))),
        (10, 25, Some((0, 0x811c9dc5))),
        (9, 22, Some((0, 0x811c9dc5))),
        (9, 21, Some((0, 0x811c9dc5))),
        (27, 57, Some((0, 0x811c9dc5))),
        (14, 35, Some((0, 0x811c9dc5))),
        (18, 42, Some((0, 0x811c9dc5))),
        (11, 26, Some((0, 0x811c9dc5))),
        (23, 60, Some((12, 0x81c28967))),
        (23, 47, Some((11, 0x717b5136))),
        (11, 26, Some((0, 0x811c9dc5))),
        (26, 54, Some((0, 0x811c9dc5))),
        (18, 45, Some((0, 0x811c9dc5))),
        (11, 22, Some((0, 0x811c9dc5))),
        (35, 77, Some((0, 0x811c9dc5))),
        (12, 24, None),
        (35, 79, Some((0, 0x811c9dc5))),
        (14, 36, Some((0, 0x811c9dc5))),
        (24, 60, None),
        (30, 61, Some((0, 0x811c9dc5))),
        (18, 32, Some((0, 0x811c9dc5))),
        (15, 31, Some((0, 0x811c9dc5))),
        (5, 10, Some((0, 0x811c9dc5))),
        (3, 7, Some((0, 0x811c9dc5))),
        (32, 72, Some((0, 0x811c9dc5))),
        (7, 13, Some((0, 0x811c9dc5))),
        (37, 80, Some((21, 0xc2225ef2))),
        (7, 16, Some((0, 0x811c9dc5))),
        (33, 73, Some((0, 0x811c9dc5))),
        (36, 86, Some((0, 0x811c9dc5))),
        (18, 35, Some((0, 0x811c9dc5))),
        (17, 40, Some((0, 0x811c9dc5))),
    ];
    const ALPHABET: [u8; 16] = [
        0x05, 0x09, 0x0a, 0x19, 0x29, 0x75, 0x95, 0x85, 0x81, 0x81, 0x91, 0xa1, 0xc0, 0x06, 0x0b,
        0x01,
    ];
    const SIZES: [usize; 4] = [0, 1, 2, 4];
    let mut state: u32 = 0x9e3779b9;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    for (items, len, expected) in CASES {
        let n_items = 1 + next() % 40;
        assert_eq!(n_items, items);
        let mut buf = Vec::new();
        for _ in 0..n_items {
            let prefix = ALPHABET[next() as usize % ALPHABET.len()];
            buf.push(prefix);
            for k in 0..SIZES[usize::from(prefix & 3)] {
                buf.push(if k == 0 { next() % 24 } else { next() % 3 } as u8);
            }
        }
        if next() % 8 == 0 {
            buf.pop(); // sometimes truncated
        }
        assert_eq!(buf.len(), len);
        let parsed = parse_report_descriptor(&buf).ok();
        assert_eq!(parsed.as_ref().map(fields_hash), expected, "{buf:02x?}");
    }
}

#[test]
fn report_data() {
    let data = [
        0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88,
    ];
    const CASES: [(i32, i32, Option<u32>); 56] = [
        (0, 1, Some(0x00000000)),
        (0, 4, Some(0x00000002)),
        (0, 8, Some(0x00000012)),
        (0, 10, Some(0x00000012)),
        (0, 15, Some(0x00003412)),
        (0, 16, Some(0x00003412)),
        (0, 32, Some(0x78563412)),
        (3, 1, Some(0x00000000)),
        (3, 4, Some(0x00000002)),
        (3, 8, Some(0x00000002)),
        (3, 10, Some(0x00000282)),
        (3, 15, Some(0x00000682)),
        (3, 16, Some(0x00000682)),
        (3, 32, Some(0x0f0ac682)),
        (8, 1, Some(0x00000000)),
        (8, 4, Some(0x00000004)),
        (8, 8, Some(0x00000034)),
        (8, 10, Some(0x00000234)),
        (8, 15, Some(0x00005634)),
        (8, 16, Some(0x00005634)),
        (8, 32, Some(0x9a785634)),
        (12, 1, Some(0x00000001)),
        (12, 4, Some(0x00000003)),
        (12, 8, Some(0x00000003)),
        (12, 10, Some(0x00000163)),
        (12, 15, Some(0x00000563)),
        (12, 16, Some(0x00000563)),
        (12, 32, Some(0x09a78563)),
        (17, 1, Some(0x00000001)),
        (17, 4, Some(0x0000000b)),
        (17, 8, Some(0x0000002b)),
        (17, 10, Some(0x0000002b)),
        (17, 15, Some(0x00003c2b)),
        (17, 16, Some(0x00003c2b)),
        (17, 32, Some(0x5e4d3c2b)),
        (31, 1, Some(0x00000000)),
        (31, 4, Some(0x00000000)),
        (31, 8, Some(0x00000000)),
        (31, 10, Some(0x00000134)),
        (31, 15, Some(0x00000134)),
        (31, 16, Some(0x00000134)),
        (31, 32, Some(0x01bd7934)),
        (64, 1, Some(0x00000001)),
        (64, 4, Some(0x00000001)),
        (64, 8, Some(0x00000011)),
        (64, 10, Some(0x00000211)),
        (64, 15, Some(0x00002211)),
        (64, 16, Some(0x00002211)),
        (64, 32, Some(0x44332211)),
        (100, 1, Some(0x00000001)),
        (100, 4, Some(0x00000005)),
        (100, 8, Some(0x00000005)),
        (100, 10, Some(0x00000265)),
        (100, 15, Some(0x00000665)),
        (100, 16, Some(0x00000665)),
        (100, 32, Some(0x08877665)),
    ];
    for (bit_offset, bit_size, expected) in CASES {
        assert_eq!(
            read_report_data(&data, data.len(), bit_offset, bit_size).ok(),
            expected,
            "{bit_offset} {bit_size}"
        );
    }
}
