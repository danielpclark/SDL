// Rust translation of examples/showrtf.c from SDL_rtf.
// Copyright (C) 2003-2024 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

/* A simple program to test the RTF rendering of the SDL_rtf library */

// The font engine (CreateFont() to FreeFont()) is sdl3_rtf::TtfFontEngine.

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use sdl3::events::keyboard::{Keycode, Scancode};
use sdl3::events::window::WindowFlags;
use sdl3::events::{Event, EventType};
use sdl3::log::Category;
use sdl3_rtf::{font_family_to_index, Context, FontFamily, FontSource, TtfFontEngine};

const SCREEN_WIDTH: i32 = 640;
const SCREEN_HEIGHT: i32 = 480;

fn load_rtf(ctx: &mut Context<TtfFontEngine>, file: &str) {
    if let Err(e) = ctx.load(file) {
        sdl3::error!(Category::Application, "Couldn't load {}: {}\n", file, e);
    }
}

fn print_usage(argv0: &str) {
    sdl3::log!("Usage: {} -fdefault font.ttf [-froman font.ttf] [-fswiss font.ttf] [-fmodern font.ttf] [-fscript font.ttf] [-fdecor font.ttf] [-ftech font.ttf] file.rtf\n", argv0);
}

fn cleanup() {
    sdl3_ttf::quit();
    sdl3::init::quit();
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let argc = argv.len();
    let mut font_engine = TtfFontEngine::default();

    /* Parse command line arguments */
    let mut i = 1;
    while i < argc {
        let family = match argv[i].as_str() {
            "-fdefault" => FontFamily::DEFAULT,
            "-froman" => FontFamily::ROMAN,
            "-fswiss" => FontFamily::SWISS,
            "-fmodern" => FontFamily::MODERN,
            "-fscript" => FontFamily::SCRIPT,
            "-fdecor" => FontFamily::DECOR,
            "-ftech" => FontFamily::TECH,
            _ => break,
        };
        i += 1;
        font_engine.font_list[font_family_to_index(family)] =
            argv.get(i).cloned().map(FontSource::File);
        i += 1;
    }
    let start = i;
    let stop = argc as isize - 1;
    if font_engine.font_list[0].is_none() || start as isize > stop {
        print_usage(&argv[0]);
        return ExitCode::from(1);
    }
    let stop = stop as usize;

    /* Initialize the TTF library */
    if let Err(e) = sdl3_ttf::init() {
        sdl3::error!(Category::Application, "Couldn't initialize TTF: {}\n", e);
        sdl3::init::quit();
        return ExitCode::from(3);
    }

    let (window, renderer) = match sdl3::render::create_window_and_renderer(
        "showrtf demo",
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
        WindowFlags::RESIZABLE,
    ) {
        Ok(created) => created,
        Err(e) => {
            sdl3::error!(
                Category::Application,
                "SDL_CreateWindowAndRenderer() failed: {}\n",
                e
            );
            cleanup();
            return ExitCode::from(4);
        }
    };
    let renderer = Rc::new(RefCell::new(renderer));

    /* Create and load the RTF document */
    let mut ctx = match Context::new(renderer.clone(), font_engine) {
        Ok(ctx) => ctx,
        Err(e) => {
            sdl3::error!(
                Category::Application,
                "Couldn't create RTF context: {}\n",
                e
            );
            cleanup();
            return ExitCode::from(5);
        }
    };
    load_rtf(&mut ctx, &argv[i]);
    let _ = window.set_title(ctx.title());

    /* Render the document to the screen */
    let mut done = false;
    let mut offset = 0;
    let (mut w, mut h) = window.size().unwrap_or((0, 0));
    let mut height = ctx.height(w);
    while !done {
        while let Some(event) = sdl3::events::poll() {
            if event.event_type() == EventType::WINDOW_RESIZED {
                let ratio = offset as f32 / height as f32;
                sdl3::info!(Category::Application, "Resetting window\n");
                (w, h) = window.size().unwrap_or((w, h));
                let _ = renderer.borrow_mut().set_viewport(None);
                height = ctx.height(w);
                offset = (ratio * height as f32) as i32;
            }
            if let Event::Key(key) = &event {
                if key.down {
                    match key.key {
                        Keycode::ESCAPE => {
                            done = true;
                        }
                        Keycode::LEFT => {
                            if i > start {
                                i -= 1;
                                load_rtf(&mut ctx, &argv[i]);
                                offset = 0;
                                height = ctx.height(w);
                            }
                        }
                        Keycode::RIGHT => {
                            if i < stop {
                                i += 1;
                                load_rtf(&mut ctx, &argv[i]);
                                offset = 0;
                                height = ctx.height(w);
                            }
                        }
                        Keycode::HOME => {
                            offset = 0;
                        }
                        Keycode::END => {
                            offset = height - h;
                        }
                        Keycode::PAGEUP => {
                            offset -= h;
                            if offset < 0 {
                                offset = 0;
                            }
                        }
                        Keycode::PAGEDOWN | Keycode::SPACE => {
                            offset += h;
                            if offset > (height - h) {
                                offset = height - h;
                            }
                        }
                        _ => {}
                    }
                }
            }
            if event.event_type() == EventType::QUIT {
                done = true;
            }
        }
        let keystate = sdl3::events::keyboard::keyboard_state();
        if keystate[Scancode::UP.0 as usize] {
            offset -= 1;
            if offset < 0 {
                offset = 0;
            }
        }
        if keystate[Scancode::DOWN.0 as usize] {
            offset += 1;
            if offset > (height - h) {
                offset = height - h;
            }
        }

        {
            let mut r = renderer.borrow_mut();
            r.set_draw_color(0xFF, 0xFF, 0xFF, 0xFF);
            let _ = r.clear();
        }
        ctx.render(None, offset);
        let _ = renderer.borrow_mut().present();
        sdl3::timer::delay(Duration::from_millis(10));
    }

    /* Clean up and exit */
    drop(ctx);
    drop(renderer);
    window.destroy();
    cleanup();

    ExitCode::SUCCESS
}
