// OpenType SVG tests of sdl3-ttf against upstream SDL_ttf's C and PlutoSVG.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The fonts are `testdata/fonts/DejaVuSans-SVG.ttf` and
//! `NotoSans-SVG.ttf`, made by `tools/gen_sdl_ttf_testdata.py --svg`: the
//! DejaVu Sans and Noto Sans subsets (see `testdata/fonts/LICENSE` and
//! `OFL.txt`) with `SVG ` tables whose documents cover what PlutoSVG
//! renders (paths, fills, strokes, dashes, gradients, transforms, groups,
//! opacities, `use`, clip paths, nested `svg` elements, shapes, units,
//! colors and CPAL palette entries, an embedded PNG image), per glyph,
//! shared by ranges of glyphs and gzip-compressed, alongside plain
//! glyphs. `testdata/svg_reference.txt` is the output of a C program
//! built from upstream SDL_ttf (with its bundled FreeType, HarfBuzz,
//! PlutoSVG and PlutoVG) and SDL3, which runs the cases below in the same
//! order: for each font, the glyphs' metrics and renderings at several
//! sizes (in every render mode at two), text rendered with them in every
//! mode, glyph images, hinting modes, styles, an outline, SDF rendering,
//! wrapping, a size with different horizontal and vertical resolutions,
//! text objects drawn by the surface text engine; the font truncated and
//! with flipped bytes; and the glyphs of each of its SVG documents with
//! bytes of the table's index and of the documents changed, the documents
//! truncated, and the font truncated in the table.
//!
//! The C program opens each font with a new FreeType library: PlutoSVG's
//! FreeType hooks cache documents by their address and length, so a font
//! whose SVG table is allocated where an earlier (closed) font's was finds
//! that font's freed document there (see `plutosvg::ft`, which caches them
//! by their contents).

use std::rc::Rc;

use super::{
    corrupt_cases, describe, draw_text, info, open_mem, render, render_glyph, surface_hash, BG, FG,
    FGA, HINTS, MODES,
};
use crate::{
    Font, Hinting, ImageType, SurfaceTextEngine, Text, TextEngine, STYLE_BOLD, STYLE_ITALIC,
    STYLE_NORMAL, STYLE_STRIKETHROUGH, STYLE_UNDERLINE,
};

static FONTS: [(&str, &[u8]); 2] = [
    (
        "DejaVuSans-SVG.ttf",
        include_bytes!("../testdata/fonts/DejaVuSans-SVG.ttf"),
    ),
    (
        "NotoSans-SVG.ttf",
        include_bytes!("../testdata/fonts/NotoSans-SVG.ttf"),
    ),
];

fn svg_chars(name: &str) -> &'static str {
    if name.contains("Noto") {
        "abcdefghvwxyzZ\u{e9}"
    } else {
        "ABCDEFGHIJKLMNQR0123456789()Sz"
    }
}

fn glyph_metrics_line(f: &Font, label: &str, ch: u32) -> String {
    format!(
        "metrics {label} U+{ch:04X}: has={} {}",
        f.has_glyph(ch) as i32,
        match f.glyph_metrics(ch) {
            Ok((a, b, c, d, e)) => format!("{a} {b} {c} {d} {e}"),
            Err(e) => format!("err: {e}"),
        }
    )
}

fn svg_font_cases(out: &mut Vec<String>, name: &str, data: &[u8]) {
    out.push(format!("== svg {name}"));
    let chars = svg_chars(name);
    let f = match open_mem(data, 16.5) {
        Ok(f) => f,
        Err(e) => {
            out.push(format!("open: err: {e}"));
            return;
        }
    };
    out.push(format!("info: {}", info(&f)));
    for (s, size) in [9.0f32, 16.5, 40.0, 96.0].into_iter().enumerate() {
        let _ = f.set_size(size);
        let label = format!("{size}");
        out.push(format!("info: {}", info(&f)));
        for ch in chars.chars().map(|c| c as u32) {
            out.push(glyph_metrics_line(&f, &label, ch));
            let modes = if s == 1 || s == 2 { 4 } else { 1 };
            for m in 0..modes {
                let mode = if modes == 1 { 2 } else { m };
                out.push(format!(
                    "glyph {label} U+{ch:04X} {}: {}",
                    MODES[mode],
                    render_glyph(&f, mode, ch)
                ));
            }
        }
        for (m, mname) in MODES.iter().enumerate() {
            out.push(format!("render {label} {mname}: {}", render(&f, m, chars)));
        }
    }
    f.set_size(40.0).unwrap();
    for ch in chars.chars().map(|c| c as u32) {
        let (ty, s) = match f.glyph_image(ch) {
            Ok((s, ty)) => (ty, Ok(s)),
            Err(e) => (ImageType::Invalid, Err(e)),
        };
        out.push(format!(
            "glyph_image U+{ch:04X}: type={} {}",
            ty as i32,
            describe(s)
        ));
    }
    f.set_size(16.5).unwrap();
    for (hname, hinting) in HINTS {
        f.set_hinting(hinting);
        out.push(format!("render hinting {hname}: {}", render(&f, 2, chars)));
    }
    f.set_hinting(Hinting::Normal);

    for style in [
        STYLE_BOLD,
        STYLE_ITALIC,
        STYLE_UNDERLINE | STYLE_STRIKETHROUGH,
        STYLE_BOLD | STYLE_ITALIC,
    ] {
        f.set_style(style);
        for (m, mname) in MODES.iter().enumerate().take(3).skip(1) {
            out.push(format!(
                "render style {} {mname}: {}",
                style,
                render(&f, m, chars)
            ));
        }
        out.push(format!(
            "glyph style {}: {}",
            style,
            render_glyph(&f, 2, chars.as_bytes()[0] as u32)
        ));
    }
    f.set_style(STYLE_NORMAL);
    f.set_outline(2).unwrap();
    for (m, mname) in MODES.iter().enumerate().take(3).skip(1) {
        out.push(format!("render outline {mname}: {}", render(&f, m, chars)));
    }
    f.set_outline(0).unwrap();
    out.push(format!("sdf: {}", f.set_sdf(true).is_ok() as i32));
    out.push(format!("render sdf: {}", render(&f, 2, chars)));
    let _ = f.set_sdf(false);

    out.push(format!(
        "render blended alpha: {}",
        describe(f.render_text_blended(chars, FGA))
    ));
    out.push(format!(
        "render wrapped: {}",
        describe(f.render_text_blended_wrapped(chars, FG, 100))
    ));
    out.push(format!(
        "render wrapped shaded: {}",
        describe(f.render_text_shaded_wrapped(chars, FG, BG, 60))
    ));

    let ok = f.set_size_dpi(12.0, 96, 144).is_ok() as i32;
    out.push(format!("sizedpi: {ok} info: {}", info(&f)));
    out.push(format!(
        "render dpi: {}",
        describe(f.render_text_blended(chars, FG))
    ));
    for ch in chars.chars().map(|c| c as u32) {
        out.push(glyph_metrics_line(&f, "dpi", ch));
        out.push(format!("glyph dpi U+{ch:04X}: {}", render_glyph(&f, 2, ch)));
    }
    f.set_size(16.5).unwrap();

    let engine: Rc<dyn TextEngine> = SurfaceTextEngine::new().unwrap();
    let t = Text::new(Some(engine.clone()), Some(&f), chars).unwrap();
    draw_text(out, "svg text", &t, 3, 4);
    t.set_wrap_width(120).unwrap();
    draw_text(out, "svg text wrap", &t, 0, 0);
    f.set_size(30.0).unwrap();
    draw_text(out, "svg text 30", &t, 0, 0);
}

fn be32(p: &[u8]) -> u32 {
    u32::from_be_bytes([p[0], p[1], p[2], p[3]])
}

fn be16(p: &[u8]) -> u32 {
    u16::from_be_bytes([p[0], p[1]]) as u32
}

/// The offset and length of the `SVG ` table.
fn find_svg_table(data: &[u8]) -> (usize, usize) {
    let n = be16(&data[4..]) as usize;
    for i in 0..n {
        let r = &data[12 + 16 * i..];
        if &r[..4] == b"SVG " {
            return (be32(&r[8..]) as usize, be32(&r[12..]) as usize);
        }
    }
    (0, 0)
}

fn svg_corrupt_case(out: &mut Vec<String>, label: &str, data: &[u8], gid1: u32, gid2: u32) {
    let f = match open_mem(data, 20.0) {
        Ok(f) => f,
        Err(e) => {
            out.push(format!("{label}: err: {e}"));
            return;
        }
    };
    let mut line = format!("{label}: ");
    for k in 0..if gid1 == gid2 { 1 } else { 2 } {
        if k != 0 {
            line += " | ";
        }
        match f.glyph_image_for_index(if k != 0 { gid2 } else { gid1 }) {
            Ok((img, ty)) => {
                line += &format!(
                    "type={} {}x{} {:016x}",
                    ty as i32,
                    img.width(),
                    img.height(),
                    surface_hash(&img)
                )
            }
            Err(e) => line += &format!("type=0 err: {e}"),
        }
    }
    match f.render_text_blended("AQ0z", FG) {
        Ok(s) => line += &format!(" | {}x{} {:016x}", s.width(), s.height(), surface_hash(&s)),
        Err(e) => line += &format!(" | err: {e}"),
    }
    out.push(line);
}

fn svg_corrupt_cases(out: &mut Vec<String>, name: &str, data: &[u8]) {
    out.push(format!("== svg corrupt {name}"));
    let (off, tlen) = find_svg_table(data);
    let mut copy = data.to_vec();
    /* the table's header and document index */
    let index = off + be32(&data[off + 2..]) as usize;
    let count = be16(&data[index..]) as usize;
    let mut i = 0;
    while i < 10 + count * 12 && i < tlen {
        for x in [0x01u8, 0x80, 0xFF] {
            copy.copy_from_slice(data);
            copy[off + i] ^= x;
            let label = format!("index {i} ^{x:02x}");
            let r = if i < 12 { 0 } else { (i - 12) / 12 }.min(count - 1);
            let rec = &data[index + 2 + 12 * r..];
            svg_corrupt_case(out, &label, &copy, be16(rec), be16(&rec[2..]));
        }
        i += 1;
    }
    /* each document's bytes */
    for r in 0..count {
        let rec = &data[index + 2 + 12 * r..];
        let (gid1, gid2) = (be16(rec), be16(&rec[2..]));
        let doff = index + be32(&rec[4..]) as usize;
        let dlen = be32(&rec[8..]) as usize;
        for j in 0..16 {
            let o = (doff + j * dlen / 16 + j % 3).min(doff + dlen - 1);
            for (x, set) in [0u8, b'<', b'9', b'"', b'-'].into_iter().enumerate() {
                copy.copy_from_slice(data);
                copy[o] = if set != 0 { set } else { copy[o] ^ 0xA5 };
                let label = format!("doc {r} at {} set {x}", o - doff);
                svg_corrupt_case(out, &label, &copy, gid1, gid2);
            }
        }
        /* the document truncated */
        for j in 0..4 {
            let cut = ((j + 1) * dlen / 5) as u32;
            copy.copy_from_slice(data);
            let rc = index + 2 + 12 * r;
            copy[rc + 8..rc + 12].copy_from_slice(&cut.to_be_bytes());
            let label = format!("doc {r} cut {cut}");
            svg_corrupt_case(out, &label, &copy, gid1, gid2);
        }
    }
    /* the font truncated in the table */
    for j in 0..8 {
        let cut = off + j * tlen / 8;
        svg_corrupt_case(out, &format!("trunc {cut}"), &data[..cut], 0, 0);
    }
}

#[test]
fn matches_upstream_svg_reference() {
    crate::init().unwrap();
    let mut out = Vec::new();
    for (name, data) in FONTS {
        svg_font_cases(&mut out, name, data);
        corrupt_cases(&mut out, name, data);
        svg_corrupt_cases(&mut out, name, data);
    }
    crate::quit();

    let reference: Vec<&str> = include_str!("../testdata/svg_reference.txt")
        .lines()
        .collect();
    let mut failures = Vec::new();
    for (i, expected) in reference.iter().enumerate() {
        let actual = out.get(i).map(String::as_str).unwrap_or("<missing>");
        if actual != *expected {
            failures.push(format!(
                "line {}:\n  expected {expected}\n  actual   {actual}",
                i + 1
            ));
        }
    }
    if out.len() > reference.len() {
        failures.push(format!("{} extra lines", out.len() - reference.len()));
    }
    if std::env::var_os("SDL3_TTF_SVG_DUMP").is_some() {
        std::fs::write(
            std::env::var_os("SDL3_TTF_SVG_DUMP").unwrap(),
            out.join("\n") + "\n",
        )
        .unwrap();
    }
    assert!(
        failures.is_empty(),
        "{} of {} lines differ from upstream:\n{}",
        failures.len(),
        reference.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
