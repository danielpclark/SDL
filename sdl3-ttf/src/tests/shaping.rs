// Shaping tests of sdl3-ttf against upstream SDL_ttf's C and HarfBuzz.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `testdata/shaping_cases.txt` lists the cases and the fonts they use:
//! subsets of Noto fonts (see `testdata/fonts/OFL.txt`) and a DejaVu Sans
//! subset without OpenType layout tables, made by
//! `tools/gen_sdl_ttf_testdata.py`. `testdata/shaping_reference.txt` is
//! the output of a C program built from upstream SDL_ttf (with its bundled
//! FreeType and HarfBuzz) and SDL3, which runs them in the same order: the
//! scripts of some characters, and, for each font at two sizes, each
//! case's glyphs shaped by HarfBuzz on FreeType (`hb_buffer_serialize`'s
//! text format with glyph flags) and the same text through SDL_ttf with
//! the case's direction, script, language and kerning set on the font
//! (with the fallback font the case file names): its size, the rendered
//! surfaces (also wrapped), and a text object's layout, clusters,
//! substrings and drawing; then text objects with their own direction and
//! script, and, for the Arabic and Devanagari fonts, the shaping and
//! rendering of the fonts with bytes of their GSUB, GPOS and GDEF tables
//! flipped or set and with those tables truncated. Malformed UTF-8 goes
//! through text objects only (the C passes it to the render functions
//! too, which this API's `&str` cannot hold).

use std::rc::Rc;

use super::{describe, draw_text, dump_text, info, open_mem, surface_hash, FG};
use crate::harfbuzz::tests::hb_line;
use crate::{
    glyph_script, harfbuzz_version, string_to_tag, tag_to_string, Direction, Font,
    SurfaceTextEngine, Text, TextEngine,
};

static CASES: &str = include_str!("../testdata/shaping_cases.txt");

static FONTS: [(&str, &[u8]); 10] = [
    (
        "NotoSans-Regular.ttf",
        include_bytes!("../testdata/fonts/NotoSans-Regular.ttf"),
    ),
    (
        "NotoSansArabic-Regular.ttf",
        include_bytes!("../testdata/fonts/NotoSansArabic-Regular.ttf"),
    ),
    (
        "NotoSansHebrew-Regular.ttf",
        include_bytes!("../testdata/fonts/NotoSansHebrew-Regular.ttf"),
    ),
    (
        "NotoSansDevanagari-Regular.ttf",
        include_bytes!("../testdata/fonts/NotoSansDevanagari-Regular.ttf"),
    ),
    (
        "NotoSansThai-Regular.ttf",
        include_bytes!("../testdata/fonts/NotoSansThai-Regular.ttf"),
    ),
    (
        "NotoSansKR-Regular.otf",
        include_bytes!("../testdata/fonts/NotoSansKR-Regular.otf"),
    ),
    (
        "NotoSansKhmer-Regular.ttf",
        include_bytes!("../testdata/fonts/NotoSansKhmer-Regular.ttf"),
    ),
    (
        "NotoSansMyanmar-Regular.ttf",
        include_bytes!("../testdata/fonts/NotoSansMyanmar-Regular.ttf"),
    ),
    (
        "NotoSansSinhala-Regular.ttf",
        include_bytes!("../testdata/fonts/NotoSansSinhala-Regular.ttf"),
    ),
    (
        "DejaVuSans-NoLayout.ttf",
        include_bytes!("../testdata/fonts/DejaVuSans-NoLayout.ttf"),
    ),
];

const SIZES: [f32; 2] = [16.0, 37.0];

fn font_data(name: &str) -> &'static [u8] {
    FONTS
        .iter()
        .find(|(n, _)| *n == name)
        .expect("a test font")
        .1
}

/// The text of a case, its `\xNN` escapes made bytes.
fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() && b[i + 1] == b'x' {
            out.push(u8::from_str_radix(&s[i + 2..i + 4], 16).unwrap());
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

fn direction_of(s: &str) -> Direction {
    match s {
        "ltr" => Direction::Ltr,
        "rtl" => Direction::Rtl,
        "ttb" => Direction::Ttb,
        "btt" => Direction::Btt,
        _ => Direction::Invalid,
    }
}

/// `TTF_TagToString()` printed with `%s`
fn tag_str(tag: u32) -> String {
    tag_to_string(tag)
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as char)
        .collect()
}

fn ttf_case(
    out: &mut Vec<String>,
    font: &Font,
    engine: &Rc<dyn TextEngine>,
    f: &[&str],
    text: &[u8],
) {
    let d = font.set_direction(direction_of(f[0])).is_ok() as i32;
    let s = font
        .set_script(if f[1] != "-" {
            string_to_tag(Some(f[1]))
        } else {
            0
        })
        .is_ok() as i32;
    let l = font
        .set_language(if f[2] != "-" { Some(f[2]) } else { None })
        .is_ok() as i32;
    font.set_kerning(f[3] != "0");
    out.push(format!(
        "set: {d} {s} {l} dir={} script={:08x} {} kerning={}",
        font.direction() as i32,
        font.script(),
        tag_str(font.script()),
        font.kerning() as i32
    ));
    if let Ok(text) = std::str::from_utf8(text) {
        out.push(match font.string_size(text) {
            Ok((w, h)) => format!("string_size: {w} {h}"),
            Err(e) => format!("string_size: err: {e}"),
        });
        out.push(format!(
            "blended: {}",
            describe(font.render_text_blended(text, FG))
        ));
        out.push(format!(
            "wrapped: {}",
            describe(font.render_text_blended_wrapped(text, FG, 120))
        ));
    }
    match Text::new_bytes(Some(engine.clone()), Some(font), text) {
        Ok(t) => {
            dump_text(out, "text", &t, font);
            draw_text(out, "text", &t, 5, 5);
        }
        Err(e) => out.push(format!("text: err: {e}")),
    }
    let _ = font.set_direction(Direction::Invalid);
    let _ = font.set_script(0);
    let _ = font.set_language(None);
    font.set_kerning(true);
}

fn text_overrides(out: &mut Vec<String>, font: &Font, engine: &Rc<dyn TextEngine>) {
    let t = Text::new(Some(engine.clone()), Some(font), "abc مرحبا 123").unwrap();
    dump_text(out, "override default", &t, font);
    out.push(format!(
        "override set: {} {}",
        t.set_direction(Direction::Rtl).is_ok() as i32,
        t.set_script(string_to_tag(Some("Arab"))).is_ok() as i32
    ));
    out.push(format!(
        "override get: {} {:08x}",
        t.direction() as i32,
        t.script()
    ));
    dump_text(out, "override rtl", &t, font);
    draw_text(out, "override rtl", &t, 5, 5);
    out.push(format!(
        "override set: {} {}",
        t.set_direction(Direction::Ltr).is_ok() as i32,
        t.set_script(string_to_tag(Some("Latn"))).is_ok() as i32
    ));
    dump_text(out, "override ltr", &t, font);
}

fn glyph_scripts(out: &mut Vec<String>) {
    for ch in [
        'A' as u32, '1' as u32, ' ' as u32, 0x0300, 0x0627, 0x05D0, 0x0915, 0x0E01, 0x0E81, 0xAC00,
        0x1100, 0x1780, 0x1000, 0x0D9A, 0x4E00, 0x3042, 0x30A2, 0x0410, 0x03B1, 0x1F600, 0xE000,
        0x0378, 0x10FFFF, 0x110000, 0xFFFFFFFF,
    ] {
        out.push(match glyph_script(ch) {
            Ok(s) => format!("glyph_script U+{ch:04X}: {s:08x} {}", tag_str(s)),
            Err(e) => format!("glyph_script U+{ch:04X}: 00000000  err: {e}"),
        });
    }
}

/// The table directory entry of `tag`
fn find_table(d: &[u8], tag: &[u8; 4]) -> Option<(usize, usize)> {
    if d.len() < 12 {
        return None;
    }
    let be32 = |i: usize| u32::from_be_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]) as usize;
    let n = u16::from_be_bytes([d[4], d[5]]) as usize;
    for i in 0..n {
        let r = 12 + 16 * i;
        if r + 16 > d.len() {
            return None;
        }
        if &d[r..r + 4] == tag {
            let (off, len) = (be32(r + 8), be32(r + 12));
            return (off + len <= d.len()).then_some((off, len));
        }
    }
    None
}

const CORRUPT_TEXTS: [[&str; 2]; 2] = [["بِسْمِ ٱللَّٰهِ لا", "السلام عليكم"], ["क्षत्रिय र्क श्री", "कि कीं ि"]];

fn corrupt_shaping_case(out: &mut Vec<String>, label: &str, data: &[u8], which: usize) {
    let f = ["-", "-", "-", "1"];
    out.push(format!(
        "{label}: | {}",
        hb_line(data, 16.0, &f, CORRUPT_TEXTS[which][0].as_bytes())
    ));
    out.push(format!(
        " | {}",
        hb_line(data, 16.0, &f, CORRUPT_TEXTS[which][1].as_bytes())
    ));
    let font = match open_mem(data, 16.0) {
        Ok(font) => font,
        Err(e) => {
            out.push(format!("  ttf: err: {e}"));
            return;
        }
    };
    for (i, text) in CORRUPT_TEXTS[which].iter().enumerate() {
        let mut line = format!("  ttf {i}: ");
        match font.string_size(text) {
            Ok((w, h)) => line += &format!("{w} {h} "),
            Err(e) => line += &format!("err: {e} "),
        }
        match font.render_text_blended(text, FG) {
            Ok(s) => line += &format!("{}x{} {:016x}", s.width(), s.height(), surface_hash(&s)),
            Err(e) => line += &format!("err: {e}"),
        }
        out.push(line);
    }
}

fn corrupt_shaping(out: &mut Vec<String>, name: &str, data: &[u8], which: usize) {
    out.push(format!("== corrupt layout {name}"));
    for table in [b"GSUB", b"GPOS", b"GDEF"] {
        let tname = std::str::from_utf8(table).unwrap();
        let Some((off, tlen)) = find_table(data, table) else {
            out.push(format!("no {tname}"));
            continue;
        };
        for i in 0..24 {
            let mut o = off + i * tlen / 24 + i % 7;
            if o >= off + tlen {
                o = off + tlen - 1;
            }
            let mut copy = data.to_vec();
            copy[o] ^= 0xA5;
            corrupt_shaping_case(out, &format!("{tname} flip {}", o - off), &copy, which);
        }
        for i in 0..6 {
            let o = off + i * 2 + if i > 2 { tlen / 3 } else { 0 };
            let mut copy = data.to_vec();
            copy[o] = 0xFF;
            copy[o + 1] = 0xFF;
            corrupt_shaping_case(out, &format!("{tname} ffff {}", o - off), &copy, which);
        }
        corrupt_shaping_case(
            out,
            &format!("{tname} trunc"),
            &data[..off + tlen / 2],
            which,
        );
    }
}

#[test]
fn matches_upstream_shaping_reference() {
    crate::init().unwrap();
    let engine: Rc<dyn TextEngine> = SurfaceTextEngine::new().unwrap();
    let mut out = Vec::new();
    let (major, minor, micro) = harfbuzz_version();
    out.push(format!("harfbuzz: {major}.{minor}.{micro}"));
    glyph_scripts(&mut out);

    /* the fonts and their cases */
    let mut fonts: Vec<(&str, Option<&str>, Vec<&str>)> = Vec::new();
    for line in CASES.lines() {
        if let Some(rest) = line.strip_prefix("font ") {
            let mut names = rest.splitn(2, ' ');
            fonts.push((names.next().unwrap(), names.next(), Vec::new()));
        } else if let Some(case) = line.strip_prefix("case\t") {
            fonts.last_mut().unwrap().2.push(case);
        }
    }
    for (name, fallback, cases) in &fonts {
        let data = font_data(name);
        out.push(format!(
            "== shaping {name}{}",
            fallback.map(|f| format!(" {f}")).unwrap_or_default()
        ));
        for size in SIZES {
            let font = open_mem(data, size).unwrap();
            let fb = fallback.map(|f| open_mem(font_data(f), size).unwrap());
            if let Some(fb) = &fb {
                font.add_fallback_font(fb).unwrap();
            }
            out.push(format!("-- size {size}"));
            out.push(format!("info: {}", info(&font)));
            for (c, case) in cases.iter().enumerate() {
                let f: Vec<&str> = case.splitn(5, '\t').collect();
                let text = unescape(f[4]);
                out.push(format!("case {c}: {case}"));
                out.push(hb_line(data, size, &f, &text));
                ttf_case(&mut out, &font, &engine, &f, &text);
            }
            if *name == "NotoSansArabic-Regular.ttf" {
                text_overrides(&mut out, &font, &engine);
            }
        }
        if *name == "NotoSansArabic-Regular.ttf" {
            corrupt_shaping(&mut out, name, data, 0);
        }
        if *name == "NotoSansDevanagari-Regular.ttf" {
            corrupt_shaping(&mut out, name, data, 1);
        }
    }
    drop(engine);
    crate::quit();

    let reference: Vec<&str> = include_str!("../testdata/shaping_reference.txt")
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
    assert!(
        failures.is_empty(),
        "{} of {} lines differ from upstream:\n{}",
        failures.len(),
        reference.len(),
        failures.join("\n")
    );
}
