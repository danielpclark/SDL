// AAT shaping tests of sdl3-ttf against upstream SDL_ttf's C and HarfBuzz.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! `testdata/aat_cases.txt` lists the cases and the fonts they use: fonts
//! with Apple Advanced Typography tables made by
//! `tools/gen_sdl_ttf_testdata.py --aat` (DejaVu Sans subsets, see
//! `testdata/fonts/LICENSE`, and copies of the Noto Sans subsets of the
//! shaping tests, see `testdata/fonts/OFL.txt`): `morx` and `mort` with
//! every subtable type and `feat` and `ltag`, `kerx` with formats 0, 1,
//! 2, 4 and 6 and `ankr` and `trak`, and Apple and OpenType `kern` tables
//! with state-machine subtables, and fonts with both `morx` and GSUB
//! and GPOS. `testdata/aat_reference.txt` is the output of a C program
//! built from upstream SDL_ttf (with its bundled FreeType and HarfBuzz)
//! and SDL3, which runs them in the same order: for each font at two
//! sizes, each case's glyphs shaped by HarfBuzz on FreeType (with the
//! case's features and point size, `hb_buffer_serialize`'s text format
//! with glyph flags), and, for the cases without features and point size,
//! the same text through SDL_ttf (its size, rendering and a text object's
//! drawing); then the shaping and rendering of the font's two corrupt
//! cases with bytes of its AAT tables flipped or set and with those
//! tables truncated.

use std::rc::Rc;

use super::shaping::{direction_of, find_table, tag_str, unescape};
use super::{describe, draw_text, info, open_mem, surface_hash, FG};
use crate::harfbuzz::tests::hb_aat_line;
use crate::{
    harfbuzz_version, string_to_tag, Direction, Font, SurfaceTextEngine, Text, TextEngine,
};

static CASES: &str = include_str!("../testdata/aat_cases.txt");

static FONTS: [(&str, &[u8]); 6] = [
    (
        "DejaVuSans-AAT-morx.ttf",
        include_bytes!("../testdata/fonts/DejaVuSans-AAT-morx.ttf"),
    ),
    (
        "DejaVuSans-AAT-mort.ttf",
        include_bytes!("../testdata/fonts/DejaVuSans-AAT-mort.ttf"),
    ),
    (
        "DejaVuSans-AAT-kerx.ttf",
        include_bytes!("../testdata/fonts/DejaVuSans-AAT-kerx.ttf"),
    ),
    (
        "DejaVuSans-AAT-kernot.ttf",
        include_bytes!("../testdata/fonts/DejaVuSans-AAT-kernot.ttf"),
    ),
    (
        "NotoSans-AAT.ttf",
        include_bytes!("../testdata/fonts/NotoSans-AAT.ttf"),
    ),
    (
        "NotoSans-Unhinted-AAT.ttf",
        include_bytes!("../testdata/fonts/NotoSans-Unhinted-AAT.ttf"),
    ),
];

const SIZES: [f32; 2] = [16.0, 37.0];

/// The AAT tables corrupted, in order.
const AAT_TABLES: [&[u8; 4]; 8] = [
    b"morx", b"mort", b"kerx", b"kern", b"trak", b"ankr", b"feat", b"ltag",
];

fn font_data(name: &str) -> &'static [u8] {
    FONTS
        .iter()
        .find(|(n, _)| *n == name)
        .expect("a test font")
        .1
}

/// SDL_ttf's own shaping of a case (the shaping tests' `ttf_case`, with
/// kerning, without the wrapped rendering and the text object's dump).
fn ttf_aat_case(
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
    out.push(format!(
        "set: {d} {s} {l} dir={} script={:08x} {}",
        font.direction() as i32,
        font.script(),
        tag_str(font.script()),
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
    }
    match Text::new_bytes(Some(engine.clone()), Some(font), text) {
        Ok(t) => draw_text(out, "text", &t, 5, 5),
        Err(e) => out.push(format!("text: err: {e}")),
    }
    let _ = font.set_direction(Direction::Invalid);
    let _ = font.set_script(0);
    let _ = font.set_language(None);
}

fn corrupt_aat_case(out: &mut Vec<String>, label: &str, data: &[u8], cases: &[&str]) {
    /* (the C prints the label, then " | " and each case's line) */
    for (i, case) in cases.iter().enumerate() {
        let f: Vec<&str> = case.splitn(6, '\t').collect();
        let line = hb_aat_line(data, 16.0, &f, &unescape(f[5]));
        out.push(if i == 0 {
            format!("{label}: | {line}")
        } else {
            format!(" | {line}")
        });
    }
    let font = match open_mem(data, 16.0) {
        Ok(font) => font,
        Err(e) => {
            out.push(format!("  ttf: err: {e}"));
            return;
        }
    };
    for (i, case) in cases.iter().enumerate() {
        let text = unescape(case.rsplit('\t').next().unwrap());
        let text = String::from_utf8_lossy(&text);
        let mut line = format!("  ttf {i}: ");
        match font.string_size(&text) {
            Ok((w, h)) => line += &format!("{w} {h} "),
            Err(e) => line += &format!("err: {e} "),
        }
        match font.render_text_blended(&text, FG) {
            Ok(s) => line += &format!("{}x{} {:016x}", s.width(), s.height(), surface_hash(&s)),
            Err(e) => line += &format!("err: {e}"),
        }
        out.push(line);
    }
}

fn corrupt_aat(out: &mut Vec<String>, name: &str, data: &[u8], cases: &[&str]) {
    out.push(format!("== corrupt aat {name}"));
    for table in AAT_TABLES {
        let tname = std::str::from_utf8(table).unwrap();
        let Some((off, tlen)) = find_table(data, table) else {
            continue;
        };
        for i in 0..24 {
            let mut o = off + i * tlen / 24 + i % 7;
            if o >= off + tlen {
                o = off + tlen - 1;
            }
            let mut copy = data.to_vec();
            copy[o] ^= 0xA5;
            corrupt_aat_case(out, &format!("{tname} flip {}", o - off), &copy, cases);
        }
        for i in 0..6 {
            let o = off + i * 2 + if i > 2 { tlen / 3 } else { 0 };
            if o + 1 >= off + tlen {
                continue;
            }
            let mut copy = data.to_vec();
            copy[o] = 0xFF;
            copy[o + 1] = 0xFF;
            corrupt_aat_case(out, &format!("{tname} ffff {}", o - off), &copy, cases);
        }
        corrupt_aat_case(
            out,
            &format!("{tname} trunc"),
            &data[..off + tlen / 2],
            cases,
        );
    }
}

#[test]
fn matches_upstream_aat_reference() {
    crate::init().unwrap();
    let engine: Rc<dyn TextEngine> = SurfaceTextEngine::new().unwrap();
    let mut out = Vec::new();
    let (major, minor, micro) = harfbuzz_version();
    out.push(format!("harfbuzz: {major}.{minor}.{micro}"));

    /* the fonts, their cases and their corrupt cases */
    let mut fonts: Vec<(&str, Vec<&str>, Vec<&str>)> = Vec::new();
    for line in CASES.lines() {
        if let Some(name) = line.strip_prefix("font ") {
            fonts.push((name, Vec::new(), Vec::new()));
        } else if let Some(case) = line.strip_prefix("case\t") {
            fonts.last_mut().unwrap().1.push(case);
        } else if let Some(case) = line.strip_prefix("corrupt\t") {
            fonts.last_mut().unwrap().2.push(case);
        }
    }
    for (name, cases, corrupt) in &fonts {
        let data = font_data(name);
        out.push(format!("== aat {name}"));
        for size in SIZES {
            let font = open_mem(data, size).unwrap();
            out.push(format!("-- size {size}"));
            out.push(format!("info: {}", info(&font)));
            for (c, case) in cases.iter().enumerate() {
                let f: Vec<&str> = case.splitn(6, '\t').collect();
                let text = unescape(f[5]);
                out.push(format!("case {c}: {case}"));
                out.push(hb_aat_line(data, size, &f, &text));
                if f[3] == "-" && f[4] == "-" {
                    ttf_aat_case(&mut out, &font, &engine, &f, &text);
                }
            }
        }
        if corrupt.len() == 2 {
            corrupt_aat(&mut out, name, data, corrupt);
        }
    }
    drop(engine);
    crate::quit();

    let reference: Vec<&str> = include_str!("../testdata/aat_reference.txt")
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
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
