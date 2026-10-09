// Tests of the dav1d translation against upstream dav1d's C (the reference
// lines in testdata/dav1d/reference.txt come from a C harness built from
// dav1d at the pinned revision, decoding the same streams the same way).

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
        for l in decode_ivf(&buf, grain) {
            println!("{l}");
        }
    }
}
