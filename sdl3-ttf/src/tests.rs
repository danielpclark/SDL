// Tests of sdl3-ttf against upstream SDL_ttf's C.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The fonts in `testdata/fonts/` are subsets of DejaVu fonts (see their
//! `LICENSE`) made by `tools/gen_sdl_ttf_testdata.py`.
//! `testdata/reference.txt` is the output of a C program built from
//! upstream SDL_ttf (with its bundled FreeType, without HarfBuzz and
//! PlutoSVG) and SDL3, which runs the cases below in the same order: font
//! information and glyph metrics, kerning, string sizes and measures at
//! several sizes, text rendered in every render mode (solid, shaded,
//! blended, LCD) with every hinting mode, style, a few outlines, without
//! kerning, wrapped and aligned, single glyphs and glyph images, fallback
//! fonts, and the fonts truncated and with flipped bytes. Surfaces are
//! described by their size, format, pitch and FNV-1a hashes of their pixel
//! rows and palette, their color key and blend mode.

use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{Color, Surface};

use crate::{Direction, Font, Hinting, HorizontalAlignment, ImageType};

static SANS: &[u8] = include_bytes!("testdata/fonts/DejaVuSans.ttf");
static SERIF_BOLD: &[u8] = include_bytes!("testdata/fonts/DejaVuSerif-Bold.ttf");
static MONO: &[u8] = include_bytes!("testdata/fonts/DejaVuSansMono.ttf");

const FNV0: u64 = 14695981039346656037;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn surface_hash(s: &Surface<'_>) -> u64 {
    let mut h = FNV0;
    let row = s.width() as usize * s.format().bytes_per_pixel() as usize;
    let Some(pixels) = s.pixels() else {
        return h;
    };
    let pitch = s.pitch() as usize;
    for y in 0..s.height() as usize {
        h = fnv(h, &pixels[y * pitch..y * pitch + row]);
    }
    h
}

/// The harness's `describe()`.
fn describe(s: sdl3::Result<Surface<'static>>) -> String {
    let s = match s {
        Ok(s) => s,
        Err(e) => return format!("err: {e}"),
    };
    let mut out = format!(
        "{}x{} {} pitch={} hash={:016x}",
        s.width(),
        s.height(),
        s.format().name(),
        s.pitch(),
        surface_hash(&s)
    );
    if let Some(p) = s.palette() {
        let p = p.read().unwrap();
        let bytes: Vec<u8> = p
            .colors()
            .iter()
            .flat_map(|c| [c.r, c.g, c.b, c.a])
            .collect();
        out += &format!(" pal={}:{:016x}", p.len(), fnv(FNV0, &bytes));
    }
    if let Some(key) = s.color_key() {
        out += &format!(" key={key}");
    }
    out += &format!(" blend={}", s.blend_mode().0);
    out
}

const FG: Color = Color::new(0x20, 0x40, 0xC0, 0xFF);
const FGA: Color = Color::new(0xC0, 0x30, 0x10, 0x80);
const BG: Color = Color::new(0xF0, 0xE0, 0x10, 0x80);

const TEXT: &str = "The quick brown fox jumps over the lazy dog. AVATAR Wave To été «0123»";
const WRAP: &str =
    "Hello world, this is a wrapped line of text.\nSecond line\r\nThird\tline with  two  spaces.\n\nEnd";
const MODES: [&str; 4] = ["solid", "shaded", "blended", "lcd"];

fn render(f: &Font, mode: usize, text: &str) -> String {
    describe(match mode {
        0 => f.render_text_solid(text, FG),
        1 => f.render_text_shaded(text, FG, BG),
        2 => f.render_text_blended(text, FG),
        _ => f.render_text_lcd(text, FG, BG),
    })
}

fn render_wrapped(f: &Font, mode: usize, text: &str, w: i32) -> String {
    describe(match mode {
        0 => f.render_text_solid_wrapped(text, FG, w),
        1 => f.render_text_shaded_wrapped(text, FG, BG, w),
        2 => f.render_text_blended_wrapped(text, FG, w),
        _ => f.render_text_lcd_wrapped(text, FG, BG, w),
    })
}

fn render_glyph(f: &Font, mode: usize, ch: u32) -> String {
    describe(match mode {
        0 => f.render_glyph_solid(ch, FG),
        1 => f.render_glyph_shaded(ch, FG, BG),
        2 => f.render_glyph_blended(ch, FG),
        _ => f.render_glyph_lcd(ch, FG, BG),
    })
}

const HINTS: [(&str, Hinting); 5] = [
    ("normal", Hinting::Normal),
    ("light", Hinting::Light),
    ("mono", Hinting::Mono),
    ("none", Hinting::None),
    ("light_subpixel", Hinting::LightSubpixel),
];

/// A stream over a copy of `data` (`SDL_IOFromConstMem()` in the C)
fn stream(data: &[u8]) -> IoStream<'static> {
    let mut io = IoStream::from_dynamic_mem();
    assert_eq!(io.write(data), data.len());
    io.seek(0, IoWhence::Set).unwrap();
    io
}

fn open_mem(data: &[u8], size: f32) -> sdl3::Result<Font> {
    Font::open_io(stream(data), size)
}

fn info(f: &Font) -> String {
    let (h, v) = f.dpi();
    format!(
        "family={} style={} size={} dpi={h},{v} height={} ascent={} descent={} lineskip={} fixed={} scalable={} faces={} weight={} kerning={}",
        f.family_name().unwrap_or_default(),
        f.style_name().unwrap_or_default(),
        f.size(),
        f.height(),
        f.ascent(),
        f.descent(),
        f.line_skip(),
        f.is_fixed_width() as i32,
        f.is_scalable() as i32,
        f.num_faces(),
        f.weight(),
        f.kerning() as i32
    )
}

fn res<T>(r: sdl3::Result<T>, ok: impl FnOnce(T) -> String) -> String {
    match r {
        Ok(v) => ok(v),
        Err(e) => format!("err: {e}"),
    }
}

fn metrics(out: &mut Vec<String>, f: &Font, label: &str) {
    for ch in [
        'A' as u32, 'g' as u32, 'W' as u32, 'j' as u32, ' ' as u32, 0xE9, 0x20AC, 0x4E00,
    ] {
        out.push(format!(
            "metrics {label} U+{ch:04X}: has={} {}",
            f.has_glyph(ch) as i32,
            res(f.glyph_metrics(ch), |(a, b, c, d, e)| format!(
                "{a} {b} {c} {d} {e}"
            ))
        ));
    }
    for (a, b) in [
        ('A' as u32, 'V' as u32),
        ('T' as u32, 'o' as u32),
        ('W' as u32, 'a' as u32),
        ('f' as u32, 'j' as u32),
        ('A' as u32, 0x4E00),
    ] {
        out.push(format!(
            "kern {label} U+{a:04X} U+{b:04X}: {}",
            res(f.glyph_kerning(a, b), |k| k.to_string())
        ));
    }
    out.push(format!(
        "string_size {label}: {}",
        res(f.string_size(TEXT), |(w, h)| format!("{w} {h}"))
    ));
    for width in [0, 50, 200] {
        out.push(format!(
            "measure {label} {width}: {}",
            res(f.measure_string(TEXT, width), |(e, c)| format!("{e} {c}"))
        ));
        out.push(format!(
            "string_size_wrapped {label} {width}: {}",
            res(f.string_size_wrapped(WRAP, width), |(w, h)| format!(
                "{w} {h}"
            ))
        ));
    }
}

fn font_cases(out: &mut Vec<String>, name: &str, data: &[u8]) {
    out.push(format!("== {name}"));
    let f = match open_mem(data, 16.5) {
        Ok(f) => f,
        Err(e) => {
            out.push(format!("open: err: {e}"));
            return;
        }
    };
    out.push(format!("info: {}", info(&f)));
    for size in [9.0f32, 16.5, 33.0] {
        f.set_hinting(Hinting::Normal);
        if let Err(e) = f.set_size(size) {
            out.push(format!("set_size: err: {e}"));
        }
        let label = size.to_string();
        out.push(format!("info: {}", info(&f)));
        metrics(out, &f, &label);
        for (hname, hinting) in HINTS {
            f.set_hinting(hinting);
            out.push(format!("hinting {label} {hname}: {}", f.hinting() as i32));
            for (m, mname) in MODES.iter().enumerate() {
                out.push(format!(
                    "render {label} {hname} {mname}: {}",
                    render(&f, m, TEXT)
                ));
            }
        }
    }
    f.set_hinting(Hinting::Normal);
    f.set_size(16.5).unwrap();

    use crate::{STYLE_BOLD, STYLE_ITALIC, STYLE_NORMAL, STYLE_STRIKETHROUGH, STYLE_UNDERLINE};
    for style in [
        STYLE_BOLD,
        STYLE_ITALIC,
        STYLE_UNDERLINE,
        STYLE_STRIKETHROUGH,
        STYLE_BOLD | STYLE_ITALIC | STYLE_UNDERLINE | STYLE_STRIKETHROUGH,
    ] {
        f.set_style(style);
        out.push(format!("style {style}: {}", f.style()));
        out.push(format!("info: {}", info(&f)));
        for (m, mname) in MODES.iter().enumerate() {
            out.push(format!(
                "render style {style} {mname}: {}",
                render(&f, m, TEXT)
            ));
        }
    }
    f.set_style(STYLE_NORMAL);

    for outline in [1, 3] {
        for bold in 0..2 {
            f.set_style(if bold != 0 {
                STYLE_BOLD | STYLE_UNDERLINE
            } else {
                STYLE_NORMAL
            });
            let ok = f.set_outline(outline).is_ok() as i32;
            out.push(format!("outline {outline}: {ok} {}", f.outline()));
            out.push(format!("info: {}", info(&f)));
            for (m, mname) in MODES.iter().enumerate() {
                out.push(format!(
                    "render outline {outline} {bold} {mname}: {}",
                    render(&f, m, TEXT)
                ));
            }
        }
    }
    f.set_outline(0).unwrap();
    f.set_style(STYLE_NORMAL);

    f.set_kerning(false);
    out.push(format!("kerning off: {}", f.kerning() as i32));
    for (m, mname) in MODES.iter().enumerate() {
        out.push(format!("render nokern {mname}: {}", render(&f, m, TEXT)));
    }
    f.set_kerning(true);

    out.push(format!(
        "render blended alpha: {}",
        describe(f.render_text_blended(TEXT, FGA))
    ));
    out.push(format!(
        "render shaded alpha: {}",
        describe(f.render_text_shaded(TEXT, FGA, BG))
    ));
    out.push(format!(
        "render solid len 10: {}",
        describe(f.render_text_solid(&TEXT[..10], FG))
    ));
    out.push(format!(
        "render empty: {}",
        describe(f.render_text_blended("", FG))
    ));
    out.push(format!(
        "render space: {}",
        describe(f.render_text_blended(" ", FG))
    ));
    out.push(format!(
        "render newline: {}",
        describe(f.render_text_blended("a\nb", FG))
    ));
    out.push(format!(
        "render nul: {}",
        describe(f.render_text_blended("a\0b", FG))
    ));
    out.push(format!(
        "render nul shaded: {}",
        describe(f.render_text_shaded("\0ab", FG, BG))
    ));

    for w in [0, 80, 200] {
        for (m, mname) in MODES.iter().enumerate() {
            out.push(format!(
                "wrapped {w} {mname}: {}",
                render_wrapped(&f, m, WRAP, w)
            ));
        }
    }
    for align in [
        HorizontalAlignment::Center,
        HorizontalAlignment::Right,
        HorizontalAlignment::Left,
    ] {
        f.set_wrap_alignment(align);
        out.push(format!(
            "wrapped align {}: {} {}",
            align as i32,
            f.wrap_alignment() as i32,
            render_wrapped(&f, 2, WRAP, 150)
        ));
    }
    f.set_line_skip(30);
    out.push(format!(
        "wrapped lineskip 30: {} {}",
        f.line_skip(),
        render_wrapped(&f, 1, WRAP, 150)
    ));
    f.set_line_skip(3);
    out.push(format!(
        "wrapped lineskip 3: {} {}",
        f.line_skip(),
        render_wrapped(&f, 2, WRAP, 150)
    ));
    f.set_style(STYLE_UNDERLINE | STYLE_STRIKETHROUGH);
    out.push(format!(
        "wrapped underline lineskip 3: {}",
        render_wrapped(&f, 2, WRAP, 150)
    ));
    f.set_style(STYLE_NORMAL);
    f.set_size(16.5).unwrap();
    out.push(format!("lineskip after set_size: {}", f.line_skip()));

    for ch in ['g' as u32, 0xE9, 0x20AC, 0x4E00] {
        for (m, mname) in MODES.iter().enumerate() {
            out.push(format!(
                "glyph U+{ch:04X} {mname}: {}",
                render_glyph(&f, m, ch)
            ));
        }
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
    for gi in 0..4 {
        let (ty, s) = match f.glyph_image_for_index(gi) {
            Ok((s, ty)) => (ty, Ok(s)),
            Err(e) => (ImageType::Invalid, Err(e)),
        };
        out.push(format!(
            "glyph_image_for_index {gi}: type={} {}",
            ty as i32,
            describe(s)
        ));
    }
    out.push(format!(
        "glyph_image_for_index 100000: {}",
        describe(f.glyph_image_for_index(100000).map(|(s, _)| s))
    ));

    let r = f.set_direction(Direction::Rtl);
    out.push(format!(
        "direction rtl: {} {} {}",
        r.is_ok() as i32,
        f.direction() as i32,
        r.err().map(|e| e.to_string()).unwrap_or_default()
    ));
    let r = f.set_direction(Direction::Ltr);
    out.push(format!(
        "direction ltr: {} {}",
        r.is_ok() as i32,
        f.direction() as i32
    ));
    let r = f.set_script(crate::string_to_tag(Some("Latn")));
    out.push(format!(
        "script: {} {}",
        r.is_ok() as i32,
        r.err().map(|e| e.to_string()).unwrap_or_default()
    ));
    let r = f.set_language(Some("en"));
    out.push(format!(
        "language: {} {}",
        r.is_ok() as i32,
        r.err().map(|e| e.to_string()).unwrap_or_default()
    ));

    let ok = f.set_size_dpi(12.0, 96, 144).is_ok() as i32;
    out.push(format!("sizedpi: {ok} info: {}", info(&f)));
    out.push(format!(
        "render dpi: {}",
        describe(f.render_text_blended(TEXT, FG))
    ));

    match f.copy() {
        Ok(copy) => {
            out.push(format!("copy: info: {}", info(&copy)));
            out.push(format!(
                "render copy: {}",
                describe(copy.render_text_shaded(TEXT, FG, BG))
            ));
        }
        Err(e) => out.push(format!("copy: err: {e}")),
    }
    f.set_size(16.5).unwrap();

    let fb = open_mem(MONO, 16.5).unwrap();
    let fbtext = "a€…←一z";
    out.push(format!(
        "fallback add: {}",
        f.add_fallback_font(&fb).is_ok() as i32
    ));
    for (m, mname) in MODES.iter().enumerate() {
        out.push(format!(
            "render fallback {mname}: {}",
            render(&f, m, fbtext)
        ));
    }
    out.push(format!(
        "metrics fallback U+20AC: {}",
        res(f.glyph_metrics(0x20AC), |(a, b, c, d, e)| format!(
            "{a} {b} {c} {d} {e}"
        ))
    ));
    fb.set_size(25.0).unwrap();
    out.push(format!(
        "render fallback resized: {}",
        describe(f.render_text_blended(fbtext, FG))
    ));
    f.remove_fallback_font(&fb);
    out.push(format!(
        "render fallback removed: {}",
        describe(f.render_text_blended(fbtext, FG))
    ));
}

fn corrupt_case(out: &mut Vec<String>, label: &str, data: &[u8]) {
    let f = match open_mem(data, 16.5) {
        Ok(f) => f,
        Err(e) => {
            out.push(format!("{label}: err: {e}"));
            return;
        }
    };
    let head = format!(
        "{label}: ok {} {} {} {} | ",
        if f.family_name().is_some() {
            "named"
        } else {
            "noname"
        },
        if f.style_name().is_some() {
            "styled"
        } else {
            "nostyle"
        },
        f.height(),
        f.line_skip()
    );
    let tail = match f.render_text_blended("Hamburgefonstiv AVA", FG) {
        Ok(s) => format!("{}x{} {:016x}", s.width(), s.height(), surface_hash(&s)),
        Err(e) => format!("err: {e}"),
    };
    out.push(head + &tail);
}

fn corrupt_cases(out: &mut Vec<String>, name: &str, data: &[u8]) {
    out.push(format!("== corrupt {name}"));
    let len = data.len();
    let cuts = [
        0,
        1,
        4,
        12,
        16,
        100,
        200,
        300,
        600,
        1000,
        2000,
        4000,
        8000,
        len / 2,
        len - 100,
        len - 1,
    ];
    for cut in cuts {
        corrupt_case(out, &format!("trunc {cut}"), &data[..cut]);
    }
    let mut copy = data.to_vec();
    for i in 0..48usize {
        let mut off = if i < 24 {
            i * 12 + 4
        } else {
            ((i - 24) as u64 * len as u64 / 24) as usize + 7 * i
        };
        if off >= len {
            off = len - 1;
        }
        copy.copy_from_slice(data);
        copy[off] ^= 0xA5;
        corrupt_case(out, &format!("flip {off}"), &copy);
    }
}

#[test]
fn matches_upstream_reference() {
    crate::init().unwrap();
    let mut out = Vec::new();
    for (name, data) in [
        ("DejaVuSans.ttf", SANS),
        ("DejaVuSerif-Bold.ttf", SERIF_BOLD),
        ("DejaVuSansMono.ttf", MONO),
    ] {
        font_cases(&mut out, name, data);
        corrupt_cases(&mut out, name, data);
    }
    crate::quit();

    let reference: Vec<&str> = include_str!("testdata/reference.txt").lines().collect();
    let mut failures = Vec::new();
    for (i, expected) in reference.iter().enumerate() {
        let actual = out.get(i).map(String::as_str).unwrap_or("<missing>");
        // FreeType's light hinting is its auto-hinter's (autofit), which
        // isn't translated yet
        let autofit = expected.starts_with("render ")
            && (expected.contains(" light ") || expected.contains(" light_subpixel "));
        if actual != *expected && !autofit {
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
