// Tests of sdl3-ttf against upstream SDL_ttf's C.
// Copyright (C) 2001-2025 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The fonts in `testdata/fonts/` are subsets of DejaVu fonts (see their
//! `LICENSE`), and OpenType/CFF, CFF2 variable and bare CFF versions of
//! the DejaVu Sans subset, and fonts of FreeType's other formats made from
//! them (Windows FNT and FON, BDF, and PCF bitmap fonts, the latter also
//! compressed, and PFR fonts), made by `tools/gen_sdl_ttf_testdata.py`, and the Noto
//! font subsets (see their `OFL.txt`) of the shaping tests in
//! `tests/shaping.rs`.
//! `testdata/reference.txt` is the output of a C program built from
//! upstream SDL_ttf (with its bundled FreeType and HarfBuzz, without
//! PlutoSVG) and SDL3, which runs the cases below in the same order: font
//! information and glyph metrics, kerning, string sizes and measures at
//! several sizes, text rendered in every render mode (solid, shaded,
//! blended, LCD) with every hinting mode, style, a few outlines, without
//! kerning, wrapped and aligned, single glyphs and glyph images, fallback
//! fonts, text objects (layouts, clusters, substrings and edits) drawn
//! with the surface and renderer (software, with several atlas sizes) text
//! engines, signed distance field rendering, the CFF2 font's named
//! instances, the faces of fonts with several, and the fonts truncated
//! and with flipped bytes. Surfaces are
//! described by their size, format, pitch and FNV-1a hashes of their pixel
//! rows and palette, their color key and blend mode.

use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{Color, Surface};

use std::cell::RefCell;
use std::rc::Rc;

use sdl3::render::Renderer;

use crate::{
    draw_renderer_text, draw_surface_text, gpu_text_draw_data, Direction, DrawOperation, Font,
    GpuTextEngine, GpuTextEngineWinding, Hinting, HorizontalAlignment, ImageType,
    RendererTextEngine, SubString, SurfaceTextEngine, Text, TextEngine,
};

static SANS: &[u8] = include_bytes!("testdata/fonts/DejaVuSans.ttf");
static SERIF_BOLD: &[u8] = include_bytes!("testdata/fonts/DejaVuSerif-Bold.ttf");
static MONO: &[u8] = include_bytes!("testdata/fonts/DejaVuSansMono.ttf");
static SANS_CFF: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-CFF.otf");
static SANS_CFF2: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-CFF2.otf");
static SANS_BARE_CFF: &[u8] = include_bytes!("testdata/fonts/DejaVuSans.cff");
static SANS_FNT: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-13.fnt");
static SANS_FON: &[u8] = include_bytes!("testdata/fonts/DejaVuSans.fon");
static SANS_PE_FON: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-PE.fon");
static SANS_BDF: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-13.bdf");
static SANS_BDF_GRAY: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-13-4bpp.bdf");
static SANS_PCF: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-13.pcf");
static SANS_PCF_LSB: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-13-lsb.pcf");
static SANS_PCF_GZ: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-13.pcf.gz");
static SANS_PCF_GZ_NOSIZE: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-13-nosize.pcf.gz");
static SANS_PCF_Z: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-13.pcf.Z");
static SANS_PCF_BZ2: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-13.pcf.bz2");
static SANS_PFR: &[u8] = include_bytes!("testdata/fonts/DejaVuSans.pfr");
static SANS_PFR_BITMAP: &[u8] = include_bytes!("testdata/fonts/DejaVuSans-bitmap.pfr");

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
        /* (glibc's printf prints a null string as "(null)") */
        f.family_name().unwrap_or_else(|| "(null)".into()),
        f.style_name().unwrap_or_else(|| "(null)".into()),
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

/// C's `%#x`
fn hex(v: u32) -> String {
    if v == 0 {
        "0".into()
    } else {
        format!("{v:#x}")
    }
}

fn sub_str(s: &SubString) -> String {
    format!(
        "{} {} {} {} {} {},{},{},{}",
        hex(s.flags),
        s.offset,
        s.length,
        s.line_index,
        s.cluster_index,
        s.rect.x,
        s.rect.y,
        s.rect.w,
        s.rect.h
    )
}

fn print_sub(out: &mut Vec<String>, label: &str, s: sdl3::Result<SubString>) {
    match s {
        Ok(s) => out.push(format!("{label}: {}", sub_str(&s))),
        Err(e) => out.push(format!("{label}: err: {e}")),
    }
}

fn escaped(s: Option<Vec<u8>>) -> String {
    let Some(s) = s else {
        return "(null)".into();
    };
    let mut out = String::new();
    for c in s {
        match c {
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            c if !(0x20..0x7f).contains(&c) => out.push_str(&format!("\\x{c:02x}")),
            c => out.push(c as char),
        }
    }
    out
}

fn dump_text(out: &mut Vec<String>, label: &str, t: &Text, main_font: &Font) {
    let r = t.size();
    let (ok, w, h) = match &r {
        Ok((w, h)) => (1, *w, *h),
        Err(_) => (0, 0, 0),
    };
    out.push(format!(
        "{label} size: {ok} {w} {h} lines={} text={}",
        t.num_lines(),
        escaped(t.text_bytes())
    ));
    if let Err(e) = r {
        out.push(format!("  err: {e}"));
        return;
    }
    t.with_data(|d| {
        let mut line = format!("{label} ops: {}", d.ops().len());
        for op in d.ops() {
            match op {
                DrawOperation::Fill(f) => {
                    line += &format!(" | F {},{},{},{}", f.rect.x, f.rect.y, f.rect.w, f.rect.h)
                }
                DrawOperation::Copy(c) => {
                    line += &format!(
                        " | C {} {} {} {},{},{},{} {},{},{},{}",
                        c.text_offset,
                        if c.glyph_font.is(main_font) { 0 } else { 1 },
                        c.glyph_index,
                        c.src.x,
                        c.src.y,
                        c.src.w,
                        c.src.h,
                        c.dst.x,
                        c.dst.y,
                        c.dst.w,
                        c.dst.h
                    )
                }
                DrawOperation::Noop => line += " | N",
            }
        }
        out.push(line);
        let mut line = format!("{label} clusters: {}", d.clusters().len());
        for s in d.clusters() {
            line += &format!(" | {}", sub_str(s));
        }
        out.push(line);
    });
    let len = t.text_bytes().map_or(0, |t| t.len()) as i32;
    for offset in [-1, 0, 1, 2, 5, len / 2, len - 1, len, len + 3] {
        print_sub(out, &format!("{label} sub {offset}"), t.substring(offset));
        if let Ok(sub) = t.substring(offset) {
            print_sub(
                out,
                &format!("{label} prev {offset}"),
                t.previous_substring(&sub),
            );
            print_sub(
                out,
                &format!("{label} next {offset}"),
                t.next_substring(&sub),
            );
        }
    }
    for line in -1..=t.num_lines() {
        print_sub(
            out,
            &format!("{label} line {line}"),
            t.substring_for_line(line),
        );
    }
    for (a, b) in [(0, -1), (0, 0), (3, 4), (2, 30), (len - 2, 5), (-1, 3)] {
        let mut line;
        match t.substrings_for_range(a, b) {
            Ok(subs) => {
                line = format!("{label} range {a} {b}: {}", subs.len());
                for s in &subs {
                    line += &format!(" | {}", sub_str(s));
                }
            }
            Err(e) => line = format!("{label} range {a} {b}: 0 err: {e}"),
        }
        out.push(line);
    }
    let mut line = format!("{label} points:");
    let mut y = -5;
    while y < h + 10 {
        let mut x = -5;
        while x < w + 10 {
            match t.substring_for_point(x, y) {
                Ok(s) => line += &format!(" {}", s.offset),
                Err(_) => line += " e",
            }
            x += 11;
        }
        y += 7;
    }
    out.push(line);
}

fn draw_text(out: &mut Vec<String>, label: &str, t: &Text, x: i32, y: i32) {
    let mut s = Surface::new(220, 160, sdl3::video::PixelFormat::ARGB8888).unwrap();
    let c = s.map_rgba(0x10, 0x20, 0x30, 0xFF);
    s.fill_rect(None, c).unwrap();
    match draw_surface_text(t, x, y, &mut s) {
        Ok(()) => out.push(format!("{label} draw: 1 {}", describe(Ok(s)))),
        Err(e) => out.push(format!("{label} draw: 0 err: {e}")),
    }
}

fn text_cases(out: &mut Vec<String>, name: &str, data: &[u8]) {
    use crate::{STYLE_NORMAL, STYLE_STRIKETHROUGH, STYLE_UNDERLINE};

    out.push(format!("== text {name}"));
    let f = open_mem(data, 16.5).unwrap();
    let fb = open_mem(MONO, 14.0).unwrap();
    let engine: Rc<dyn TextEngine> = SurfaceTextEngine::new().unwrap();
    let t = Text::new(Some(engine.clone()), Some(&f), WRAP).unwrap();
    dump_text(out, "plain", &t, &f);
    draw_text(out, "plain", &t, 3, 4);
    t.set_color(0xC0, 0x40, 0x20, 0xFF).unwrap();
    t.set_wrap_width(150).unwrap();
    dump_text(out, "wrap150", &t, &f);
    draw_text(out, "wrap150", &t, 0, 0);
    t.set_wrap_whitespace_visible(true).unwrap();
    dump_text(out, "wrapws", &t, &f);
    t.set_wrap_whitespace_visible(false).unwrap();
    t.set_position(5, 7).unwrap();
    dump_text(out, "pos", &t, &f);
    draw_text(out, "pos", &t, -3, 2);
    t.set_color_float(0.25, 0.5, 1.5, 0.5).unwrap();
    {
        let (r, g, b, a) = t.color();
        let (fr, fg, fb_, fa) = t.color_float();
        out.push(format!("color: {r} {g} {b} {a} {fr} {fg} {fb_} {fa}"));
    }
    draw_text(out, "halfalpha", &t, 1, 1);
    t.set_position(0, 0).unwrap();
    t.set_color(0xFF, 0xFF, 0xFF, 0xFF).unwrap();
    f.set_style(STYLE_UNDERLINE | STYLE_STRIKETHROUGH);
    dump_text(out, "styled", &t, &f);
    draw_text(out, "styled", &t, 0, 0);
    f.set_style(STYLE_NORMAL);
    f.set_outline(2).unwrap();
    dump_text(out, "outline", &t, &f);
    draw_text(out, "outline", &t, 0, 0);
    f.set_outline(0).unwrap();
    f.set_size(12.0).unwrap();
    dump_text(out, "resized", &t, &f);
    draw_text(out, "resized", &t, 0, 0);
    f.set_wrap_alignment(HorizontalAlignment::Center);
    dump_text(out, "center", &t, &f);
    f.set_wrap_alignment(HorizontalAlignment::Right);
    draw_text(out, "right", &t, 0, 0);
    f.set_wrap_alignment(HorizontalAlignment::Left);

    out.push(format!(
        "insert: {}",
        t.insert_string(6, "big ").is_ok() as i32
    ));
    dump_text(out, "inserted", &t, &f);
    out.push(format!(
        "append: {}",
        t.append_string(" tail€").is_ok() as i32
    ));
    out.push(format!(
        "insert neg: {}",
        t.insert_string(-3, "XY").is_ok() as i32
    ));
    out.push(format!(
        "insert far: {}",
        t.insert_string(1000, "!").is_ok() as i32
    ));
    out.push(format!("delete: {}", t.delete_string(3, 5).is_ok() as i32));
    out.push(format!(
        "delete neg: {}",
        t.delete_string(-4, 2).is_ok() as i32
    ));
    dump_text(out, "edited", &t, &f);
    f.add_fallback_font(&fb).unwrap();
    dump_text(out, "fallback", &t, &f);
    draw_text(out, "fallback", &t, 0, 0);
    out.push(format!(
        "delete tail: {}",
        t.delete_string(20, -1).is_ok() as i32
    ));
    dump_text(out, "cut", &t, &f);
    out.push(format!(
        "set string: {}",
        t.set_string(Some("One\n\nThree  ")).is_ok() as i32
    ));
    t.set_wrap_width(0).unwrap();
    dump_text(out, "lines", &t, &f);
    draw_text(out, "lines", &t, 0, 0);
    out.push(format!(
        "set empty: {}",
        t.set_string(Some("")).is_ok() as i32
    ));
    dump_text(out, "empty", &t, &f);
    draw_text(out, "empty", &t, 0, 0);
    out.push(format!(
        "set string: {}",
        t.set_string(Some("abc")).is_ok() as i32
    ));
    out.push(format!(
        "direction rtl: {}",
        t.set_direction(Direction::Rtl).is_ok() as i32
    ));
    out.push(format!(
        "direction: {} script: {}",
        t.direction() as i32,
        t.script()
    ));
    out.push(format!(
        "set font null: {}",
        t.set_font(None).is_ok() as i32
    ));
    dump_text(out, "nofont", &t, &f);
    out.push(format!(
        "set font fb: {}",
        t.set_font(Some(&fb)).is_ok() as i32
    ));
    dump_text(out, "fbfont", &t, &fb);
    draw_text(out, "fbfont", &t, 10, 10);
    let t2 = Text::new(None, Some(&f), "no engine").unwrap();
    dump_text(out, "noengine", &t2, &f);
    drop(t2);
    let t3 = Text::new(Some(engine.clone()), Some(&f), "").unwrap();
    dump_text(out, "emptynew", &t3, &f);
    drop(t3);
    drop(fb);
    dump_text(out, "closedfb", &t, &f);
    drop(t);
    drop(engine);
    drop(f);
}

fn rdraw(out: &mut Vec<String>, label: &str, r: &Rc<RefCell<Renderer>>, t: &Text, x: f32, y: f32) {
    {
        let mut r = r.borrow_mut();
        r.set_draw_color(0x10, 0x20, 0x30, 0xFF);
        let _ = r.clear();
    }
    match draw_renderer_text(t, x, y) {
        Ok(()) => {
            let s = r.borrow_mut().read_pixels(None);
            out.push(format!("{label} rdraw: 1 {}", describe(s)));
        }
        Err(e) => out.push(format!("{label} rdraw: 0 err: {e}")),
    }
}

/// Texts whose glyphs don't overlap: the order of drawing glyphs from
/// different atlases follows the atlases' addresses in upstream.
const SPACED1: &str = "H e l l o  w o r l d ,  t h i s  i s\nw r a p p e d  t e x t .\n\nE n d";
const SPACED2: &str =
    "T h e  q u i c k  b r o w n  f o x  j u m p s  0 1 2 3 4 5 6 7 8 9  A V W é « »";

#[allow(clippy::too_many_arguments)]
fn renderer_pass(
    out: &mut Vec<String>,
    tag: &str,
    r: &Rc<RefCell<Renderer>>,
    engine: Rc<dyn TextEngine>,
    f: &Font,
    fb: &Font,
    text1: &str,
    text2: &str,
) {
    let spaced = text1 == SPACED1;
    use crate::{STYLE_NORMAL, STYLE_STRIKETHROUGH, STYLE_UNDERLINE};

    let l = |s: &str| format!("{tag} {s}");
    f.set_size(16.5).unwrap();
    let t = Text::new(Some(engine.clone()), Some(f), text1);
    let t2 = Text::new(Some(engine.clone()), Some(f), text2);
    out.push(format!(
        "{tag} create: {} {}",
        t.is_ok() as i32,
        t2.is_ok() as i32
    ));
    let (t, t2) = (t.unwrap(), t2.unwrap());
    rdraw(out, &l("plain"), r, &t, 3.0, 4.0);
    t2.set_wrap_width(200).unwrap();
    rdraw(out, &l("second"), r, &t2, 0.0, 0.0);
    t.set_color(0xC0, 0x40, 0x20, 0xFF).unwrap();
    t.set_wrap_width(150).unwrap();
    rdraw(out, &l("wrap150"), r, &t, 0.0, 0.0);
    t.set_position(5, 7).unwrap();
    rdraw(out, &l("pos"), r, &t, -3.5, 2.25);
    t.set_color_float(0.25, 0.5, 1.5, 0.5).unwrap();
    rdraw(out, &l("halfalpha"), r, &t, 1.0, 1.0);
    t.set_position(0, 0).unwrap();
    t.set_color(0xFF, 0xFF, 0xFF, 0xFF).unwrap();
    f.set_style(STYLE_UNDERLINE | STYLE_STRIKETHROUGH);
    rdraw(out, &l("styled"), r, &t, 0.0, 0.0);
    f.set_style(STYLE_NORMAL);
    f.set_outline(2).unwrap();
    rdraw(out, &l("outline"), r, &t, 0.0, 0.0);
    if !spaced {
        // (the outlined glyphs of the second text overlap)
        rdraw(out, &l("outline2"), r, &t2, 0.5, 0.5);
    }
    f.set_outline(0).unwrap();
    rdraw(out, &l("again"), r, &t, 0.0, 0.0);
    f.set_size(12.0).unwrap();
    rdraw(out, &l("resized"), r, &t, 0.0, 0.0);
    rdraw(out, &l("resized2"), r, &t2, 0.0, 0.0);
    f.set_size(16.5).unwrap();
    rdraw(out, &l("back"), r, &t, 0.0, 0.0);
    rdraw(out, &l("back2"), r, &t2, 2.0, 3.0);
    f.add_fallback_font(fb).unwrap();
    if spaced {
        t.append_string(" t a i l € ← ☺").unwrap();
    } else {
        t.append_string(" tail€ ←☺").unwrap();
    }
    rdraw(out, &l("fallback"), r, &t, 0.0, 0.0);
    f.remove_fallback_font(fb);
    t.set_string(Some("")).unwrap();
    rdraw(out, &l("empty"), r, &t, 0.0, 0.0);
    t.set_font(Some(fb)).unwrap();
    t.set_string(Some("abc def")).unwrap();
    rdraw(out, &l("fbfont"), r, &t, 10.0, 10.0);
    t.set_font(None).unwrap();
    rdraw(out, &l("nofont"), r, &t, 0.0, 0.0);
    drop(t2);
    drop(t);
}

fn renderer_cases(out: &mut Vec<String>, name: &str, data: &[u8]) {
    out.push(format!("== renderer {name}"));
    let f = open_mem(data, 16.5).unwrap();
    let fb = open_mem(MONO, 14.0).unwrap();
    let surface = Surface::new(220, 160, sdl3::video::PixelFormat::ARGB8888).unwrap();
    let r = Rc::new(RefCell::new(Renderer::software(surface).unwrap()));
    let engine: Rc<dyn TextEngine> = RendererTextEngine::new(r.clone()).unwrap();
    renderer_pass(out, "big", &r, engine.clone(), &f, &fb, WRAP, TEXT);
    {
        // a text of another engine
        let se: Rc<dyn TextEngine> = SurfaceTextEngine::new().unwrap();
        let st = Text::new(Some(se), Some(&f), "x").unwrap();
        rdraw(out, "surfaceengine", &r, &st, 0.0, 0.0);
    }
    drop(engine);
    let mut size = 32;
    while size <= 128 {
        let tag = format!("atlas{size}");
        let engine = RendererTextEngine::with_atlas_texture_size(r.clone(), size);
        out.push(format!("{tag} engine: {}", engine.is_ok() as i32));
        renderer_pass(out, &tag, &r, engine.unwrap(), &f, &fb, SPACED1, SPACED2);
        size *= 2;
    }
    match RendererTextEngine::with_atlas_texture_size(r.clone(), 0) {
        Ok(_) => out.push("atlas0 engine: 1 err: ".into()),
        Err(e) => out.push(format!("atlas0 engine: 0 err: {e}")),
    }
    let engine: Rc<dyn TextEngine> =
        RendererTextEngine::with_atlas_texture_size(r.clone(), 8).unwrap();
    {
        f.set_size(16.5).unwrap();
        let t = Text::new(Some(engine.clone()), Some(&f), "Wide").unwrap();
        rdraw(out, "atlas8", &r, &t, 0.0, 0.0);
    }
    drop(engine);
    drop(r);
    drop(fb);
    drop(f);
}

fn sdf_cases(out: &mut Vec<String>, name: &str, data: &[u8]) {
    use crate::{STYLE_BOLD, STYLE_ITALIC, STYLE_NORMAL, STYLE_STRIKETHROUGH, STYLE_UNDERLINE};

    out.push(format!("== sdf {name}"));
    let f = open_mem(data, 16.5).unwrap();
    let ok = f.set_sdf(true).is_ok() as i32;
    out.push(format!("sdf set: {ok} get: {}", f.sdf() as i32));
    out.push(format!(
        "sdf string_size: {}",
        res(f.string_size(TEXT), |(w, h)| format!("{w} {h}"))
    ));
    out.push(format!(
        "sdf blended: {}",
        describe(f.render_text_blended(TEXT, FG))
    ));
    out.push(format!(
        "sdf blended wrapped: {}",
        describe(f.render_text_blended_wrapped(WRAP, FGA, 150))
    ));
    for ch in ['A' as u32, 'g' as u32, 0xE9, 0x20AC] {
        let (ty, s) = match f.glyph_image(ch) {
            Ok((s, ty)) => (ty, Ok(s)),
            Err(e) => (ImageType::Invalid, Err(e)),
        };
        out.push(format!(
            "sdf glyph_image U+{ch:04X}: type={} {}",
            ty as i32,
            describe(s)
        ));
        out.push(format!(
            "sdf glyph blended U+{ch:04X}: {}",
            describe(f.render_glyph_blended(ch, FG))
        ));
    }
    for size in [9.0f32, 33.0] {
        let _ = f.set_size(size);
        out.push(format!(
            "sdf blended {size}: {}",
            describe(f.render_text_blended(TEXT, FG))
        ));
    }
    let _ = f.set_size(16.5);
    for style in [
        STYLE_BOLD,
        STYLE_ITALIC,
        STYLE_UNDERLINE | STYLE_STRIKETHROUGH,
    ] {
        f.set_style(style);
        out.push(format!(
            "sdf style {style}: {}",
            describe(f.render_text_blended(TEXT, FG))
        ));
    }
    f.set_style(STYLE_NORMAL);
    let _ = f.set_outline(1);
    out.push(format!(
        "sdf outline: {}",
        describe(f.render_text_blended(TEXT, FG))
    ));
    let _ = f.set_outline(0);
    f.set_hinting(Hinting::None);
    out.push(format!(
        "sdf nohinting: {}",
        describe(f.render_text_blended(TEXT, FG))
    ));
    f.set_hinting(Hinting::Light);
    out.push(format!(
        "sdf light: {}",
        describe(f.render_text_blended(TEXT, FG))
    ));
    f.set_hinting(Hinting::Normal);
    {
        let engine: Rc<dyn TextEngine> = SurfaceTextEngine::new().unwrap();
        let t = Text::new(Some(engine), Some(&f), WRAP).unwrap();
        t.set_wrap_width(150).unwrap();
        dump_text(out, "sdftext", &t, &f);
        draw_text(out, "sdftext", &t, 2, 3);
    }
    {
        let surface = Surface::new(220, 160, sdl3::video::PixelFormat::ARGB8888).unwrap();
        let r = Rc::new(RefCell::new(Renderer::software(surface).unwrap()));
        let engine: Rc<dyn TextEngine> = RendererTextEngine::new(r.clone()).unwrap();
        let t = Text::new(Some(engine), Some(&f), WRAP).unwrap();
        t.set_wrap_width(150).unwrap();
        rdraw(out, "sdfrenderer", &r, &t, 1.0, 2.0);
    }
    out.push(format!(
        "sdf solid: {}",
        describe(f.render_text_solid(TEXT, FG))
    ));
    out.push(format!("sdf after solid: {}", f.sdf() as i32));
    out.push(format!(
        "sdf blended after: {}",
        describe(f.render_text_blended(TEXT, FG))
    ));
    let ok = f.set_sdf(true).is_ok() as i32;
    out.push(format!("sdf set again: {ok} get: {}", f.sdf() as i32));
    out.push(format!(
        "sdf shaded: {}",
        describe(f.render_text_shaded(TEXT, FG, BG))
    ));
    out.push(format!("sdf after shaded: {}", f.sdf() as i32));
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
        let cut = cut.min(len);
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

/// The CFF2 variable font's named instances (face index `instance << 16`).
fn cff_cases(out: &mut Vec<String>, name: &str, data: &[u8]) {
    out.push(format!("== instances {name}"));
    for inst in 0..=4i64 {
        let f = Font::open_with(crate::FontOptions {
            iostream: Some(stream(data)),
            size: 20.0,
            face: Some(inst << 16),
            ..Default::default()
        });
        let f = match f {
            Ok(f) => f,
            Err(e) => {
                out.push(format!("instance {inst}: err: {e}"));
                continue;
            }
        };
        out.push(format!("instance {inst}: info: {}", info(&f)));
        metrics(out, &f, "instance");
        for (hname, hinting) in HINTS {
            f.set_hinting(hinting);
            for (m, mname) in MODES.iter().enumerate() {
                out.push(format!(
                    "render instance {inst} {hname} {mname}: {}",
                    render(&f, m, TEXT)
                ));
            }
        }
    }
}

/// The faces of a font with several (face index `face`).
fn faces_cases(out: &mut Vec<String>, name: &str, data: &[u8]) {
    out.push(format!("== faces {name}"));
    for face in 0..=2i64 {
        let f = Font::open_with(crate::FontOptions {
            iostream: Some(stream(data)),
            size: 20.0,
            face: Some(face),
            ..Default::default()
        });
        let f = match f {
            Ok(f) => f,
            Err(e) => {
                out.push(format!("face {face}: err: {e}"));
                continue;
            }
        };
        out.push(format!("face {face}: info: {}", info(&f)));
        metrics(out, &f, "face");
        for (m, mname) in MODES.iter().enumerate() {
            out.push(format!(
                "render face {face} {mname}: {}",
                render(&f, m, TEXT)
            ));
        }
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
        ("DejaVuSans-CFF.otf", SANS_CFF),
        ("DejaVuSans-CFF2.otf", SANS_CFF2),
        ("DejaVuSans.cff", SANS_BARE_CFF),
        ("DejaVuSans-13.fnt", SANS_FNT),
        ("DejaVuSans.fon", SANS_FON),
        ("DejaVuSans-PE.fon", SANS_PE_FON),
        ("DejaVuSans-13.bdf", SANS_BDF),
        ("DejaVuSans-13-4bpp.bdf", SANS_BDF_GRAY),
        ("DejaVuSans-13.pcf", SANS_PCF),
        ("DejaVuSans-13-lsb.pcf", SANS_PCF_LSB),
        ("DejaVuSans-13.pcf.gz", SANS_PCF_GZ),
        ("DejaVuSans-13-nosize.pcf.gz", SANS_PCF_GZ_NOSIZE),
        ("DejaVuSans-13.pcf.Z", SANS_PCF_Z),
        ("DejaVuSans-13.pcf.bz2", SANS_PCF_BZ2),
        ("DejaVuSans.pfr", SANS_PFR),
        ("DejaVuSans-bitmap.pfr", SANS_PFR_BITMAP),
    ] {
        /* (the other fonts: the font cases, the CFF2 font's named
        instances, the faces of fonts with several, and the corrupt
        fonts) */
        let full = name.ends_with(".ttf") || name.ends_with("CFF.otf");
        font_cases(&mut out, name, data);
        if full {
            text_cases(&mut out, name, data);
            renderer_cases(&mut out, name, data);
            sdf_cases(&mut out, name, data);
        }
        if name.contains("CFF2") {
            cff_cases(&mut out, name, data);
        }
        if name.contains(".fon") || name.contains(".pfr") {
            faces_cases(&mut out, name, data);
        }
        corrupt_cases(&mut out, name, data);
    }
    crate::quit();

    let reference: Vec<&str> = include_str!("testdata/reference.txt").lines().collect();
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

/// Atlas textures smaller than 4x4 pixels have no packing nodes, which
/// upstream divides by; this fails instead.
#[test]
fn tiny_renderer_atlas() {
    crate::init().unwrap();
    let f = open_mem(SANS, 2.0).unwrap();
    let surface = Surface::new(16, 16, sdl3::video::PixelFormat::ARGB8888).unwrap();
    let r = Rc::new(RefCell::new(Renderer::software(surface).unwrap()));
    let (w, h) = f
        .glyph_image(u32::from('.'))
        .map(|(s, _)| (s.width(), s.height()))
        .unwrap();
    assert!(w > 0 && w < 4 && h > 0 && h < 4, "{w}x{h}");
    for size in 1..4 {
        let engine: Rc<dyn TextEngine> =
            RendererTextEngine::with_atlas_texture_size(r.clone(), size).unwrap();
        let t = Text::new(Some(engine), Some(&f), ".").unwrap();
        let err = draw_renderer_text(&t, 0.0, 0.0).unwrap_err().to_string();
        if w <= size && h <= size {
            assert!(err.contains("Invalid texture atlas size"), "{err}");
        } else {
            assert!(err.contains("larger than atlas texture"), "{err}");
        }
    }
    drop(r);
    drop(f);
    crate::quit();
}

/* The GPU text engine, against testdata/gpu_reference.txt (the output of
a C program built from upstream SDL_ttf and SDL3 with the GPU API,
which runs the cases below in the same order). */

/// Initializes the offscreen video driver for a GPU device, and quits it.
struct OffscreenVideo;

impl OffscreenVideo {
    fn init() -> sdl3::Result<OffscreenVideo> {
        sdl3::hints::set(sdl3::hints::VIDEO_DRIVER, "offscreen")?;
        sdl3::init::init(sdl3::init::InitFlags::VIDEO)?;
        Ok(OffscreenVideo)
    }
}

impl Drop for OffscreenVideo {
    fn drop(&mut self) {
        sdl3::init::quit_subsystem(sdl3::init::InitFlags::VIDEO);
        sdl3::hints::reset(sdl3::hints::VIDEO_DRIVER);
    }
}

/// The hash of a GPU atlas texture's pixels.
fn gpu_texture_hash(device: &sdl3::gpu::Device, texture: &sdl3::gpu::Texture, size: i32) -> u64 {
    use sdl3::gpu::{
        TextureRegion, TextureTransferInfo, TransferBufferCreateInfo, TransferBufferUsage,
    };
    let mut buffer = device
        .create_transfer_buffer(&TransferBufferCreateInfo {
            usage: TransferBufferUsage::Download,
            size: (size * size * 4) as u32,
            props: None,
        })
        .unwrap();
    let mut cmd = device.acquire_command_buffer().unwrap();
    let mut pass = cmd.begin_copy_pass().unwrap();
    pass.download_from_texture(
        &TextureRegion {
            texture,
            mip_level: 0,
            layer: 0,
            x: 0,
            y: 0,
            z: 0,
            w: size as u32,
            h: size as u32,
            d: 1,
        },
        &TextureTransferInfo {
            transfer_buffer: &buffer,
            offset: 0,
            pixels_per_row: 0,
            rows_per_layer: 0,
        },
    );
    pass.end();
    cmd.submit().unwrap();
    device.wait_for_idle().unwrap();
    let pixels = buffer.map(false).unwrap();
    fnv(FNV0, &pixels)
}

fn fpoints_bytes(points: &[sdl3::video::FPoint]) -> Vec<u8> {
    points
        .iter()
        .flat_map(|p| p.x.to_le_bytes().into_iter().chain(p.y.to_le_bytes()))
        .collect()
}

/// The harness's `gdump()`: the draw sequences, one line each, sorted,
/// with the hash of their sorted quads (upstream orders the glyphs of
/// different atlases by the atlases' addresses); atlases are named by the
/// hash of their pixels.
fn gdump(out: &mut Vec<String>, label: &str, device: &sdl3::gpu::Device, t: &Text, size: i32) {
    let seqs = match gpu_text_draw_data(t) {
        Ok(Some(seqs)) => seqs,
        Ok(None) => {
            out.push(format!("{label} gpu: none"));
            return;
        }
        Err(e) => {
            out.push(format!("{label} gpu: none err: {e}"));
            return;
        }
    };
    let mut lines = Vec::new();
    for seq in seqs.iter() {
        let tex = seq
            .atlas_texture
            .as_ref()
            .map_or(0, |t| gpu_texture_hash(device, t, size));
        let nq = (seq.num_vertices / 4) as usize;
        let mut quads: Vec<u64> = (0..nq)
            .map(|q| {
                let mut h = fnv(FNV0, &fpoints_bytes(&seq.xy[q * 4..q * 4 + 4]));
                if !seq.uv.is_empty() {
                    h = fnv(h, &fpoints_bytes(&seq.uv[q * 4..q * 4 + 4]));
                }
                h
            })
            .collect();
        quads.sort_unstable();
        let qbytes: Vec<u8> = quads.iter().flat_map(|q| q.to_le_bytes()).collect();
        let ibytes: Vec<u8> = seq.indices[..seq.num_indices as usize]
            .iter()
            .flat_map(|i| i.to_le_bytes())
            .collect();
        lines.push(format!(
            "tex={tex:016x} type={} nv={} ni={} uv={} quads={:016x} idx={:016x}",
            seq.image_type as i32,
            seq.num_vertices,
            seq.num_indices,
            !seq.uv.is_empty() as i32,
            fnv(FNV0, &qbytes),
            fnv(FNV0, &ibytes)
        ));
    }
    lines.sort();
    out.push(format!("{label} gpu: {}", lines.len()));
    for line in lines {
        out.push(format!("{label} seq: {line}"));
    }
}

/// Each engine sees one generation of the font: when glyphs are
/// recreated, upstream reuses free atlas areas a pixel larger than the
/// glyphs, reading past the glyphs' pixels, in the order of its hash
/// tables.
fn gpu_pass(
    out: &mut Vec<String>,
    device: &sdl3::gpu::Device,
    tag: &str,
    size: i32,
    f: &Font,
    fb: &Font,
    text1: &str,
) {
    use crate::{STYLE_NORMAL, STYLE_STRIKETHROUGH, STYLE_UNDERLINE};

    let l = |s: &str| format!("{tag} {s}");
    f.set_size(16.5).unwrap();
    let engine = GpuTextEngine::with_atlas_texture_size(device, size).unwrap();
    out.push(format!(
        "{tag} engine: 1 winding: {}",
        engine.winding() as i32
    ));
    let dyn_engine: Rc<dyn TextEngine> = engine.clone();
    let t = Text::new(Some(dyn_engine), Some(f), text1).unwrap();
    gdump(out, &l("plain"), device, &t, size);
    t.set_wrap_width(150).unwrap();
    gdump(out, &l("wrap150"), device, &t, size);
    t.set_position(5, 7).unwrap();
    gdump(out, &l("pos"), device, &t, size);
    engine
        .set_winding(GpuTextEngineWinding::CounterClockwise)
        .unwrap();
    out.push(format!("{tag} winding: {}", engine.winding() as i32));
    t.set_wrap_width(160).unwrap();
    gdump(out, &l("ccw"), device, &t, size);
    let e = engine
        .set_winding(GpuTextEngineWinding::Invalid)
        .unwrap_err();
    out.push(format!(
        "{tag} winding invalid: {e} {}",
        engine.winding() as i32
    ));
    engine.set_winding(GpuTextEngineWinding::Clockwise).unwrap();
    t.set_string(Some("")).unwrap();
    gdump(out, &l("empty"), device, &t, size);
    t.set_string(Some(text1)).unwrap();
    gdump(out, &l("again"), device, &t, size);
    t.set_font(None).unwrap();
    gdump(out, &l("nofont"), device, &t, size);
    drop(t);
    drop(engine);

    let new_text = || {
        let engine: Rc<dyn TextEngine> =
            GpuTextEngine::with_atlas_texture_size(device, size).unwrap();
        Text::new(Some(engine), Some(f), text1).unwrap()
    };
    f.set_style(STYLE_UNDERLINE | STYLE_STRIKETHROUGH);
    let t = new_text();
    t.set_wrap_width(150).unwrap();
    gdump(out, &l("styled"), device, &t, size);
    drop(t);
    f.set_style(STYLE_NORMAL);

    f.set_outline(2).unwrap();
    let t = new_text();
    gdump(out, &l("outline"), device, &t, size);
    drop(t);
    f.set_outline(0).unwrap();

    f.add_fallback_font(fb).unwrap();
    let t = new_text();
    t.append_string(" t a i l € ← ☺").unwrap();
    gdump(out, &l("fallback"), device, &t, size);
    t.set_font(Some(fb)).unwrap();
    t.set_string(Some("abc def")).unwrap();
    gdump(out, &l("fbfont"), device, &t, size);
    drop(t);
    f.remove_fallback_font(fb);
}

#[test]
fn gpu_text_engine_matches_upstream_reference() {
    use sdl3::gpu::{Device, ShaderFormat};

    // Declared before the device, so dropped after it.
    let video = OffscreenVideo::init();
    let device = match video
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|_| Device::new(ShaderFormat::SPIRV, false, None))
    {
        Ok(device) => device,
        Err(e) => {
            // SDL3_TEST_REQUIRE (docs/HARDWARE_TESTING.md) makes a missing
            // GPU a failure.
            let list = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
            let required = list
                .split(',')
                .any(|c| c == "all" || (cfg!(target_os = "linux") && c == "vulkan"));
            assert!(!required, "no GPU device: {e}");
            eprintln!("skipped: no GPU device ({e})");
            return;
        }
    };

    crate::init().unwrap();
    let mut out = Vec::new();
    for (name, data) in [
        ("DejaVuSans.ttf", SANS),
        ("DejaVuSerif-Bold.ttf", SERIF_BOLD),
    ] {
        out.push(format!("== gpu {name}"));
        let f = open_mem(data, 16.5).unwrap();
        let fb = open_mem(MONO, 14.0).unwrap();
        gpu_pass(&mut out, &device, "big", 1024, &f, &fb, WRAP);
        gpu_pass(&mut out, &device, "atlas32", 32, &f, &fb, SPACED1);
        gpu_pass(&mut out, &device, "atlas64", 64, &f, &fb, SPACED1);
        {
            // a text of another engine, and a glyph as large as the atlas
            // (which upstream pads past it and never packs: not run)
            let se: Rc<dyn TextEngine> = SurfaceTextEngine::new().unwrap();
            let st = Text::new(Some(se), Some(&f), "x").unwrap();
            gdump(&mut out, "surfaceengine", &device, &st, 0);
            drop(st);
            match GpuTextEngine::with_atlas_texture_size(&device, 0) {
                Ok(_) => out.push("atlas0 engine: 1 err: ".into()),
                Err(e) => out.push(format!("atlas0 engine: 0 err: {e}")),
            }
            let e: Rc<dyn TextEngine> = GpuTextEngine::with_atlas_texture_size(&device, 8).unwrap();
            let t = Text::new(Some(e), Some(&f), "Wide").unwrap();
            gdump(&mut out, "atlas8", &device, &t, 8);
        }
    }
    crate::quit();

    let reference: Vec<&str> = include_str!("testdata/gpu_reference.txt").lines().collect();
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

/// A glyph that a renderer atlas can't hold once stb_rect_pack aligns its
/// width fails (upstream creates atlases without end).
#[test]
fn renderer_engine_rejects_glyphs_that_never_pack() {
    use crate::stb_rect_pack::stbrp_fits_empty_target;

    // 9 pixel atlases have 2 nodes and align widths to 5
    assert!(stbrp_fits_empty_target(5, 9, 9));
    assert!(!stbrp_fits_empty_target(6, 9, 9));
    assert!(!stbrp_fits_empty_target(5, 10, 9));

    crate::init().unwrap();
    let f = open_mem(SANS, 16.5).unwrap();
    let (w, h) = f
        .glyph_image(u32::from('W'))
        .map(|(s, _)| (s.width(), s.height()))
        .unwrap();
    let size = (w.max(h)..w.max(h) + 64)
        .find(|&size| !stbrp_fits_empty_target(w, h, size))
        .unwrap();
    let surface = Surface::new(16, 16, sdl3::video::PixelFormat::ARGB8888).unwrap();
    let r = Rc::new(RefCell::new(Renderer::software(surface).unwrap()));
    let engine: Rc<dyn TextEngine> =
        RendererTextEngine::with_atlas_texture_size(r.clone(), size).unwrap();
    let t = Text::new(Some(engine), Some(&f), "W").unwrap();
    let err = draw_renderer_text(&t, 0.0, 0.0).unwrap_err().to_string();
    assert!(
        err.contains("larger than atlas texture"),
        "{w}x{h} in {size}: {err}"
    );
    drop(t);
    drop(r);
    drop(f);
    crate::quit();
}

mod shaping;
