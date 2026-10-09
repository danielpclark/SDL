// Tests of sdl3-rtf against upstream SDL_rtf's C.
// Copyright (C) 2003-2024 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The documents in `testdata/docs/` were written for these tests (SDL_rtf
//! ships none): fonts, colors, bold, italic and underline, sizes,
//! paragraphs and line breaks, alignment, indents, tabs, escapes and hex
//! characters, `\bin` data, skipped and unknown destinations, the
//! document information, a table (whose controls the reader skips),
//! property values out of range, raw 8-bit characters, and malformed
//! documents. The fonts are sdl3-ttf's DejaVu subsets.
//!
//! `testdata/reference.txt` is the output of a C program built from
//! upstream SDL_rtf (the revision of [`crate::REVISION`]) over SDL_ttf
//! (with its bundled FreeType and HarfBuzz) and SDL3, with the font engine
//! of upstream's showrtf.c, which runs the cases below in the same order:
//! each document loaded into a context drawing with a software renderer,
//! its title, subject and author, its height at several widths, and the
//! pixels rendered at those widths, in the whole viewport, in a rectangle
//! scrolled by several offsets, with the clip rectangle left behind; then
//! the documents truncated, and with flipped bytes. The font engine's
//! calls are logged: the fonts created and freed, the texts measured and
//! rendered. Pixels are FNV-1a hashes of the rows read back.

use std::cell::RefCell;
use std::rc::Rc;

use sdl3::io::IoStream;
use sdl3::render::{Renderer, Texture};
use sdl3::video::{Color, PixelFormat, Rect, Surface};

use crate::{
    font_family_to_index, version, Context, FontEngine, FontFamily, FontSource, FontStyle,
    TtfFontEngine,
};

static SANS: &[u8] = include_bytes!("../../sdl3-ttf/src/testdata/fonts/DejaVuSans.ttf");
static SERIF_BOLD: &[u8] = include_bytes!("../../sdl3-ttf/src/testdata/fonts/DejaVuSerif-Bold.ttf");
static MONO: &[u8] = include_bytes!("../../sdl3-ttf/src/testdata/fonts/DejaVuSansMono.ttf");

static REFERENCE: &str = include_str!("testdata/reference.txt");

const FULL_DOCS: &[(&str, &[u8])] = &[
    ("sample.rtf", include_bytes!("testdata/docs/sample.rtf")),
    ("table.rtf", include_bytes!("testdata/docs/table.rtf")),
    ("edge.rtf", include_bytes!("testdata/docs/edge.rtf")),
    ("nocolor.rtf", include_bytes!("testdata/docs/nocolor.rtf")),
    (
        "unmatched.rtf",
        include_bytes!("testdata/docs/unmatched.rtf"),
    ),
    ("unclosed.rtf", include_bytes!("testdata/docs/unclosed.rtf")),
    ("badhex.rtf", include_bytes!("testdata/docs/badhex.rtf")),
    ("eofkw.rtf", include_bytes!("testdata/docs/eofkw.rtf")),
    ("nofont.rtf", include_bytes!("testdata/docs/nofont.rtf")),
    ("plain.txt", include_bytes!("testdata/docs/plain.txt")),
    ("empty.rtf", include_bytes!("testdata/docs/empty.rtf")),
];
const DAMAGED_DOCS: &[usize] = &[0, 1, 2];

type Log = Rc<RefCell<Vec<String>>>;

/// The harness's logging engine around showrtf's
struct LogEngine {
    inner: TtfFontEngine,
    log: Log,
    verbose: bool,
    version: i32,
}

impl FontEngine for LogEngine {
    type Font = (sdl3_ttf::Font, String);

    fn version(&self) -> i32 {
        self.version
    }

    fn create_font(
        &mut self,
        name: &str,
        family: FontFamily,
        charset: i32,
        size: i32,
        style: FontStyle,
    ) -> Option<Self::Font> {
        let font = self.inner.create_font(name, family, charset, size, style);
        if self.verbose {
            self.log.borrow_mut().push(format!(
                "create name=\"{name}\" family={} charset={charset} size={size} style={style}: {}",
                family.0,
                if font.is_some() { "ok" } else { "fail" }
            ));
        }
        Some((font?, format!("name=\"{name}\" size={size} style={style}")))
    }

    fn line_spacing(&mut self, font: &Self::Font) -> i32 {
        self.inner.line_spacing(&font.0)
    }

    fn character_offsets(
        &mut self,
        font: &Self::Font,
        text: &str,
        byte_offsets: &mut [i32],
        pixel_offsets: &mut [i32],
    ) -> i32 {
        let n = self
            .inner
            .character_offsets(&font.0, text, byte_offsets, pixel_offsets);
        if self.verbose {
            let mut line = format!("offsets \"{text}\": {n}");
            if (n as usize) < byte_offsets.len() {
                line += &format!(" width={}", pixel_offsets[n as usize]);
            }
            self.log.borrow_mut().push(line);
        }
        n
    }

    fn render_text(
        &mut self,
        font: &Self::Font,
        renderer: &mut Renderer,
        text: &str,
        fg: Color,
    ) -> Option<Texture> {
        let t = self.inner.render_text(&font.0, renderer, text, fg);
        if self.verbose {
            let (w, h) = t
                .and_then(|t| renderer.texture_size(t).ok())
                .unwrap_or((0.0, 0.0));
            self.log.borrow_mut().push(format!(
                "render \"{text}\" {:02x}{:02x}{:02x}{:02x}: {} {}x{}",
                fg.r,
                fg.g,
                fg.b,
                fg.a,
                if t.is_some() { "ok" } else { "fail" },
                w as i32,
                h as i32
            ));
        }
        t
    }

    fn free_font(&mut self, font: Self::Font) {
        if self.verbose {
            self.log.borrow_mut().push(format!("free {}", font.1));
        }
        self.inner.free_font(font.0);
    }
}

fn engine(log: &Log, verbose: bool) -> LogEngine {
    let mut inner = TtfFontEngine::new(FontSource::Memory(SANS));
    inner.set_font(FontFamily::ROMAN, FontSource::Memory(SERIF_BOLD));
    inner.set_font(FontFamily::MODERN, FontSource::Memory(MONO));
    LogEngine {
        inner,
        log: log.clone(),
        verbose,
        version: crate::FONT_ENGINE_VERSION,
    }
}

const FNV0: u64 = 14695981039346656037;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn print_pixels(log: &Log, label: &str, r: &Rc<RefCell<Renderer>>) {
    let line = match r.borrow_mut().read_pixels(None) {
        Ok(s) => {
            let mut h = FNV0;
            let row = s.width() as usize * s.format().bytes_per_pixel() as usize;
            let pitch = s.pitch() as usize;
            if let Some(pixels) = s.pixels() {
                for y in 0..s.height() as usize {
                    h = fnv(h, &pixels[y * pitch..y * pitch + row]);
                }
            }
            format!(
                "{label}: {}x{} {} hash={h:016x}",
                s.width(),
                s.height(),
                s.format().name()
            )
        }
        Err(e) => format!("{label}: err: {e}"),
    };
    log.borrow_mut().push(line);
}

fn print_clip(log: &Log, r: &Rc<RefCell<Renderer>>) {
    let r = r.borrow();
    let c = r.clip_rect();
    log.borrow_mut().push(format!(
        "clip: {} {},{} {}x{}",
        r.clip_enabled() as i32,
        c.x,
        c.y,
        c.w,
        c.h
    ));
}

fn clear(r: &Rc<RefCell<Renderer>>) {
    let mut r = r.borrow_mut();
    r.set_draw_color(0xFF, 0xFF, 0xF0, 0xFF);
    let _ = r.clear();
}

fn load(log: &Log, r: &Rc<RefCell<Renderer>>, data: &[u8], verbose: bool) -> Context<LogEngine> {
    let mut ctx = Context::new(r.clone(), engine(log, verbose)).unwrap();
    let mut src = IoStream::from_const_mem(data);
    let res = ctx.load_io(&mut src);
    drop(src);
    let line = match res {
        Ok(()) => "load: ok".to_owned(),
        Err(e) => format!("load: err: {e}"),
    };
    log.borrow_mut().push(line);
    log.borrow_mut().push(format!(
        "title=\"{}\" subject=\"{}\" author=\"{}\"",
        ctx.title(),
        ctx.subject(),
        ctx.author()
    ));
    ctx
}

fn renderer(w: i32, h: i32) -> Rc<RefCell<Renderer>> {
    let surface = Surface::new(w, h, PixelFormat::ARGB8888).unwrap();
    Rc::new(RefCell::new(Renderer::software(surface).unwrap()))
}

const WIDTHS: &[i32] = &[320, 150, 60];

fn full_case(log: &Log, name: &str, data: &[u8]) {
    log.borrow_mut().push(format!("== {name}"));
    let r = renderer(360, 640);
    let mut ctx = load(log, &r, data, true);
    let height = ctx.height(0);
    log.borrow_mut().push(format!("height 0: {height}"));
    for &w in WIDTHS {
        let height = ctx.height(w);
        log.borrow_mut().push(format!("height {w}: {height}"));
        let height = ctx.height(w);
        log.borrow_mut().push(format!("height {w} again: {height}"));
        clear(&r);
        ctx.render(Some(&Rect::new(0, 0, w, 640)), 0);
        print_pixels(log, &format!("pixels {w}"), &r);
        print_clip(log, &r);
    }
    /* the whole viewport, scrolled */
    let _ = r.borrow_mut().set_clip_rect(None);
    clear(&r);
    ctx.render(None, 10);
    print_pixels(log, "pixels viewport 10", &r);
    print_clip(log, &r);
    /* a rectangle, scrolled, with a clip rectangle set before */
    let _ = r
        .borrow_mut()
        .set_clip_rect(Some(&Rect::new(5, 5, 300, 400)));
    clear(&r);
    let rect = Rect::new(13, 9, 200, 120);
    ctx.render(Some(&rect), 25);
    print_pixels(log, "pixels rect 25", &r);
    print_clip(log, &r);
    /* scrolled up past the top, and below the end */
    clear(&r);
    ctx.render(Some(&rect), -30);
    print_pixels(log, "pixels rect -30", &r);
    clear(&r);
    ctx.render(Some(&rect), 100000);
    print_pixels(log, "pixels rect 100000", &r);
    drop(ctx);
}

/* Truncated and corrupted documents: quieter */
fn short_case(log: &Log, label: &str, data: &[u8]) {
    log.borrow_mut().push(format!("-- {label}"));
    let r = renderer(200, 300);
    let mut ctx = load(log, &r, data, false);
    let height = ctx.height(200);
    log.borrow_mut().push(format!("height 200: {height}"));
    clear(&r);
    ctx.render(None, 0);
    print_pixels(log, "pixels", &r);
}

/// The harness's cases, `damaged` selecting the full cases or the
/// truncated and flipped documents
fn run_cases(damaged: bool) -> Vec<String> {
    sdl3_ttf::init().unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    if !damaged {
        log.borrow_mut()
            .push(format!("version: {}", version().to_number()));
        let mut bad = engine(&log, true);
        bad.version = 2;
        let line = match Context::new(renderer(1, 1), bad) {
            Ok(_) => "bad engine: ok".to_owned(),
            Err(e) => format!("bad engine: {e}"),
        };
        log.borrow_mut().push(line);
        for (name, data) in FULL_DOCS {
            full_case(&log, name, data);
        }
    } else {
        for &i in DAMAGED_DOCS {
            let (name, data) = FULL_DOCS[i];
            let len = data.len();
            for n in (0..len).step_by(11) {
                short_case(&log, &format!("{name} truncated {n}"), &data[..n]);
            }
            for p in (3..len).step_by(23) {
                let x = ((p * 37) % 255 + 1) as u8;
                let mut copy = data.to_vec();
                copy[p] ^= x;
                short_case(&log, &format!("{name} flipped {p} ^ {x:02x}"), &copy);
            }
        }
    }
    sdl3_ttf::quit();
    let out = log.borrow().clone();
    out
}

fn compare(out: &[String], expected: &[&str]) {
    for (i, (a, b)) in out.iter().zip(expected).enumerate() {
        assert_eq!(a, b, "line {} of the reference differs", i + 1);
    }
    assert_eq!(
        out.len(),
        expected.len(),
        "line count differs from the reference"
    );
}

/// The reference's lines: the full cases, and the damaged documents'
fn reference() -> (Vec<&'static str>, Vec<&'static str>) {
    let lines: Vec<&str> = REFERENCE.lines().collect();
    let split = lines
        .iter()
        .position(|l| l.starts_with("-- "))
        .unwrap_or(lines.len());
    (lines[..split].to_vec(), lines[split..].to_vec())
}

#[test]
fn documents_match_upstream() {
    let out = run_cases(false);
    compare(&out, &reference().0);
}

#[test]
fn damaged_documents_match_upstream() {
    let out = run_cases(true);
    compare(&out, &reference().1);
}

#[test]
fn family_indices() {
    assert_eq!(font_family_to_index(FontFamily::DEFAULT), 0);
    assert_eq!(font_family_to_index(FontFamily::BIDI), 7);
    assert_eq!(font_family_to_index(FontFamily(300)), 0);
    assert_eq!(font_family_to_index(FontFamily(-1)), 0);
}

/// Loading again into a context frees the previous document (upstream
/// frees the color table a second time there, FIXME (upstream))
#[test]
fn reload_context() {
    sdl3_ttf::init().unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let r = renderer(320, 200);
    let mut ctx = Context::new(r.clone(), engine(&log, false)).unwrap();
    let (_, sample) = FULL_DOCS[0];
    let (_, table) = FULL_DOCS[1];
    ctx.load_io(&mut IoStream::from_const_mem(sample)).unwrap();
    let h1 = ctx.height(320);
    ctx.render(None, 0);
    ctx.load_io(&mut IoStream::from_const_mem(table)).unwrap();
    assert_eq!(ctx.title(), "Table");
    let h2 = ctx.height(320);
    ctx.load_io(&mut IoStream::from_const_mem(sample)).unwrap();
    assert_eq!(ctx.title(), "Sample Document");
    assert_eq!(ctx.author(), "A. Writer");
    assert_eq!(ctx.height(320), h1);
    assert_ne!(h1, h2);
    // A failed load leaves what was read, and the context usable
    assert!(ctx
        .load_io(&mut IoStream::from_const_mem(b"{\\rtf1}}"))
        .is_err());
    ctx.load_io(&mut IoStream::from_const_mem(table)).unwrap();
    assert_eq!(ctx.height(320), h2);
    drop(ctx);
    sdl3_ttf::quit();
}

/// Upstream dereferences a NULL buffer on these (a first font without a
/// name, an empty title before any text): empty strings here
#[test]
fn empty_names() {
    sdl3_ttf::init().unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let r = renderer(320, 200);
    let mut ctx = Context::new(r.clone(), engine(&log, true)).unwrap();
    ctx.load_io(&mut IoStream::from_const_mem(
        b"{\\rtf1{\\info{\\title}}{\\fonttbl{\\f0\\fswiss;}}Text\\par}",
    ))
    .unwrap();
    assert_eq!(ctx.title(), "");
    assert!(ctx.height(100) > 0);
    assert!(log
        .borrow()
        .iter()
        .any(|l| l.starts_with("create name=\"\" family=2")));
    drop(ctx);
    sdl3_ttf::quit();
}

/// Keywords and parameters longer than upstream's buffers (which overflow
/// there) are read whole; parameters saturate as `strtol()` does
#[test]
fn long_keywords() {
    sdl3_ttf::init().unwrap();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let r = renderer(320, 200);
    let mut ctx = Context::new(r.clone(), engine(&log, true)).unwrap();
    let mut doc =
        b"{\\rtf1{\\fonttbl{\\f0 Sans;}}{\\colortbl;\\red0\\green0\\blue0;}\\cf1 \\".to_vec();
    doc.extend(std::iter::repeat_n(b'k', 5000));
    doc.extend(b" a\\fs");
    doc.extend(std::iter::repeat_n(b'9', 100));
    doc.extend(b" b\\fs-");
    doc.extend(std::iter::repeat_n(b'9', 100));
    doc.extend(b" c\\fs24 d\\par}");
    ctx.load_io(&mut IoStream::from_const_mem(&doc)).unwrap();
    assert!(ctx.height(320) > 0);
    drop(ctx);
    let log = log.borrow();
    assert!(log.iter().all(|l| !l.contains("kkk")));
    assert!(log.iter().any(|l| l.starts_with("offsets \"a\": 1 ")));
    // \fs99...9 is (int)LONG_MAX = -1 half-points, \fs-99...9 is
    // -(int)LONG_MAX = 1: no font at 0 points, b and c are dropped
    assert_eq!(
        log.iter()
            .filter(|l| l.contains("size=0 style=0: fail"))
            .count(),
        2
    );
    assert!(log
        .iter()
        .all(|l| !l.contains("\"b\"") && !l.contains("\"c\"")));
    assert!(log.iter().any(|l| l.starts_with("offsets \"d\": 1 ")));
    sdl3_ttf::quit();
}
