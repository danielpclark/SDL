//! Tests of the aom translation: generated 10-bit frames coded with the
//! settings libavif gives the saver, checked against the reference
//! encoder's output (the sizes and FNV-1a 64 hashes of the packets).

use super::av1::av1_cx_iface::*;
use super::av1::encoder::encoder::*;

fn fnv64(p: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in p {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1103515245).wrapping_add(12345);
        (self.0 >> 8) & 0xffffff
    }
}

fn sample(rng: &mut Rng, pattern: i32, plane: i32, x: i32, y: i32, w: i32, h: i32) -> u16 {
    match pattern {
        0 => {
            (((x * 1023) / if w > 1 { w - 1 } else { 1 }
                + (y * 511) / if h > 1 { h - 1 } else { 1 }
                + plane * 100)
                % 1024) as u16
        }
        1 => (rng.next() % 1024) as u16,
        2 => {
            if (((x / 3) + (y / 2) + plane) & 1) != 0 {
                940
            } else {
                64
            }
        }
        _ => (512 + plane * 37) as u16,
    }
}

/// Codes one generated frame the way libavif drives the encoder for the
/// saver.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode(
    w: i32,
    h: i32,
    mono: bool,
    quant: u32,
    lossless: bool,
    pattern: i32,
    cp: i32,
    tc: i32,
    mc: i32,
    range: i32,
) -> Option<Vec<u8>> {
    let mut rng = Rng((w * 131 + h * 7 + mono as i32 * 3 + pattern) as u32);
    let nplanes = if mono { 1 } else { 3 };
    let mut planes: Vec<Vec<u16>> = Vec::new();
    for p in 0..nplanes {
        let mut v = vec![0u16; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                v[(y * w + x) as usize] = sample(&mut rng, pattern, p, x, y, w, h);
            }
        }
        planes.push(v);
    }
    let mut cfg = aom_codec_enc_config_default(AOM_USAGE_ALL_INTRA)?;
    cfg.rc_end_usage = AOM_Q;
    cfg.g_profile = if mono { 0 } else { 1 };
    cfg.g_bit_depth = 10;
    cfg.g_input_bit_depth = 10;
    cfg.g_w = w as u32;
    cfg.g_h = h as u32;
    cfg.g_limit = 1;
    cfg.g_lag_in_frames = 0;
    cfg.kf_mode = AOM_KF_DISABLED;
    cfg.kf_max_dist = 0;
    if mono {
        cfg.monochrome = 1;
    }
    cfg.rc_min_quantizer = 0;
    cfg.rc_max_quantizer = 63;
    let mut ctx = encoder_init(&cfg, AOM_CODEC_USE_HIGHBITDEPTH).ok()?;
    ctrl_set_cq_level(&mut ctx, quant).ok()?;
    if lossless {
        ctrl_set_lossless(&mut ctx, 1).ok()?;
    }
    ctrl_set_cpuused(&mut ctx, 9).ok()?;
    if mono {
        ctrl_set_color_range(&mut ctx, AOM_CR_FULL_RANGE).ok()?;
    } else {
        if cp != 2 {
            ctrl_set_color_primaries(&mut ctx, cp).ok()?;
        }
        if tc != 2 {
            ctrl_set_transfer_characteristics(&mut ctx, tc).ok()?;
        }
        if mc != 2 {
            ctrl_set_matrix_coefficients(&mut ctx, mc).ok()?;
        }
        if range != 0 {
            ctrl_set_color_range(&mut ctx, range).ok()?;
        }
    }
    ctrl_set_skip_postproc_filtering(&mut ctx, 1).ok()?;
    ctrl_set_tuning(&mut ctx, AOM_TUNE_SSIM).ok()?;

    let empty: &[u16] = &[];
    let img = AomImage {
        fmt: if mono {
            AOM_IMG_FMT_I42016
        } else {
            AOM_IMG_FMT_I44416
        },
        cp: if mono { 0 } else { cp },
        tc: if mono { 0 } else { tc },
        mc: if mono { 0 } else { mc },
        monochrome: mono,
        csp: 0,
        range: if mono { AOM_CR_FULL_RANGE } else { range },
        w: w as u32,
        h: h as u32,
        bit_depth: 16,
        d_w: w as u32,
        d_h: h as u32,
        x_chroma_shift: mono as u32,
        y_chroma_shift: mono as u32,
        planes: [
            &planes[0],
            if mono { empty } else { &planes[1] },
            if mono { empty } else { &planes[2] },
        ],
        stride: [w, if mono { 0 } else { w }, if mono { 0 } else { w }],
    };
    let mut out = Vec::new();
    for pkt in encoder_encode(&mut ctx, Some(&img), 0, 1, 0).ok()? {
        out.extend_from_slice(&pkt.buf);
    }
    let extra = encoder_encode(&mut ctx, None, 0, 1, 0).ok()?;
    if !extra.is_empty() {
        return None;
    }
    Some(out)
}

/// Frames coded with the saver's settings, against the reference encoder:
/// libaom as SDL_image's external/aom builds it (with `AOM_TARGET_CPU=generic`),
/// driven through `aom_codec_*()` with the same configuration and controls,
/// coding the same generated frames (`(width, height, monochrome, cq level,
/// lossless, pattern, color primaries, transfer characteristics, matrix
/// coefficients, color range, size, FNV-1a 64 hash)`).
type Case = (
    i32,
    i32,
    bool,
    u32,
    bool,
    i32,
    i32,
    i32,
    i32,
    i32,
    usize,
    u64,
);

#[test]
fn aom_frames_match_reference() {
    #[rustfmt::skip]
    let cases: &[Case] = &[
        (1, 1, false, 63, false, 0, 2, 2, 2, 0, 23, 0x9dee394fab4afe09),
        (1, 1, true, 0, true, 1, 2, 2, 2, 0, 19, 0x2dc87e50c902deff),
        (2, 3, false, 32, false, 1, 1, 13, 6, 0, 77, 0x71e6dbe9d86abc76),
        (3, 5, true, 0, true, 1, 2, 2, 2, 0, 64, 0xd5c79316878b790e),
        (8, 8, false, 6, false, 2, 1, 13, 1, 1, 188, 0x2489c447d4ce2a10),
        (16, 16, false, 32, false, 0, 1, 13, 6, 1, 269, 0xbb484264cc3e32f1),
        (17, 9, false, 20, false, 1, 9, 16, 9, 0, 420, 0x7b80e8647d9ea375),
        (33, 70, false, 40, false, 0, 1, 13, 0, 1, 724, 0xbf98700ac585a295),
        (48, 48, true, 63, false, 0, 1, 8, 6, 0, 55, 0x3c003f6f4441d331),
        (64, 64, false, 63, false, 0, 1, 8, 6, 0, 149, 0x74d4f45eb443ad36),
        (65, 33, true, 0, true, 1, 2, 2, 2, 0, 4535, 0x5906373590b52ace),
        (65, 33, false, 0, true, 2, 1, 13, 6, 1, 5566, 0x3f9de188877bfe81),
        (64, 64, false, 50, false, 3, 2, 2, 2, 0, 27, 0x1713fb3ea088e817),
        (130, 66, false, 28, false, 1, 1, 13, 6, 1, 15509, 0xe061018dbc093aa8),
    ];
    for &(w, h, mono, q, ll, pat, cp, tc, mc, range, size, hash) in cases {
        let out = encode(w, h, mono, q, ll, pat, cp, tc, mc, range);
        let got = out.as_ref().map(|o| (o.len(), fnv64(o)));
        assert_eq!(
            got,
            Some((size, hash)),
            "{w}x{h} mono {mono} q {q} lossless {ll} pattern {pat}"
        );
    }
}
