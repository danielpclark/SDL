// Tests of the dav1d translation against upstream dav1d's C.
//
// The streams in testdata/dav1d/ are made by tools/gen_dav1d_testdata.py;
// the lines of testdata/dav1d/reference.txt come from a C harness built
// from dav1d at the pinned revision (plain C, no assembly), decoding each
// stream and its truncated and corrupted variants (see `variants()`) the
// way `decode_ivf()` does, with one thread and a frame delay of one.

use super::*;

/// FNV-1a 64.
fn fnv(mut h: u64, p: &[u8]) -> u64 {
    for &b in p {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The line of one output picture: its size, layout, bit depth and the
/// hashes of its visible planes.
fn pic_line(p: &Dav1dPicture, n: usize) -> String {
    let ss_ver = (p.p.layout == headers::DAV1D_PIXEL_LAYOUT_I420) as i32;
    let ss_hor = (p.p.layout != headers::DAV1D_PIXEL_LAYOUT_I444) as i32;
    let data = p.data.as_deref().expect("picture data");
    let mut s = format!(
        "pic {} {}x{} layout={} bpc={}",
        n, p.p.w, p.p.h, p.p.layout, p.p.bpc
    );
    let n_planes = if p.p.layout == headers::DAV1D_PIXEL_LAYOUT_I400 {
        1
    } else {
        3
    };
    for pl in 0..n_planes {
        let w = if pl != 0 {
            (p.p.w + ss_hor) >> ss_hor
        } else {
            p.p.w
        } as usize;
        let h = if pl != 0 {
            (p.p.h + ss_ver) >> ss_ver
        } else {
            p.p.h
        } as usize;
        let stride = data.stride[(pl != 0) as usize];
        let mut hash = 0xcbf29ce484222325u64;
        match &data.planes {
            PicPlanes::U8(planes) => {
                for y in 0..h {
                    hash = fnv(hash, &planes[pl][y * stride..y * stride + w]);
                }
            }
            PicPlanes::U16(planes) => {
                for y in 0..h {
                    let row: Vec<u8> = planes[pl][y * stride..y * stride + w]
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect();
                    hash = fnv(hash, &row);
                }
            }
        }
        s += &format!(" {hash:016x}");
    }
    s
}

/// Decodes an IVF file like the C harness does and returns its lines.
pub(super) fn decode_ivf(buf: &[u8], apply_grain: bool) -> Vec<String> {
    let mut out = Vec::new();
    let s = Dav1dSettings {
        n_threads: 1,
        max_frame_delay: 1,
        frame_size_limit: 512 * 512,
        logger: None,
        apply_grain,
        ..Default::default()
    };
    let mut c = dav1d_open(&s).expect("dav1d_open");
    if buf.len() < 32 || &buf[..4] != b"DKIF" {
        out.push("bad ivf".to_string());
        return out;
    }
    let mut npic = 0;
    let mut pos = 32;
    while pos + 12 <= buf.len() {
        let mut fsz = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 12;
        if fsz > buf.len() - pos {
            fsz = buf.len() - pos;
        }
        if fsz == 0 {
            continue;
        }
        let mut data = Dav1dData::from_slice(&buf[pos..pos + fsz]);
        pos += fsz;
        let mut guard = 0;
        loop {
            let res = dav1d_send_data(&mut c, &mut data);
            if res < 0 && res != decode::dav1d_err(decode::EAGAIN) {
                out.push(format!("send_err {res}"));
                data.unref();
                break;
            }
            let mut p = Dav1dPicture::default();
            let res2 = dav1d_get_picture(&mut c, &mut p);
            if res2 == 0 {
                out.push(pic_line(&p, npic));
                npic += 1;
            } else if res2 != decode::dav1d_err(decode::EAGAIN) {
                out.push(format!("get_err {res2}"));
            }
            guard += 1;
            if data.sz == 0 || guard >= 1000 {
                break;
            }
        }
    }
    for _ in 0..1000 {
        let mut p = Dav1dPicture::default();
        let res = dav1d_get_picture(&mut c, &mut p);
        if res == 0 {
            out.push(pic_line(&p, npic));
            npic += 1;
        } else if res == decode::dav1d_err(decode::EAGAIN) {
            break;
        } else {
            out.push(format!("get_err {res}"));
        }
    }
    let mut c = Some(c);
    dav1d_close(&mut c);
    out
}

/// Prints the lines of the IVF file `$DAV1D_IVF` (for comparing a stream
/// with the C harness by hand).
#[test]
#[ignore]
fn dav1d_dump() {
    let paths = std::env::var("DAV1D_IVF").expect("DAV1D_IVF");
    let grain = std::env::var("DAV1D_GRAIN").map_or(true, |v| v != "0");
    for path in paths.split(':') {
        let buf = std::fs::read(path).expect("read");
        println!("== {path}");
        match std::panic::catch_unwind(|| decode_ivf(&buf, grain)) {
            Ok(lines) => {
                for l in lines {
                    println!("{l}");
                }
            }
            Err(_) => println!("PANIC"),
        }
    }
}

macro_rules! streams {
    ($($name:expr,)*) => {
        &[$(($name, include_bytes!(concat!("../testdata/dav1d/", $name)) as &[u8]),)*]
    };
}

/// The streams of testdata/dav1d/.
static STREAMS: &[(&str, &[u8])] = streams![
    "i400_10bit.ivf",
    "i400_12bit.ivf",
    "i400_8bit.ivf",
    "i420_10bit.ivf",
    "i420_10bit_grain.ivf",
    "i420_10bit_still.ivf",
    "i420_10bit_tools.ivf",
    "i420_12bit.ivf",
    "i420_8bit_grain.ivf",
    "i420_8bit_grain2.ivf",
    "i420_8bit_inter.ivf",
    "i420_8bit_intrabc.ivf",
    "i420_8bit_key.ivf",
    "i420_8bit_lossless.ivf",
    "i420_8bit_nofilters.ivf",
    "i420_8bit_qm.ivf",
    "i420_8bit_sb128.ivf",
    "i420_8bit_screen.ivf",
    "i420_8bit_segments.ivf",
    "i420_8bit_still.ivf",
    "i420_8bit_tiles.ivf",
    "i420_8bit_tools.ivf",
    "i422_10bit.ivf",
    "i422_12bit_grain.ivf",
    "i422_8bit.ivf",
    "i444_10bit.ivf",
    "i444_12bit.ivf",
    "i444_8bit.ivf",
    "i444_8bit_grain.ivf",
    "i444_8bit_screen.ivf",
    "rav1e_i420_8bit.ivf",
    "svt_i420_10bit.ivf",
    "svt_i420_8bit_resize.ivf",
    "svt_i420_8bit_superres.ivf",
];

/// A stream and its variants: truncated to 1/4, 2/4 and 3/4 of its size,
/// and with one byte XORed with 0x55 at 1/8, 3/8, 5/8 and 7/8 of the way
/// past the IVF file header.
fn variants(buf: &[u8]) -> Vec<(String, Vec<u8>)> {
    let n = buf.len();
    let mut out = vec![(String::new(), buf.to_vec())];
    for k in 1..4 {
        out.push((format!(" trunc{k}"), buf[..n * k / 4].to_vec()));
    }
    for k in 1..5 {
        let pos = 32 + (n - 32) * (2 * k - 1) / 8;
        let mut b = buf.to_vec();
        b[pos] ^= 0x55;
        out.push((format!(" flip{k}"), b));
    }
    out
}

/// The sections of reference.txt by name.
fn reference() -> std::collections::HashMap<String, Vec<String>> {
    let mut map = std::collections::HashMap::new();
    let mut cur: Option<String> = None;
    for l in include_str!("../testdata/dav1d/reference.txt").lines() {
        if let Some(name) = l.strip_prefix("== ") {
            map.insert(name.to_string(), Vec::new());
            cur = Some(name.to_string());
        } else if !l.starts_with('#') {
            if let Some(c) = &cur {
                map.get_mut(c).expect("section").push(l.to_string());
            }
        }
    }
    map
}

/// Every stream and variant decodes to the pictures and errors upstream's C
/// does (bit-exact planes).
#[test]
fn dav1d_matches_upstream() {
    let reference = reference();
    let mut failures = Vec::new();
    let mut n = 0;
    for (name, buf) in STREAMS {
        for (tag, data) in variants(buf) {
            let key = format!("{name}{tag}");
            let expected = reference
                .get(&key)
                .unwrap_or_else(|| panic!("no reference for {key}"));
            let got = decode_ivf(&data, true);
            if &got != expected {
                failures.push(format!(
                    "{key}:\n  expected {expected:?}\n  got      {got:?}"
                ));
            }
            n += 1;
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {n} mismatched:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(n, reference.len());
}

/// Film grain off: the streams with grain decode to other pictures, the
/// others to the same.
#[test]
fn dav1d_without_grain() {
    let reference = reference();
    for (name, buf) in STREAMS {
        let without = decode_ivf(buf, false);
        if name.contains("grain") {
            assert_eq!(without.len(), reference[*name].len(), "{name}");
            assert_ne!(&without, &reference[*name], "{name}");
        } else {
            assert_eq!(&without, &reference[*name], "{name}");
        }
    }
}

/// The API's argument checks, event flags and the sequence header parser.
#[test]
fn dav1d_api() {
    assert_eq!(dav1d_version(), "1.2.1");
    let s = Dav1dSettings::default();
    assert_eq!(dav1d_get_frame_delay(&s), Ok(1));
    let bad = Dav1dSettings {
        n_threads: -1,
        ..Default::default()
    };
    assert!(dav1d_open(&bad).is_err());
    assert!(dav1d_get_frame_delay(&bad).is_err());
    let bad = Dav1dSettings {
        operating_point: 32,
        ..Default::default()
    };
    assert!(dav1d_open(&bad).is_err());

    // the first IVF frame of a stream starts with its temporal delimiter
    // and sequence header OBUs
    let (_, buf) = STREAMS
        .iter()
        .find(|(n, _)| *n == "i420_10bit_still.ivf")
        .expect("stream");
    let fsz = u32::from_le_bytes(buf[32..36].try_into().unwrap()) as usize;
    let frame = &buf[44..44 + fsz];
    let seq = dav1d_parse_sequence_header(frame).expect("sequence header");
    assert_eq!((seq.max_width, seq.max_height, seq.hbd), (96, 80, 1));
    assert_eq!(seq.reduced_still_picture_header, 1);
    assert!(dav1d_parse_sequence_header(&[]).is_err());
    assert!(dav1d_parse_sequence_header(&[0x12, 0x00]).is_err());

    let mut c = dav1d_open(&Dav1dSettings {
        logger: None,
        ..Default::default()
    })
    .expect("open");
    let mut p = Dav1dPicture::default();
    let eagain = decode::dav1d_err(decode::EAGAIN);
    assert_eq!(dav1d_get_picture(&mut c, &mut p), eagain);
    let mut data = Dav1dData::from_slice(frame);
    assert_eq!(dav1d_send_data(&mut c, &mut data), 0);
    assert!(!data.has_data());
    assert_eq!(dav1d_get_picture(&mut c, &mut p), 0);
    assert_eq!((p.p.w, p.p.h, p.p.bpc), (96, 80, 10));
    assert!(matches!(
        p.data.as_deref().map(|d| &d.planes),
        Some(PicPlanes::U16(_))
    ));
    assert_eq!(
        dav1d_get_event_flags(&mut c),
        picture::DAV1D_EVENT_FLAG_NEW_SEQUENCE
    );
    assert_eq!(dav1d_get_event_flags(&mut c), 0);
    assert_eq!(dav1d_get_picture(&mut c, &mut p), eagain);
    let _ = dav1d_get_decode_error_data_props(&mut c);
    dav1d_flush(&mut c);
    let mut c = Some(c);
    dav1d_close(&mut c);
    assert!(c.is_none());
}
