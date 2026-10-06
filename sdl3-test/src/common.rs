// Rust translation of src/test/SDL_test_common.c and include/SDL3/SDL_test_common.h
// from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Common functions of SDL test framework.
//!
//! What SDL's test programs share: the command line options for the
//! subsystems they use (video, renderer, audio, logging, memory tracking),
//! the windows, renderers and audio device made from them, logging of
//! events, the hotkeys every test window has, and a summary of a window's
//! state drawn with the debug font.
//!
//! Ported from original test/common.c file.
//!
//! ```no_run
//! use sdl3::init::InitFlags;
//! use sdl3_test::common::CommonState;
//!
//! let mut state = CommonState::new(std::env::args().collect(), InitFlags::VIDEO);
//! if !state.default_args() {
//!     std::process::exit(1);
//! }
//! state.init().unwrap();
//! let mut done = false;
//! while !done {
//!     while let Some(event) = sdl3::events::poll() {
//!         state.event(&event, &mut done);
//!     }
//! }
//! state.quit();
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sdl3::app::AppResult;
use sdl3::audio::{AudioDeviceID, AudioFormat, AudioSpec};
use sdl3::events::keyboard::{Keycode, Keymod};
use sdl3::events::mouse::MouseButtonFlags;
use sdl3::events::window::WindowFlags;
use sdl3::events::{CommonEvent, DisplayID, Event, EventType, WindowID};
use sdl3::init::InitFlags;
use sdl3::io::IoStream;
use sdl3::log::{Category, Priority};
use sdl3::properties::Properties;
use sdl3::render::{LogicalPresentation, Renderer, Texture};
use sdl3::stdlib::string::{strcasecmp, strtod, strtol};
use sdl3::timer::Timer;
use sdl3::video::gl::{gl_set_attribute, GlAttr, GL_CONTEXT_DEBUG_FLAG};
use sdl3::video::messagebox::{show_simple_message_box, MessageBoxFlags};
use sdl3::video::pixels::PixelFormat;
use sdl3::video::rect::{Point, Rect};
use sdl3::video::surface::Surface;
use sdl3::video::window as video_window;
use sdl3::video::{
    self, DisplayMode, DisplayOrientation, FlashOperation, HitTestResult, ProgressState,
    SystemTheme, Window,
};
use sdl3::{hints, log as sdl_log, Error, Result};

use crate::font::draw_string;
use crate::internal::{fmt_g, set_color};

/// The size test windows are made by default. Translation of
/// `DEFAULT_WINDOW_WIDTH` (480 on the PSP and 960 on the Vita upstream,
/// which aren't targets here).
pub const DEFAULT_WINDOW_WIDTH: i32 = 640;
/// Translation of `DEFAULT_WINDOW_HEIGHT` (272 on the PSP and 544 on the
/// Vita upstream).
pub const DEFAULT_WINDOW_HEIGHT: i32 = 480;

/// What a test program logs about the subsystems (`--info`). Translation
/// of `SDLTest_VerboseFlags`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct VerboseFlags(pub u32);

impl VerboseFlags {
    /// Translation of `VERBOSE_VIDEO`.
    pub const VIDEO: VerboseFlags = VerboseFlags(0x00000001);
    /// Translation of `VERBOSE_MODES`.
    pub const MODES: VerboseFlags = VerboseFlags(0x00000002);
    /// Translation of `VERBOSE_RENDER`.
    pub const RENDER: VerboseFlags = VerboseFlags(0x00000004);
    /// Translation of `VERBOSE_EVENT`.
    pub const EVENT: VerboseFlags = VerboseFlags(0x00000008);
    /// Translation of `VERBOSE_AUDIO`.
    pub const AUDIO: VerboseFlags = VerboseFlags(0x00000010);
    /// Translation of `VERBOSE_MOTION`.
    pub const MOTION: VerboseFlags = VerboseFlags(0x00000020);

    /// Whether any of `other`'s flags are set (`verbose & FLAG`).
    pub const fn intersects(self, other: VerboseFlags) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for VerboseFlags {
    type Output = VerboseFlags;
    fn bitor(self, rhs: VerboseFlags) -> VerboseFlags {
        VerboseFlags(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for VerboseFlags {
    fn bitor_assign(&mut self, rhs: VerboseFlags) {
        self.0 |= rhs.0;
    }
}

/// A parser of command line options, chained after the common state's own
/// (see [`CommonState::add_argument_parser`]). Translation of
/// `SDLTest_ArgumentParser` (the closure-like object replaces its `data`).
pub trait ArgumentParser: fmt::Debug {
    /// Parse one argument at `argv[index]`, returning the number of parsed
    /// arguments, 0 when it isn't this parser's, or a negative value when
    /// the argument is invalid. Translation of `SDLTest_ParseArgumentsFp`.
    fn parse_arguments(&mut self, argv: &[String], index: usize) -> i32;

    /// Finalize this argument parser: called before its usage is printed.
    /// Translation of `SDLTest_FinalizeArgumentParserFp`.
    fn finalize(&mut self) {}

    /// The options it takes, printed with `--help` (`None` for none).
    /// Translation of `SDLTest_ArgumentParser.usage`.
    fn usage(&self) -> Option<&[&'static str]>;
}

/// The state of a test program: its options, and the windows, renderers
/// and audio device made from them. Translation of `SDLTest_CommonState`.
pub struct CommonState {
    /* SDL init flags */
    pub argv: Vec<String>,
    pub flags: InitFlags,
    pub verbose: VerboseFlags,

    /* Video info */
    pub videodriver: Option<String>,
    pub display_index: i32,
    pub display_id: DisplayID,
    pub window_title: String,
    pub window_icon: Option<String>,
    pub window_flags: WindowFlags,
    pub flash_on_focus_loss: bool,
    pub window_x: i32,
    pub window_y: i32,
    pub window_w: i32,
    pub window_h: i32,
    pub window_min_w: i32,
    pub window_min_h: i32,
    pub window_max_w: i32,
    pub window_max_h: i32,
    pub window_min_aspect: f32,
    pub window_max_aspect: f32,
    pub logical_w: i32,
    pub logical_h: i32,
    pub auto_scale_content: bool,
    pub logical_presentation: LogicalPresentation,
    pub scale: f32,
    pub depth: i32,
    pub refresh_rate: f32,
    pub fill_usable_bounds: bool,
    pub fullscreen_exclusive: bool,
    pub fullscreen_mode: DisplayMode,
    pub num_windows: i32,
    pub windows: Vec<Window>,
    pub gpudriver: Option<String>,

    /* Renderer info */
    pub renderdriver: Option<String>,
    pub render_vsync: i32,
    pub skip_renderer: bool,
    /// The renderer of each window, if it has one.
    pub renderers: Vec<Option<Renderer>>,
    /// A render target for each window, if the program makes one (destroyed
    /// with the renderers).
    pub targets: Vec<Option<Texture>>,

    /* Audio info */
    pub audiodriver: Option<String>,
    pub audio_format: AudioFormat,
    pub audio_channels: i32,
    pub audio_freq: i32,
    pub audio_id: AudioDeviceID,

    /* GL settings */
    pub gl_red_size: i32,
    pub gl_green_size: i32,
    pub gl_blue_size: i32,
    pub gl_alpha_size: i32,
    pub gl_buffer_size: i32,
    pub gl_depth_size: i32,
    pub gl_stencil_size: i32,
    pub gl_double_buffer: i32,
    pub gl_accum_red_size: i32,
    pub gl_accum_green_size: i32,
    pub gl_accum_blue_size: i32,
    pub gl_accum_alpha_size: i32,
    pub gl_stereo: i32,
    pub gl_release_behavior: i32,
    pub gl_multisamplebuffers: i32,
    pub gl_multisamplesamples: i32,
    pub gl_retained_backing: i32,
    pub gl_accelerated: i32,
    pub gl_major_version: i32,
    pub gl_minor_version: i32,
    pub gl_debug: i32,
    pub gl_profile_mask: i32,

    /* Mouse info */
    pub confine: Rect,
    pub hide_cursor: bool,

    /* Misc. */
    pub quit_after_ms_interval: i32,
    pub quit_after_ms_timer: Option<Timer>,

    /* Options info */
    video_usage: Option<&'static [&'static str]>,
    audio_usage: Option<&'static [&'static str]>,
    /// The parsers after the common, video and audio ones.
    argparsers: Vec<Box<dyn ArgumentParser>>,
}

impl fmt::Debug for CommonState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommonState")
            .field("argv", &self.argv)
            .field("flags", &self.flags)
            .field("verbose", &self.verbose)
            .field("windows", &self.windows)
            .field("argparsers", &self.argparsers)
            .finish_non_exhaustive()
    }
}

/// Translation of `get_environment_bool_variable()`.
fn get_environment_bool_variable(name: &str) -> bool {
    match sdl3::stdlib::getenv(name) {
        Some(var_string) => !var_string.is_empty(),
        None => false,
    }
}

static COMMON_USAGE: [&str; 7] = [
    "[-h | --help]",
    "[--trackmem]",
    "[--randmem]",
    "[--no-color]",
    "[--no-time]",
    "[--info all|video|modes|render|event|event_motion]",
    "[--log all|error|system|audio|video|render|input]",
];

static VIDEO_USAGE: [&str; 40] = [
    "[--always-on-top]",
    "[--aspect min-max]",
    "[--auto-scale-content]",
    "[--center | --position X,Y]",
    "[--confine-cursor X,Y,W,H]",
    "[--depth N]",
    "[--display N]",
    "[--flash-on-focus-loss]",
    "[--fullscreen | --fullscreen-desktop | --windows N]",
    "[--fill-document]",
    "[--geometry WxH]",
    "[--gldebug]",
    "[--grab]",
    "[--hidden]",
    "[--hide-cursor]",
    "[--high-pixel-density]",
    "[--icon icon.bmp]",
    "[--input-focus]",
    "[--keyboard-grab]",
    "[--logical-presentation disabled|match|stretch|letterbox|overscan|integer_scale]",
    "[--logical WxH]",
    "[--max-geometry WxH]",
    "[--maximize]",
    "[--metal-window | --opengl-window | --vulkan-window]",
    "[--min-geometry WxH]",
    "[--minimize]",
    "[--mouse-focus]",
    "[--noframe]",
    "[--quit-after-ms N]",
    "[--refresh R]",
    "[--renderer driver]",
    "[--resizable]",
    "[--scale N]",
    "[--title title]",
    "[--transparent]",
    "[--usable-bounds]",
    "[--utility]",
    "[--video driver]",
    "[--gpu driver]",
    "[--vsync]",
];

/* !!! FIXME: Float32? Sint32? */
static AUDIO_USAGE: [&str; 4] = [
    "[--audio driver]",
    "[--rate N]",
    "[--format U8|S8|S16|S16LE|S16BE|S32|S32LE|S32BE|F32|F32LE|F32BE]",
    "[--channels N]",
];

/// tests can quit after N ms. Translation of `quit_after_ms_cb()`.
fn quit_after_ms_cb() -> Option<Duration> {
    let event = Event::Quit(CommonEvent {
        event_type: EventType::QUIT,
        timestamp: Duration::ZERO,
    });
    let _ = sdl3::events::push(event);
    None
}

/// `SDL_strcasecmp(a, b) == 0`.
fn eq(a: &str, b: &str) -> bool {
    strcasecmp(a, b) == Ordering::Equal
}

/// `SDL_atoi()`.
fn atoi(text: &str) -> i32 {
    strtol(text, 10).0 as i32
}

/// `SDL_atof()`.
fn atof(text: &str) -> f64 {
    strtod(text).0
}

/// The two parts of `text` around the first `separator` (upstream cuts the
/// argument in place with a NUL), or `None` without one.
fn split_at(text: &str, separator: char) -> Option<(&str, &str)> {
    text.split_once(separator)
}

/// `SDL_Log()`.
macro_rules! sdl_log {
    ($($arg:tt)*) => {
        sdl3::log!($($arg)*)
    };
}

impl CommonState {
    /// Translation of `SDLTest_CommonArgParserFinalize()`.
    fn common_arg_parser_finalize(&mut self) {
        if !self.flags.contains(InitFlags::VIDEO) {
            self.video_usage = None;
        }
        if !self.flags.contains(InitFlags::AUDIO) {
            self.audio_usage = None;
        }
    }

    /// Translation of `SDLTest_CommonStateParseCommonArguments()`.
    fn parse_common_arguments(&mut self, index: usize) -> i32 {
        let argv = &self.argv;
        let arg = argv[index].as_str();
        let next = argv.get(index + 1).map(String::as_str);

        if eq(arg, "-h") || eq(arg, "--help") {
            /* Print the usage message */
            return -1;
        } else if eq(arg, "--trackmem") {
            /* Already handled in SDLTest_CommonCreateState() */
            return 1;
        } else if eq(arg, "--no-color") {
            set_color(false);
            return 1;
        } else if eq(arg, "--no-time") {
            crate::log::set_timestamps(false);
            return 1;
        } else if eq(arg, "--randmem") {
            /* Already handled in SDLTest_CommonCreateState() */
            return 1;
        } else if eq(arg, "--log") {
            let Some(next) = next else {
                return -1;
            };
            if eq(next, "all") {
                sdl_log::set_all_priorities(Priority::Verbose);
                return 2;
            } else if eq(next, "system") {
                sdl_log::set_priority(Category::System, Priority::Verbose);
                return 2;
            } else if eq(next, "audio") {
                sdl_log::set_priority(Category::Audio, Priority::Verbose);
                return 2;
            } else if eq(next, "video") {
                sdl_log::set_priority(Category::Video, Priority::Verbose);
                return 2;
            } else if eq(next, "render") {
                sdl_log::set_priority(Category::Render, Priority::Verbose);
                return 2;
            } else if eq(next, "input") {
                sdl_log::set_priority(Category::Input, Priority::Verbose);
                return 2;
            }
            return -1;
        } else if eq(arg, "--info") {
            let Some(next) = next else {
                return -1;
            };
            if eq(next, "all") {
                self.verbose |= VerboseFlags::VIDEO
                    | VerboseFlags::MODES
                    | VerboseFlags::RENDER
                    | VerboseFlags::EVENT;
                return 2;
            } else if eq(next, "video") {
                self.verbose |= VerboseFlags::VIDEO;
                return 2;
            } else if eq(next, "modes") {
                self.verbose |= VerboseFlags::MODES;
                return 2;
            } else if eq(next, "render") {
                self.verbose |= VerboseFlags::RENDER;
                return 2;
            } else if eq(next, "event") {
                self.verbose |= VerboseFlags::EVENT;
                return 2;
            } else if eq(next, "event_motion") {
                self.verbose |= VerboseFlags::EVENT | VerboseFlags::MOTION;
                return 2;
            }
            return -1;
        } else if arg == "-NSDocumentRevisionsDebugMode" {
            /* Debug flag sent by Xcode */
            return 2;
        }
        0
    }

    /// Translation of `SDLTest_CommonStateParseVideoArguments()`.
    fn parse_video_arguments(&mut self, index: usize) -> i32 {
        let argv = &self.argv;
        let arg = argv[index].as_str();
        let next = argv.get(index + 1).cloned();

        // (the options that take a value, returning -1 without one)
        macro_rules! value {
            () => {
                match next.as_deref() {
                    Some(next) => next,
                    None => return -1,
                }
            };
        }
        // (WxH)
        macro_rules! size {
            () => {
                match split_at(value!(), 'x') {
                    Some((w, h)) => (atoi(w), atoi(h)),
                    None => return -1,
                }
            };
        }

        if !self.flags.contains(InitFlags::VIDEO) {
            return 0;
        } else if eq(arg, "--video") {
            let driver = value!().to_owned();
            let _ = hints::set(hints::VIDEO_DRIVER, &driver);
            self.videodriver = Some(driver);
            return 2;
        } else if eq(arg, "--renderer") {
            let driver = value!().to_owned();
            let _ = hints::set(hints::RENDER_DRIVER, &driver);
            self.renderdriver = Some(driver);
            return 2;
        } else if eq(arg, "--gldebug") {
            self.gl_debug = 1;
            return 1;
        } else if eq(arg, "--display") {
            self.display_index = atoi(value!());
            return 2;
        } else if eq(arg, "--metal-window") {
            self.window_flags |= WindowFlags::METAL;
            return 1;
        } else if eq(arg, "--opengl-window") {
            self.window_flags |= WindowFlags::OPENGL;
            return 1;
        } else if eq(arg, "--vulkan-window") {
            self.window_flags |= WindowFlags::VULKAN;
            return 1;
        } else if eq(arg, "--fill-document") {
            self.window_flags |= WindowFlags::FILL_DOCUMENT;
            self.num_windows = 1;
            return 1;
        } else if eq(arg, "--fullscreen") {
            self.window_flags |= WindowFlags::FULLSCREEN;
            self.fullscreen_exclusive = true;
            self.num_windows = 1;
            return 1;
        } else if eq(arg, "--fullscreen-desktop") {
            self.window_flags |= WindowFlags::FULLSCREEN;
            self.fullscreen_exclusive = false;
            self.num_windows = 1;
            return 1;
        } else if eq(arg, "--windows") {
            let Some(next) = next.as_deref() else {
                return -1;
            };
            if !next.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                return -1;
            } else if !self.window_flags.contains(WindowFlags::FULLSCREEN) {
                self.num_windows = atoi(next);
            }
            return 2;
        } else if eq(arg, "--title") {
            self.window_title = value!().to_owned();
            return 2;
        } else if eq(arg, "--icon") {
            self.window_icon = Some(value!().to_owned());
            return 2;
        } else if eq(arg, "--center") {
            self.window_x = video::WINDOWPOS_CENTERED;
            self.window_y = video::WINDOWPOS_CENTERED;
            return 1;
        } else if eq(arg, "--position") {
            let Some((x, y)) = split_at(value!(), ',') else {
                return -1;
            };
            self.window_x = atoi(x);
            self.window_y = atoi(y);
            return 2;
        } else if eq(arg, "--confine-cursor") {
            // SEARCHARG(): each part ends at a comma, and the last one too.
            let value = value!();
            let Some((x, rest)) = split_at(value, ',') else {
                return -1;
            };
            let Some((y, rest)) = split_at(rest, ',') else {
                return -1;
            };
            let Some((w, rest)) = split_at(rest, ',') else {
                return -1;
            };
            // Note (upstream): SEARCHARG(h) looks for a comma after the
            // height too, so "X,Y,W,H" is refused and "X,Y,W,H," is taken.
            let Some((h, _)) = split_at(rest, ',') else {
                return -1;
            };
            self.confine.x = atoi(x);
            self.confine.y = atoi(y);
            self.confine.w = atoi(w);
            self.confine.h = atoi(h);
            return 2;
        } else if eq(arg, "--usable-bounds") {
            self.fill_usable_bounds = true;
            return 1;
        } else if eq(arg, "--geometry") {
            let (w, h) = size!();
            self.window_w = w;
            self.window_h = h;
            return 2;
        } else if eq(arg, "--min-geometry") {
            let (w, h) = size!();
            self.window_min_w = w;
            self.window_min_h = h;
            return 2;
        } else if eq(arg, "--max-geometry") {
            let (w, h) = size!();
            self.window_max_w = w;
            self.window_max_h = h;
            return 2;
        } else if eq(arg, "--aspect") {
            let value = value!();
            let (min_aspect, max_aspect) = split_at(value, '-').unwrap_or((value, value));
            self.window_min_aspect = atof(min_aspect) as f32;
            self.window_max_aspect = atof(max_aspect) as f32;
            return 2;
        } else if eq(arg, "--logical") {
            let (w, h) = size!();
            self.logical_w = w;
            self.logical_h = h;
            return 2;
        } else if eq(arg, "--high-pixel-density") {
            self.window_flags |= WindowFlags::HIGH_PIXEL_DENSITY;
            return 1;
        } else if eq(arg, "--auto-scale-content") {
            self.auto_scale_content = true;

            if self.logical_presentation == LogicalPresentation::Disabled {
                self.logical_presentation = LogicalPresentation::Stretch;
            }
            return 1;
        } else if eq(arg, "--logical-presentation") {
            let value = value!();
            if eq(value, "disabled") {
                self.logical_presentation = LogicalPresentation::Disabled;
                return 2;
            }
            if eq(value, "stretch") {
                self.logical_presentation = LogicalPresentation::Stretch;
                return 2;
            }
            if eq(value, "letterbox") {
                self.logical_presentation = LogicalPresentation::Letterbox;
                return 2;
            }
            if eq(value, "overscan") {
                self.logical_presentation = LogicalPresentation::Overscan;
                return 2;
            }
            if eq(value, "integer_scale") {
                self.logical_presentation = LogicalPresentation::IntegerScale;
                return 2;
            }
            return -1;
        } else if eq(arg, "--scale") {
            self.scale = atof(value!()) as f32;
            return 2;
        } else if eq(arg, "--depth") {
            self.depth = atoi(value!());
            return 2;
        } else if eq(arg, "--refresh") {
            self.refresh_rate = atof(value!()) as f32;
            return 2;
        } else if eq(arg, "--vsync") {
            self.render_vsync = 1;
            return 1;
        } else if eq(arg, "--noframe") {
            self.window_flags |= WindowFlags::BORDERLESS;
            return 1;
        } else if eq(arg, "--quit-after-ms") {
            self.quit_after_ms_interval = atoi(value!());
            if self.quit_after_ms_interval <= 0 {
                return -1;
            }
            return 2;
        } else if eq(arg, "--resizable") {
            self.window_flags |= WindowFlags::RESIZABLE;
            return 1;
        } else if eq(arg, "--transparent") {
            self.window_flags |= WindowFlags::TRANSPARENT;
            return 1;
        } else if eq(arg, "--always-on-top") {
            self.window_flags |= WindowFlags::ALWAYS_ON_TOP;
            return 1;
        } else if eq(arg, "--minimize") {
            self.window_flags |= WindowFlags::MINIMIZED;
            return 1;
        } else if eq(arg, "--maximize") {
            self.window_flags |= WindowFlags::MAXIMIZED;
            return 1;
        } else if eq(arg, "--hidden") {
            self.window_flags |= WindowFlags::HIDDEN;
            return 1;
        } else if eq(arg, "--input-focus") {
            self.window_flags |= WindowFlags::INPUT_FOCUS;
            return 1;
        } else if eq(arg, "--mouse-focus") {
            self.window_flags |= WindowFlags::MOUSE_FOCUS;
            return 1;
        } else if eq(arg, "--flash-on-focus-loss") {
            self.flash_on_focus_loss = true;
            return 1;
        } else if eq(arg, "--grab") {
            self.window_flags |= WindowFlags::MOUSE_GRABBED;
            return 1;
        } else if eq(arg, "--keyboard-grab") {
            self.window_flags |= WindowFlags::KEYBOARD_GRABBED;
            return 1;
        } else if eq(arg, "--utility") {
            self.window_flags |= WindowFlags::UTILITY;
            return 1;
        } else if eq(arg, "--hide-cursor") {
            self.hide_cursor = true;
            return 1;
        } else if eq(arg, "--gpu") {
            let driver = value!().to_owned();
            let _ = hints::set(hints::GPU_DRIVER, &driver);
            self.gpudriver = Some(driver);
            return 2;
        }
        0
    }

    /// Translation of `SDLTest_CommonStateParseAudioArguments()`.
    fn parse_audio_arguments(&mut self, index: usize) -> i32 {
        let argv = &self.argv;
        let arg = argv[index].as_str();
        let next = argv.get(index + 1).cloned();

        if !self.flags.contains(InitFlags::AUDIO) {
            return 0;
        } else if eq(arg, "--audio") {
            let Some(driver) = next else {
                return -1;
            };
            let _ = hints::set(hints::AUDIO_DRIVER, &driver);
            self.audiodriver = Some(driver);
            return 2;
        } else if eq(arg, "--rate") {
            let Some(next) = next else {
                return -1;
            };
            self.audio_freq = atoi(&next);
            return 2;
        } else if eq(arg, "--format") {
            let Some(next) = next else {
                return -1;
            };
            for (name, format) in [
                ("U8", AudioFormat::U8),
                ("S8", AudioFormat::S8),
                ("S16", AudioFormat::S16),
                ("S16LE", AudioFormat::S16LE),
                ("S16BE", AudioFormat::S16BE),
                ("S32", AudioFormat::S32),
                ("S32LE", AudioFormat::S32LE),
                ("S32BE", AudioFormat::S32BE),
                ("F32", AudioFormat::F32),
                ("F32LE", AudioFormat::F32LE),
                ("F32BE", AudioFormat::F32BE),
            ] {
                if eq(&next, name) {
                    self.audio_format = format;
                    return 2;
                }
            }
            return -1;
        } else if eq(arg, "--channels") {
            let Some(next) = next else {
                return -1;
            };
            self.audio_channels = atoi(&next) as u8 as i32;
            return 2;
        }
        0
    }

    /// Parse command line parameters and create common state, for the
    /// subsystems in `flags` (`InitFlags::VIDEO | InitFlags::AUDIO`, ...).
    /// `--trackmem` and `--randmem` start the memory tracking right away.
    /// Translation of `SDLTest_CommonCreateState()`.
    pub fn new(argv: Vec<String>, flags: InitFlags) -> CommonState {
        set_color(!get_environment_bool_variable("NO_COLOR"));

        /* Do this first so we catch all allocations */
        for arg in argv.iter().skip(1) {
            if eq(arg, "--trackmem") {
                crate::memory::track_allocations();
            } else if eq(arg, "--randmem") {
                crate::memory::rand_fill_allocations();
            }
        }

        /* Initialize some defaults */
        let window_title = argv.first().cloned().unwrap_or_default();
        CommonState {
            argv,
            flags,
            verbose: VerboseFlags::default(),
            videodriver: None,
            display_index: 0,
            display_id: 0,
            window_title,
            window_icon: None,
            window_flags: WindowFlags::HIDDEN,
            flash_on_focus_loss: false,
            window_x: video::WINDOWPOS_UNDEFINED,
            window_y: video::WINDOWPOS_UNDEFINED,
            window_w: DEFAULT_WINDOW_WIDTH,
            window_h: DEFAULT_WINDOW_HEIGHT,
            window_min_w: 0,
            window_min_h: 0,
            window_max_w: 0,
            window_max_h: 0,
            window_min_aspect: 0.0,
            window_max_aspect: 0.0,
            logical_w: 0,
            logical_h: 0,
            auto_scale_content: false,
            logical_presentation: LogicalPresentation::Disabled,
            scale: 0.0,
            depth: 0,
            refresh_rate: 0.0,
            fill_usable_bounds: false,
            fullscreen_exclusive: false,
            fullscreen_mode: DisplayMode::default(),
            num_windows: 1,
            windows: Vec::new(),
            gpudriver: None,
            renderdriver: None,
            render_vsync: 0,
            skip_renderer: false,
            renderers: Vec::new(),
            targets: Vec::new(),
            audiodriver: None,
            audio_format: AudioFormat::S16,
            audio_channels: 2,
            audio_freq: 22050,
            audio_id: 0,

            /* Set some very sane GL defaults */
            gl_red_size: 8,
            gl_green_size: 8,
            gl_blue_size: 8,
            gl_alpha_size: 8,
            gl_buffer_size: 0,
            gl_depth_size: 16,
            gl_stencil_size: 0,
            gl_double_buffer: 1,
            gl_accum_red_size: 0,
            gl_accum_green_size: 0,
            gl_accum_blue_size: 0,
            gl_accum_alpha_size: 0,
            gl_stereo: 0,
            gl_release_behavior: 0,
            gl_multisamplebuffers: 0,
            gl_multisamplesamples: 0,
            gl_retained_backing: 1,
            gl_accelerated: -1,
            gl_major_version: 0,
            gl_minor_version: 0,
            gl_debug: 0,
            gl_profile_mask: 0,

            confine: Rect::default(),
            hide_cursor: false,
            quit_after_ms_interval: 0,
            quit_after_ms_timer: None,

            video_usage: Some(&VIDEO_USAGE),
            audio_usage: Some(&AUDIO_USAGE),
            argparsers: Vec::new(),
        }
    }

    /// Append a parser of more command line options after the common
    /// state's own (and those added before), as the harness does for its
    /// options. Translation of appending to `SDLTest_CommonState.argparser`.
    pub fn add_argument_parser(&mut self, argparser: Box<dyn ArgumentParser>) {
        self.argparsers.push(argparser);
    }

    /// Process one common argument at `argv[index]`.
    ///
    /// Returns the number of arguments processed (i.e. 1 for
    /// `--fullscreen`, 2 for `--video [videodriver]`), 0 if it isn't a
    /// common one, or -1 on error. Translation of `SDLTest_CommonArg()`.
    pub fn common_arg(&mut self, index: usize) -> i32 {
        if index >= self.argv.len() {
            return 0;
        }

        /* Go back and parse arguments as we go */
        let consumed = self.parse_common_arguments(index);
        if consumed != 0 {
            return consumed;
        }
        let consumed = self.parse_video_arguments(index);
        if consumed != 0 {
            return consumed;
        }
        let consumed = self.parse_audio_arguments(index);
        if consumed != 0 {
            return consumed;
        }
        let argv = &self.argv;
        for argparser in &mut self.argparsers {
            let consumed = argparser.parse_arguments(argv, index);
            if consumed != 0 {
                return consumed;
            }
        }
        0
    }

    /// Logs command line usage info.
    ///
    /// This logs the appropriate command line options for the subsystems in
    /// use plus other common options, and then any application-specific
    /// `options`. This uses `SDL_Log()` and splits up output to be friendly
    /// to 80-character-wide terminals. Translation of
    /// `SDLTest_CommonLogUsage()`.
    pub fn log_usage(&mut self, argv0: &str, options: Option<&[&str]>) {
        sdl_log!("USAGE: {}", argv0);

        self.common_arg_parser_finalize();
        for usage in [Some(&COMMON_USAGE[..]), self.video_usage, self.audio_usage]
            .into_iter()
            .flatten()
        {
            for option in usage {
                sdl_log!("    {}", option);
            }
        }
        for argparser in &mut self.argparsers {
            argparser.finalize();
            if let Some(usage) = argparser.usage() {
                for option in usage {
                    sdl_log!("    {}", option);
                }
            }
        }
        if let Some(options) = options {
            for option in options {
                sdl_log!("    {}", option);
            }
        }
    }

    /// Easy argument handling when test app doesn't need any custom args:
    /// parses the state's `argv`, logging the usage on a bad or unknown one.
    ///
    /// Returns `false` if app should quit, `true` otherwise. Translation of
    /// `SDLTest_CommonDefaultArgs()`.
    pub fn default_args(&mut self) -> bool {
        let mut i = 1;
        while i < self.argv.len() {
            let consumed = self.common_arg(i);
            if consumed <= 0 {
                let argv0 = self.argv[0].clone();
                self.log_usage(&argv0, None);
                return false;
            }
            i += consumed as usize;
        }
        true
    }
}

/// Translation of `SDLTest_PrintDisplayOrientation()`.
fn print_display_orientation(text: &mut String, orientation: DisplayOrientation) {
    text.push_str(match orientation {
        DisplayOrientation::Unknown => "UNKNOWN",
        DisplayOrientation::Landscape => "LANDSCAPE",
        DisplayOrientation::LandscapeFlipped => "LANDSCAPE_FLIPPED",
        DisplayOrientation::Portrait => "PORTRAIT",
        DisplayOrientation::PortraitFlipped => "PORTRAIT_FLIPPED",
    });
}

/// The window flags with their names, in upstream's order.
const WINDOW_FLAGS: [(WindowFlags, &str); 25] = [
    (WindowFlags::FULLSCREEN, "FULLSCREEN"),
    (WindowFlags::OPENGL, "OPENGL"),
    (WindowFlags::OCCLUDED, "OCCLUDED"),
    (WindowFlags::HIDDEN, "HIDDEN"),
    (WindowFlags::BORDERLESS, "BORDERLESS"),
    (WindowFlags::RESIZABLE, "RESIZABLE"),
    (WindowFlags::MINIMIZED, "MINIMIZED"),
    (WindowFlags::MAXIMIZED, "MAXIMIZED"),
    (WindowFlags::MOUSE_GRABBED, "MOUSE_GRABBED"),
    (WindowFlags::INPUT_FOCUS, "INPUT_FOCUS"),
    (WindowFlags::MOUSE_FOCUS, "MOUSE_FOCUS"),
    (WindowFlags::EXTERNAL, "EXTERNAL"),
    (WindowFlags::MODAL, "MODAL"),
    (WindowFlags::HIGH_PIXEL_DENSITY, "HIGH_PIXEL_DENSITY"),
    (WindowFlags::MOUSE_CAPTURE, "MOUSE_CAPTURE"),
    (WindowFlags::MOUSE_RELATIVE_MODE, "MOUSE_RELATIVE_MODE"),
    (WindowFlags::ALWAYS_ON_TOP, "ALWAYS_ON_TOP"),
    (WindowFlags::UTILITY, "UTILITY"),
    (WindowFlags::TOOLTIP, "TOOLTIP"),
    (WindowFlags::POPUP_MENU, "POPUP_MENU"),
    (WindowFlags::KEYBOARD_GRABBED, "KEYBOARD_GRABBED"),
    (WindowFlags::VULKAN, "VULKAN"),
    (WindowFlags::METAL, "METAL"),
    (WindowFlags::TRANSPARENT, "TRANSPARENT"),
    (WindowFlags::NOT_FOCUSABLE, "NOT_FOCUSABLE"),
];

/// Translation of `SDLTest_PrintWindowFlag()`.
fn print_window_flag(text: &mut String, flag: WindowFlags) {
    match WINDOW_FLAGS.iter().find(|(f, _)| *f == flag) {
        Some((_, name)) => text.push_str(name),
        None => text.push_str(&format!("0x{:016x}", flag.0)),
    }
}

/// Translation of `SDLTest_PrintWindowFlags()`.
fn print_window_flags(text: &mut String, flags: WindowFlags) {
    let mut count = 0;
    for &(flag, _) in &WINDOW_FLAGS {
        if flags.contains(flag) {
            if count > 0 {
                text.push_str(" | ");
            }
            print_window_flag(text, flag);
            count += 1;
        }
    }
}

/// The key modifiers with their names, in upstream's order.
const KMOD_FLAGS: [(Keymod, &str); 13] = [
    (Keymod::LSHIFT, "LSHIFT"),
    (Keymod::RSHIFT, "RSHIFT"),
    (Keymod::LEVEL5, "LEVEL5"),
    (Keymod::LCTRL, "LCTRL"),
    (Keymod::RCTRL, "RCTRL"),
    (Keymod::LALT, "LALT"),
    (Keymod::RALT, "RALT"),
    (Keymod::LGUI, "LGUI"),
    (Keymod::RGUI, "RGUI"),
    (Keymod::NUM, "NUM"),
    (Keymod::CAPS, "CAPS"),
    (Keymod::MODE, "MODE"),
    (Keymod::SCROLL, "SCROLL"),
];

/// Translation of `SDLTest_PrintModStateFlag()`.
fn print_mod_state_flag(text: &mut String, flag: Keymod) {
    match KMOD_FLAGS.iter().find(|(f, _)| *f == flag) {
        Some((_, name)) => text.push_str(name),
        None => text.push_str(&format!("0x{:08x}", flag.0 as u32)),
    }
}

/// Translation of `SDLTest_PrintModState()`.
fn print_mod_state(text: &mut String, keymod: Keymod) {
    let mut count = 0;
    for &(flag, _) in &KMOD_FLAGS {
        if keymod.contains(flag) {
            if count > 0 {
                text.push_str(" | ");
            }
            print_mod_state_flag(text, flag);
            count += 1;
        }
    }
}

/// Translation of `SDLTest_PrintButtonMask()`.
fn print_button_mask(text: &mut String, flags: MouseButtonFlags) {
    let mut count = 0;
    for i in 1..=32u8 {
        let flag = MouseButtonFlags::mask(i);
        if flags.contains(flag) {
            if count > 0 {
                text.push_str(" | ");
            }
            text.push_str(&format!("SDL_BUTTON_MASK({i})"));
            count += 1;
        }
    }
}

/// Translation of `SDLTest_PrintPixelFormat()`.
fn print_pixel_format(text: &mut String, format: PixelFormat) {
    let name = format.name();
    text.push_str(name.strip_prefix("SDL_PIXELFORMAT_").unwrap_or(name));
}

/// Translation of `SDLTest_PrintLogicalPresentation()`.
fn print_logical_presentation(text: &mut String, logical_presentation: LogicalPresentation) {
    text.push_str(match logical_presentation {
        LogicalPresentation::Disabled => "DISABLED",
        LogicalPresentation::Stretch => "STRETCH",
        LogicalPresentation::Letterbox => "LETTERBOX",
        LogicalPresentation::Overscan => "OVERSCAN",
        LogicalPresentation::IntegerScale => "INTEGER_SCALE",
    });
}

/// Translation of `SDLTest_PrintRenderer()`.
fn print_renderer(renderer: &Renderer) {
    let name = renderer.name();

    sdl_log!("  Renderer {}:", name);
    let props = renderer.properties();
    if name == "gpu" {
        let device =
            props.get_any::<sdl3::gpu::Device>(sdl3::render::PROP_RENDERER_GPU_DEVICE_POINTER);
        sdl_log!(
            "    Driver: {}",
            device
                .as_deref()
                .map_or("(null)", sdl3::gpu::Device::driver)
        );
    }
    sdl_log!(
        "    VSync: {}",
        props
            .get_number(sdl3::render::PROP_RENDERER_VSYNC_NUMBER)
            .unwrap_or(0) as i32
    );

    let texture_formats = renderer.texture_formats();
    if !texture_formats.is_empty() {
        let mut text = "    Texture formats: ".to_owned();
        for (i, &format) in texture_formats.iter().enumerate() {
            if i > 0 {
                text.push_str(", ");
            }
            print_pixel_format(&mut text, format);
        }
        sdl_log!("{}", text);
    }

    let max_texture_size = props
        .get_number(sdl3::render::PROP_RENDERER_MAX_TEXTURE_SIZE_NUMBER)
        .unwrap_or(0) as i32;
    if max_texture_size != 0 {
        sdl_log!(
            "    Max Texture Size: {}x{}",
            max_texture_size,
            max_texture_size
        );
    }
}

/// Translation of `SDLTest_LoadIcon()`.
fn load_icon(file: &str) -> Option<Surface<'static>> {
    /* Load the icon surface */
    match Surface::load(file) {
        Ok(icon) => Some(icon),
        Err(error) => {
            sdl_log!("Couldn't load {}: {}", file, error.message());
            None
        }
    }
}

/// Translation of `SDLTest_ExampleHitTestCallback()`.
fn example_hit_test_callback(win: WindowID, area: Point) -> HitTestResult {
    const RESIZE_BORDER: i32 = 8;
    const DRAGGABLE_TITLE: i32 = 32;

    /*SDL_Log("Hit test point %d,%d", area->x, area->y);*/

    let (w, h) = Window::from_id(win)
        .and_then(|win| win.size())
        .unwrap_or((0, 0));

    if area.x < RESIZE_BORDER {
        if area.y < RESIZE_BORDER {
            sdl_log!("SDL_HITTEST_RESIZE_TOPLEFT");
            return HitTestResult::ResizeTopLeft;
        } else if area.y >= (h - RESIZE_BORDER) {
            sdl_log!("SDL_HITTEST_RESIZE_BOTTOMLEFT");
            return HitTestResult::ResizeBottomLeft;
        } else {
            sdl_log!("SDL_HITTEST_RESIZE_LEFT");
            return HitTestResult::ResizeLeft;
        }
    } else if area.x >= (w - RESIZE_BORDER) {
        if area.y < RESIZE_BORDER {
            sdl_log!("SDL_HITTEST_RESIZE_TOPRIGHT");
            return HitTestResult::ResizeTopRight;
        } else if area.y >= (h - RESIZE_BORDER) {
            sdl_log!("SDL_HITTEST_RESIZE_BOTTOMRIGHT");
            return HitTestResult::ResizeBottomRight;
        } else {
            sdl_log!("SDL_HITTEST_RESIZE_RIGHT");
            return HitTestResult::ResizeRight;
        }
    } else if area.y >= (h - RESIZE_BORDER) {
        sdl_log!("SDL_HITTEST_RESIZE_BOTTOM");
        return HitTestResult::ResizeBottom;
    } else if area.y < RESIZE_BORDER {
        sdl_log!("SDL_HITTEST_RESIZE_TOP");
        return HitTestResult::ResizeTop;
    } else if area.y < DRAGGABLE_TITLE {
        sdl_log!("SDL_HITTEST_DRAGGABLE");
        return HitTestResult::Draggable;
    }
    HitTestResult::Normal
}

/// Log the reason of a failure, and make it the error.
fn fail(message: String) -> Result<()> {
    sdl_log!("{}", message);
    Err(Error::new(message))
}

impl CommonState {
    /// Open test window (and the renderers, and the audio device) as the
    /// options say.
    ///
    /// Returns the error (also logged) if something can't be made.
    /// Translation of `SDLTest_CommonInit()`.
    pub fn init(&mut self) -> Result<()> {
        if self.flags.contains(InitFlags::VIDEO) {
            if self.verbose.intersects(VerboseFlags::VIDEO) {
                let n = video::num_video_drivers();
                if n == 0 {
                    sdl_log!("No built-in video drivers");
                } else {
                    let mut text = "Built-in video drivers:".to_owned();
                    for i in 0..n {
                        if i > 0 {
                            text.push(',');
                        }
                        text.push_str(&format!(" {}", video::video_driver(i).unwrap_or("")));
                    }
                    sdl_log!("{}", text);
                }
            }
            if let Err(error) = sdl3::init::init_subsystem(InitFlags::VIDEO) {
                return fail(format!(
                    "Couldn't initialize video driver: {}",
                    error.message()
                ));
            }
            if self.verbose.intersects(VerboseFlags::VIDEO) {
                sdl_log!(
                    "Video driver: {}",
                    video::current_video_driver().unwrap_or("(null)")
                );
            }

            /* Upload GL settings */
            let _ = gl_set_attribute(GlAttr::RedSize, self.gl_red_size);
            let _ = gl_set_attribute(GlAttr::GreenSize, self.gl_green_size);
            let _ = gl_set_attribute(GlAttr::BlueSize, self.gl_blue_size);
            let _ = gl_set_attribute(GlAttr::AlphaSize, self.gl_alpha_size);
            let _ = gl_set_attribute(GlAttr::DoubleBuffer, self.gl_double_buffer);
            let _ = gl_set_attribute(GlAttr::BufferSize, self.gl_buffer_size);
            let _ = gl_set_attribute(GlAttr::DepthSize, self.gl_depth_size);
            let _ = gl_set_attribute(GlAttr::StencilSize, self.gl_stencil_size);
            let _ = gl_set_attribute(GlAttr::AccumRedSize, self.gl_accum_red_size);
            let _ = gl_set_attribute(GlAttr::AccumGreenSize, self.gl_accum_green_size);
            let _ = gl_set_attribute(GlAttr::AccumBlueSize, self.gl_accum_blue_size);
            let _ = gl_set_attribute(GlAttr::AccumAlphaSize, self.gl_accum_alpha_size);
            let _ = gl_set_attribute(GlAttr::Stereo, self.gl_stereo);
            let _ = gl_set_attribute(GlAttr::ContextReleaseBehavior, self.gl_release_behavior);
            let _ = gl_set_attribute(GlAttr::MultisampleBuffers, self.gl_multisamplebuffers);
            let _ = gl_set_attribute(GlAttr::MultisampleSamples, self.gl_multisamplesamples);
            if self.gl_accelerated >= 0 {
                let _ = gl_set_attribute(GlAttr::AcceleratedVisual, self.gl_accelerated);
            }
            let _ = gl_set_attribute(GlAttr::RetainedBacking, self.gl_retained_backing);
            if self.gl_major_version != 0 {
                let _ = gl_set_attribute(GlAttr::ContextMajorVersion, self.gl_major_version);
                let _ = gl_set_attribute(GlAttr::ContextMinorVersion, self.gl_minor_version);
            }
            if self.gl_debug != 0 {
                let _ = gl_set_attribute(GlAttr::ContextFlags, GL_CONTEXT_DEBUG_FLAG);
            }
            if self.gl_profile_mask != 0 {
                let _ = gl_set_attribute(GlAttr::ContextProfileMask, self.gl_profile_mask);
            }

            if self.verbose.intersects(VerboseFlags::MODES) {
                log_modes();
            }

            if self.verbose.intersects(VerboseFlags::RENDER) {
                let n = sdl3::render::num_render_drivers();
                if n == 0 {
                    sdl_log!("No built-in render drivers");
                } else {
                    sdl_log!("Built-in render drivers:");
                    for i in 0..n {
                        sdl_log!("  {}", sdl3::render::render_driver(i).unwrap_or(""));
                    }
                }
            }

            self.display_id = video::primary_display().unwrap_or(0);
            if self.display_index > 0 {
                let displays = video::displays().unwrap_or_default();
                if (self.display_index as usize) < displays.len() {
                    self.display_id = displays[self.display_index as usize];
                }

                if video::windowpos_is_undefined(self.window_x) {
                    self.window_x = video::windowpos_undefined_display(self.display_id);
                    self.window_y = video::windowpos_undefined_display(self.display_id);
                } else if video::windowpos_is_centered(self.window_x) {
                    self.window_x = video::windowpos_centered_display(self.display_id);
                    self.window_y = video::windowpos_centered_display(self.display_id);
                }
            }

            {
                let include_high_density_modes =
                    self.window_flags.contains(WindowFlags::HIGH_PIXEL_DENSITY);
                if let Ok(mode) = video::closest_fullscreen_display_mode(
                    self.display_id,
                    self.window_w,
                    self.window_h,
                    self.refresh_rate,
                    include_high_density_modes,
                ) {
                    self.fullscreen_mode = mode;
                }
            }

            self.windows = Vec::with_capacity(self.num_windows.max(0) as usize);
            self.renderers = Vec::with_capacity(self.num_windows.max(0) as usize);
            self.targets = Vec::with_capacity(self.num_windows.max(0) as usize);
            for i in 0..self.num_windows {
                let r = if self.fill_usable_bounds {
                    video::display_usable_bounds(self.display_id).unwrap_or_default()
                } else {
                    let mut r =
                        Rect::new(self.window_x, self.window_y, self.window_w, self.window_h);
                    if self.auto_scale_content {
                        let scale = video::display_content_scale(self.display_id).unwrap_or(0.0);
                        r.w = (r.w as f32 * scale).ceil() as i32;
                        r.h = (r.h as f32 * scale).ceil() as i32;
                    }
                    r
                };

                let title = if self.num_windows > 1 {
                    format!("{} {}", self.window_title, i + 1)
                } else {
                    self.window_title.clone()
                };
                let props = Properties::new();
                let _ = props.set(video_window::PROP_WINDOW_CREATE_TITLE_STRING, title);
                let _ = props.set(video_window::PROP_WINDOW_CREATE_X_NUMBER, r.x as i64);
                let _ = props.set(video_window::PROP_WINDOW_CREATE_Y_NUMBER, r.y as i64);
                let _ = props.set(video_window::PROP_WINDOW_CREATE_WIDTH_NUMBER, r.w as i64);
                let _ = props.set(video_window::PROP_WINDOW_CREATE_HEIGHT_NUMBER, r.h as i64);
                let _ = props.set(
                    video_window::PROP_WINDOW_CREATE_FLAGS_NUMBER,
                    self.window_flags.0 as i64,
                );
                let window = match Window::create_with_properties(&props) {
                    Ok(window) => window,
                    Err(error) => {
                        return fail(format!("Couldn't create window: {}", error.message()));
                    }
                };
                drop(props);
                self.windows.push(window);
                self.renderers.push(None);
                self.targets.push(None);
                if self.window_min_w != 0 || self.window_min_h != 0 {
                    let _ = window.set_minimum_size(self.window_min_w, self.window_min_h);
                }
                if self.window_max_w != 0 || self.window_max_h != 0 {
                    let _ = window.set_maximum_size(self.window_max_w, self.window_max_h);
                }
                if self.window_min_aspect != 0.0 || self.window_max_aspect != 0.0 {
                    let _ = window.set_aspect_ratio(self.window_min_aspect, self.window_max_aspect);
                }
                let (w, h) = window.size().unwrap_or((0, 0));
                if !self.window_flags.contains(WindowFlags::RESIZABLE) && (w != r.w || h != r.h) {
                    sdl_log!("Window requested size {}x{}, got {}x{}", r.w, r.h, w, h);
                    self.window_w = w;
                    self.window_h = h;
                }
                if self.window_flags.contains(WindowFlags::FULLSCREEN) {
                    if self.fullscreen_exclusive {
                        let _ = window.set_fullscreen_mode(Some(&self.fullscreen_mode));
                    }
                    let _ = window.set_fullscreen(true);
                }

                /* Add resize/drag areas for windows that are borderless and resizable */
                if self
                    .window_flags
                    .contains(WindowFlags::RESIZABLE | WindowFlags::BORDERLESS)
                {
                    let _ = window.set_hit_test(Some(Arc::new(example_hit_test_callback)));
                }

                if let Some(window_icon) = &self.window_icon {
                    if let Some(icon) = load_icon(window_icon) {
                        let _ = window.set_icon(&icon);
                    }
                }

                if !self.confine.is_empty() {
                    let _ = window.set_mouse_rect(Some(&self.confine));
                }

                if !self.skip_renderer
                    && (self.renderdriver.is_some()
                        || !self.window_flags.intersects(
                            WindowFlags::OPENGL | WindowFlags::VULKAN | WindowFlags::METAL,
                        ))
                {
                    let mut renderer =
                        match Renderer::for_window(&window, self.renderdriver.as_deref()) {
                            Ok(renderer) => renderer,
                            Err(error) => {
                                return fail(format!(
                                    "Couldn't create renderer: {}",
                                    error.message()
                                ));
                            }
                        };
                    if self.logical_w == 0 || self.logical_h == 0 {
                        self.logical_w = self.window_w;
                        self.logical_h = self.window_h;
                    }
                    if self.render_vsync != 0 {
                        let _ = renderer.set_vsync(self.render_vsync);
                    }
                    let presentation = renderer.set_logical_presentation(
                        self.logical_w,
                        self.logical_h,
                        self.logical_presentation,
                    );
                    if let Err(error) = presentation {
                        self.renderers[i as usize] = Some(renderer);
                        return fail(format!(
                            "Couldn't set logical presentation: {}",
                            error.message()
                        ));
                    }
                    if self.scale != 0.0 {
                        let _ = renderer.set_scale(self.scale, self.scale);
                    }
                    if self.verbose.intersects(VerboseFlags::RENDER) {
                        sdl_log!("Current renderer:");
                        print_renderer(&renderer);
                    }
                    self.renderers[i as usize] = Some(renderer);
                }

                let _ = window.show();
            }
            if self.hide_cursor {
                sdl3::events::mouse::hide_cursor();
            }
        }

        if self.flags.contains(InitFlags::AUDIO) {
            if self.verbose.intersects(VerboseFlags::AUDIO) {
                let n = sdl3::audio::num_audio_drivers();
                if n == 0 {
                    sdl_log!("No built-in audio drivers");
                } else {
                    let mut text = "Built-in audio drivers:".to_owned();
                    for i in 0..n {
                        if i > 0 {
                            text.push(',');
                        }
                        text.push_str(&format!(" {}", sdl3::audio::audio_driver(i).unwrap_or("")));
                    }
                    sdl_log!("{}", text);
                }
            }
            if let Err(error) = sdl3::init::init_subsystem(InitFlags::AUDIO) {
                return fail(format!(
                    "Couldn't initialize audio driver: {}",
                    error.message()
                ));
            }
            if self.verbose.intersects(VerboseFlags::AUDIO) {
                sdl_log!(
                    "Audio driver: {}",
                    sdl3::audio::current_audio_driver().unwrap_or("(null)")
                );
            }

            let spec = AudioSpec::new(self.audio_format, self.audio_channels, self.audio_freq);
            match sdl3::audio::open_audio_device(
                sdl3::audio::AUDIO_DEVICE_DEFAULT_PLAYBACK,
                Some(&spec),
            ) {
                Ok(audio_id) => self.audio_id = audio_id,
                Err(error) => {
                    return fail(format!("Couldn't open audio: {}", error.message()));
                }
            }
        }

        let _ = sdl3::init::init_subsystem(self.flags);

        if self.quit_after_ms_interval != 0 {
            self.quit_after_ms_timer = Timer::new(
                Duration::from_millis(self.quit_after_ms_interval as u64),
                |_| quit_after_ms_cb(),
            )
            .ok();
        }

        Ok(())
    }
}

/// The display modes part of [`CommonState::init`] (`VERBOSE_MODES`).
fn log_modes() {
    let displays = video::displays().unwrap_or_default();
    sdl_log!("Number of displays: {}", displays.len());
    for &display_id in &displays {
        sdl_log!(
            "Display {}: {}",
            display_id,
            video::display_name(display_id).unwrap_or_else(|_| "(null)".to_owned())
        );

        let bounds = video::display_bounds(display_id).unwrap_or_default();

        let usablebounds = video::display_usable_bounds(display_id).unwrap_or_default();

        sdl_log!(
            "Bounds: {}x{} at {},{}",
            bounds.w,
            bounds.h,
            bounds.x,
            bounds.y
        );
        sdl_log!(
            "Usable bounds: {}x{} at {},{}",
            usablebounds.w,
            usablebounds.h,
            usablebounds.x,
            usablebounds.y
        );

        if let Ok(mode) = video::desktop_display_mode(display_id) {
            let masks = mode.format.masks().unwrap_or_default();
            sdl_log!(
                "  Desktop mode: {}x{}@{}x {}Hz, {} bits-per-pixel ({})",
                mode.w,
                mode.h,
                fmt_g(mode.pixel_density as f64),
                fmt_g(mode.refresh_rate as f64),
                masks.bpp,
                mode.format.name()
            );
            if masks.r != 0 || masks.g != 0 || masks.b != 0 {
                sdl_log!("      Red Mask   = 0x{:08x}", masks.r);
                sdl_log!("      Green Mask = 0x{:08x}", masks.g);
                sdl_log!("      Blue Mask  = 0x{:08x}", masks.b);
                if masks.a != 0 {
                    sdl_log!("      Alpha Mask = 0x{:08x}", masks.a);
                }
            }
        }

        /* Print available fullscreen video modes */
        let modes = video::fullscreen_display_modes(display_id).unwrap_or_default();
        if modes.is_empty() {
            sdl_log!("No available fullscreen video modes");
        } else {
            sdl_log!("  Fullscreen video modes:");
            for (j, mode) in modes.iter().enumerate() {
                let masks = mode.format.masks().unwrap_or_default();
                sdl_log!(
                    "    Mode {}: {}x{}@{}x {}Hz, {} bits-per-pixel ({})",
                    j,
                    mode.w,
                    mode.h,
                    fmt_g(mode.pixel_density as f64),
                    fmt_g(mode.refresh_rate as f64),
                    masks.bpp,
                    mode.format.name()
                );
                if masks.r != 0 || masks.g != 0 || masks.b != 0 {
                    sdl_log!("        Red Mask   = 0x{:08x}", masks.r);
                    sdl_log!("        Green Mask = 0x{:08x}", masks.g);
                    sdl_log!("        Blue Mask  = 0x{:08x}", masks.b);
                    if masks.a != 0 {
                        sdl_log!("        Alpha Mask = 0x{:08x}", masks.a);
                    }
                }
            }
        }

        // (upstream prints the D3D9 adapter index and the DXGI adapter and
        // output indices on Windows here, from SDL_GetDirect3D9AdapterIndex()
        // and SDL_GetDXGIOutputInfo(), which sdl3 doesn't have yet)
    }
}

/// Translation of `SystemThemeName()`.
fn system_theme_name() -> &'static str {
    match video::system_theme() {
        SystemTheme::Unknown => "UNKNOWN",
        SystemTheme::Light => "LIGHT",
        SystemTheme::Dark => "DARK",
    }
}

/// Translation of `DisplayOrientationName()`.
fn display_orientation_name(orientation: i32) -> &'static str {
    match orientation {
        0 => "UNKNOWN",
        1 => "LANDSCAPE",
        2 => "LANDSCAPE_FLIPPED",
        3 => "PORTRAIT",
        4 => "PORTRAIT_FLIPPED",
        _ => "???",
    }
}

/// Translation of `GamepadAxisName()`.
fn gamepad_axis_name(axis: i32) -> &'static str {
    use sdl3::gamepad::GamepadAxis as A;
    for (value, name) in [
        (A::Invalid, "INVALID"),
        (A::LeftX, "LEFTX"),
        (A::LeftY, "LEFTY"),
        (A::RightX, "RIGHTX"),
        (A::RightY, "RIGHTY"),
        (A::LeftTrigger, "LEFT_TRIGGER"),
        (A::RightTrigger, "RIGHT_TRIGGER"),
    ] {
        if axis == value as i32 {
            return name;
        }
    }
    "???"
}

/// Translation of `GamepadButtonName()`.
fn gamepad_button_name(button: i32) -> &'static str {
    use sdl3::gamepad::GamepadButton as B;
    for (value, name) in [
        (B::Invalid, "INVALID"),
        (B::South, "SOUTH"),
        (B::East, "EAST"),
        (B::West, "WEST"),
        (B::North, "NORTH"),
        (B::Back, "BACK"),
        (B::Guide, "GUIDE"),
        (B::Start, "START"),
        (B::LeftStick, "LEFT_STICK"),
        (B::RightStick, "RIGHT_STICK"),
        (B::LeftShoulder, "LEFT_SHOULDER"),
        (B::RightShoulder, "RIGHT_SHOULDER"),
        (B::DpadUp, "DPAD_UP"),
        (B::DpadDown, "DPAD_DOWN"),
        (B::DpadLeft, "DPAD_LEFT"),
        (B::DpadRight, "DPAD_RIGHT"),
    ] {
        if button == value as i32 {
            return name;
        }
    }
    "???"
}

/// Translation of `CapSenseName()`.
fn cap_sense_name(capsense: i32) -> &'static str {
    use sdl3::gamepad::GamepadCapSenseType as C;
    for (value, name) in [
        (C::Invalid, "INVALID"),
        (C::LeftStick, "LEFT_STICK"),
        (C::RightStick, "RIGHT_STICK"),
        (C::LeftGrip, "LEFT_GRIP"),
        (C::RightGrip, "RIGHT_GRIP"),
    ] {
        if capsense == value as i32 {
            return name;
        }
    }
    "???"
}

/// A name, or C's `(null)` for none.
fn or_null(name: Result<String>) -> String {
    name.unwrap_or_else(|_| "(null)".to_owned())
}

/// A device name that may be missing, or C's `(null)` for none.
fn or_null_opt(name: Result<Option<String>>) -> String {
    name.ok().flatten().unwrap_or_else(|| "(null)".to_owned())
}

/// `SDL_GetWindowFromEvent()`.
fn window_from_event(event: &Event) -> Option<Window> {
    Window::from_id(event.window_id()?).ok()
}

/// Print the details of an event (with `SDL_Log()`). This is
/// automatically called by [`CommonState::event`] as needed. Translation
/// of `SDLTest_PrintEvent()`.
pub fn print_event(event: &Event) {
    let event_type = event.event_type();
    match (event_type, event) {
        (EventType::SYSTEM_THEME_CHANGED, _) => {
            sdl_log!("SDL EVENT: System theme changed to {}", system_theme_name());
        }
        (EventType::DISPLAY_ADDED, Event::Display(e)) => {
            sdl_log!("SDL EVENT: Display {} attached", e.display_id);
        }
        (EventType::DISPLAY_CONTENT_SCALE_CHANGED, Event::Display(e)) => {
            let scale = video::display_content_scale(e.display_id).unwrap_or(0.0);
            sdl_log!(
                "SDL EVENT: Display {} changed content scale to {}%",
                e.display_id,
                (scale * 100.0) as i32
            );
        }
        (EventType::DISPLAY_USABLE_BOUNDS_CHANGED, Event::Display(e)) => {
            let bounds = video::display_usable_bounds(e.display_id).unwrap_or_default();
            sdl_log!(
                "SDL EVENT: Display {} changed usable bounds to {}x{} at {},{}",
                e.display_id,
                bounds.w,
                bounds.h,
                bounds.x,
                bounds.y
            );
        }
        (EventType::DISPLAY_DESKTOP_MODE_CHANGED, Event::Display(e)) => {
            sdl_log!(
                "SDL EVENT: Display {} desktop mode changed to {}x{}",
                e.display_id,
                e.data1,
                e.data2
            );
        }
        (EventType::DISPLAY_CURRENT_MODE_CHANGED, Event::Display(e)) => {
            sdl_log!(
                "SDL EVENT: Display {} current mode changed to {}x{}",
                e.display_id,
                e.data1,
                e.data2
            );
        }
        (EventType::DISPLAY_MOVED, Event::Display(e)) => {
            sdl_log!("SDL EVENT: Display {} changed position", e.display_id);
        }
        (EventType::DISPLAY_ORIENTATION, Event::Display(e)) => {
            sdl_log!(
                "SDL EVENT: Display {} changed orientation to {}",
                e.display_id,
                display_orientation_name(e.data1)
            );
        }
        (EventType::DISPLAY_REMOVED, Event::Display(e)) => {
            sdl_log!("SDL EVENT: Display {} removed", e.display_id);
        }
        (_, Event::Window(e)) if print_window_event(event, e) => {}
        (EventType::KEYBOARD_ADDED, Event::KeyboardDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Keyboard {} ({}) attached",
                e.which,
                or_null(sdl3::events::keyboard::keyboard_name(e.which))
            );
        }
        (EventType::KEYBOARD_REMOVED, Event::KeyboardDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Keyboard {} ({}) removed",
                e.which,
                or_null(sdl3::events::keyboard::keyboard_name(e.which))
            );
        }
        (EventType::KEY_DOWN | EventType::KEY_UP, Event::Key(e)) => {
            let mut modstr = String::new();
            if e.modifiers != Keymod::NONE {
                print_mod_state(&mut modstr, e.modifiers);
                // (char modstr[64])
                modstr = crate::internal::truncate(modstr, 64);
            } else {
                modstr.push_str("NONE");
            }

            sdl_log!(
                "SDL EVENT: Keyboard: key {} in window {}: scancode 0x{:08X} = {}, keycode 0x{:08X} = {}, mods = {}",
                if e.down { "pressed" } else { "released" },
                e.window_id,
                e.scancode.0 as u32,
                e.scancode.name(),
                e.key.0,
                e.key.name(),
                modstr
            );
        }
        (EventType::TEXT_EDITING, Event::TextEditing(e)) => {
            sdl_log!(
                "SDL EVENT: Keyboard: text editing \"{}\" in window {}",
                e.text,
                e.window_id
            );
        }
        (EventType::TEXT_EDITING_CANDIDATES, Event::TextEditingCandidates(e)) => {
            sdl_log!(
                "SDL EVENT: Keyboard: text editing candidates in window {}",
                e.window_id
            );
        }
        (EventType::TEXT_INPUT, Event::TextInput(e)) => {
            sdl_log!(
                "SDL EVENT: Keyboard: text input \"{}\" in window {}",
                e.text,
                e.window_id
            );
        }
        (EventType::KEYMAP_CHANGED, _) => {
            sdl_log!("SDL EVENT: Keymap changed");
        }
        (EventType::MOUSE_ADDED, Event::MouseDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Mouse {} ({}) attached",
                e.which,
                or_null(sdl3::events::mouse::mouse_name(e.which))
            );
        }
        (EventType::MOUSE_REMOVED, Event::MouseDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Mouse {} ({}) removed",
                e.which,
                or_null(sdl3::events::mouse::mouse_name(e.which))
            );
        }
        (EventType::MOUSE_MOTION, Event::MouseMotion(e)) => {
            sdl_log!(
                "SDL EVENT: Mouse: moved to {},{} ({},{}) in window {}",
                fmt_g(e.x as f64),
                fmt_g(e.y as f64),
                fmt_g(e.xrel as f64),
                fmt_g(e.yrel as f64),
                e.window_id
            );
        }
        (EventType::MOUSE_BUTTON_DOWN, Event::MouseButton(e)) => {
            sdl_log!(
                "SDL EVENT: Mouse: button {} pressed at {},{} with click count {} in window {}",
                e.button,
                fmt_g(e.x as f64),
                fmt_g(e.y as f64),
                e.clicks,
                e.window_id
            );
        }
        (EventType::MOUSE_BUTTON_UP, Event::MouseButton(e)) => {
            sdl_log!(
                "SDL EVENT: Mouse: button {} released at {},{} with click count {} in window {}",
                e.button,
                fmt_g(e.x as f64),
                fmt_g(e.y as f64),
                e.clicks,
                e.window_id
            );
        }
        (EventType::MOUSE_WHEEL, Event::MouseWheel(e)) => {
            sdl_log!(
                "SDL EVENT: Mouse: wheel scrolled {} in x and {} in y (reversed: {}) in window {}",
                fmt_g(e.x as f64),
                fmt_g(e.y as f64),
                e.direction as i32,
                e.window_id
            );
        }
        (EventType::JOYSTICK_ADDED, Event::JoyDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Joystick {} ({}) attached",
                e.which,
                or_null_opt(sdl3::joystick::joystick_name_for_id(e.which))
            );
        }
        (EventType::JOYSTICK_REMOVED, Event::JoyDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Joystick {} ({}) removed",
                e.which,
                or_null_opt(sdl3::joystick::joystick_name_for_id(e.which))
            );
        }
        (EventType::JOYSTICK_AXIS_MOTION, Event::JoyAxis(e)) => {
            sdl_log!(
                "SDL EVENT: Joystick {} axis {} value: {}",
                e.which,
                e.axis,
                e.value
            );
        }
        (EventType::JOYSTICK_BALL_MOTION, Event::JoyBall(e)) => {
            sdl_log!(
                "SDL EVENT: Joystick {}: ball {} moved by {},{}",
                e.which as i32,
                e.ball,
                e.xrel,
                e.yrel
            );
        }
        (EventType::JOYSTICK_HAT_MOTION, Event::JoyHat(e)) => {
            use sdl3::joystick::*;
            let position = match e.value {
                HAT_CENTERED => "CENTER",
                HAT_UP => "UP",
                HAT_RIGHTUP => "RIGHTUP",
                HAT_RIGHT => "RIGHT",
                HAT_RIGHTDOWN => "RIGHTDOWN",
                HAT_DOWN => "DOWN",
                HAT_LEFTDOWN => "LEFTDOWN",
                HAT_LEFT => "LEFT",
                HAT_LEFTUP => "LEFTUP",
                _ => "UNKNOWN",
            };
            sdl_log!(
                "SDL EVENT: Joystick {}: hat {} moved to {}",
                e.which,
                e.hat,
                position
            );
        }
        (EventType::JOYSTICK_BUTTON_DOWN, Event::JoyButton(e)) => {
            sdl_log!(
                "SDL EVENT: Joystick {}: button {} pressed",
                e.which,
                e.button
            );
        }
        (EventType::JOYSTICK_BUTTON_UP, Event::JoyButton(e)) => {
            sdl_log!(
                "SDL EVENT: Joystick {}: button {} released",
                e.which,
                e.button
            );
        }
        (EventType::JOYSTICK_BATTERY_UPDATED, Event::JoyBattery(e)) => {
            sdl_log!(
                "SDL EVENT: Joystick {}: battery at {} percent",
                e.which,
                e.percent
            );
        }
        (EventType::GAMEPAD_ADDED, Event::GamepadDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Gamepad {} ({}) attached",
                e.which,
                or_null_opt(sdl3::gamepad::gamepad_name_for_id(e.which))
            );
        }
        (EventType::GAMEPAD_REMOVED, Event::GamepadDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Gamepad {} ({}) removed",
                e.which,
                or_null_opt(sdl3::gamepad::gamepad_name_for_id(e.which))
            );
        }
        (EventType::GAMEPAD_REMAPPED, Event::GamepadDevice(e)) => {
            sdl_log!("SDL EVENT: Gamepad {} mapping changed", e.which);
        }
        (EventType::GAMEPAD_AXIS_MOTION, Event::GamepadAxis(e)) => {
            sdl_log!(
                "SDL EVENT: Gamepad {} axis {} ('{}') value: {}",
                e.which,
                e.axis,
                gamepad_axis_name(e.axis as i32),
                e.value
            );
        }
        (EventType::GAMEPAD_BUTTON_DOWN, Event::GamepadButton(e)) => {
            // (upstream's message has no space before "button")
            sdl_log!(
                "SDL EVENT: Gamepad {}button {} ('{}') down",
                e.which,
                e.button,
                gamepad_button_name(e.button as i32)
            );
        }
        (EventType::GAMEPAD_BUTTON_UP, Event::GamepadButton(e)) => {
            sdl_log!(
                "SDL EVENT: Gamepad {} button {} ('{}') up",
                e.which,
                e.button,
                gamepad_button_name(e.button as i32)
            );
        }
        (EventType::GAMEPAD_CAPSENSE_TOUCH, Event::GamepadCapSense(e)) => {
            // (upstream's message has no space before "capsense")
            sdl_log!(
                "SDL EVENT: Gamepad {}capsense {} ('{}') touch",
                e.which,
                e.capsense,
                cap_sense_name(e.capsense as i32)
            );
        }
        (EventType::GAMEPAD_CAPSENSE_RELEASE, Event::GamepadCapSense(e)) => {
            sdl_log!(
                "SDL EVENT: Gamepad {} capsense {} ('{}') release",
                e.which,
                e.capsense,
                cap_sense_name(e.capsense as i32)
            );
        }
        (EventType::CLIPBOARD_UPDATE, _) => {
            sdl_log!("SDL EVENT: Clipboard updated");
        }

        (EventType::FINGER_MOTION, Event::TouchFinger(e)) => {
            sdl_log!(
                "SDL EVENT: Finger: motion touch={}, finger={}, x={:.6}, y={:.6}, dx={:.6}, dy={:.6}, pressure={:.6}",
                e.touch_id,
                e.finger_id,
                e.x,
                e.y,
                e.dx,
                e.dy,
                e.pressure
            );
        }
        (
            EventType::FINGER_DOWN | EventType::FINGER_UP | EventType::FINGER_CANCELED,
            Event::TouchFinger(e),
        ) => {
            sdl_log!(
                "SDL EVENT: Finger: {} touch={}, finger={}, x={:.6}, y={:.6}, dx={:.6}, dy={:.6}, pressure={:.6}",
                if event_type == EventType::FINGER_DOWN {
                    "down"
                } else if event_type == EventType::FINGER_UP {
                    "up"
                } else {
                    "cancel"
                },
                e.touch_id,
                e.finger_id,
                e.x,
                e.y,
                e.dx,
                e.dy,
                e.pressure
            );
        }

        (EventType::PINCH_BEGIN, _) => {
            sdl_log!("SDL EVENT: Pinch Begin");
        }
        (EventType::PINCH_UPDATE, Event::Pinch(e)) => {
            sdl_log!("SDL EVENT: Pinch Update, scale={:.6}", e.scale);
        }
        (EventType::PINCH_END, _) => {
            sdl_log!("SDL EVENT: Pinch End");
        }

        (EventType::RENDER_TARGETS_RESET, Event::Render(e)) => {
            sdl_log!("SDL EVENT: render targets reset in window {}", e.window_id);
        }
        (EventType::RENDER_DEVICE_RESET, Event::Render(e)) => {
            sdl_log!("SDL EVENT: render device reset in window {}", e.window_id);
        }
        (EventType::RENDER_DEVICE_LOST, Event::Render(e)) => {
            sdl_log!("SDL EVENT: render device lost in window {}", e.window_id);
        }

        (EventType::TERMINATING, _) => {
            sdl_log!("SDL EVENT: App terminating");
        }
        (EventType::LOW_MEMORY, _) => {
            sdl_log!("SDL EVENT: App running low on memory");
        }
        (EventType::WILL_ENTER_BACKGROUND, _) => {
            sdl_log!("SDL EVENT: App will enter the background");
        }
        (EventType::DID_ENTER_BACKGROUND, _) => {
            sdl_log!("SDL EVENT: App entered the background");
        }
        (EventType::WILL_ENTER_FOREGROUND, _) => {
            sdl_log!("SDL EVENT: App will enter the foreground");
        }
        (EventType::DID_ENTER_FOREGROUND, _) => {
            sdl_log!("SDL EVENT: App entered the foreground");
        }
        (EventType::DROP_BEGIN, Event::Drop(e)) => {
            sdl_log!(
                "SDL EVENT: Drag and drop beginning in window {}",
                e.window_id
            );
        }
        (EventType::DROP_POSITION, Event::Drop(e)) => {
            sdl_log!(
                "SDL EVENT: Drag and drop moving in window {}: {},{}",
                e.window_id,
                fmt_g(e.x as f64),
                fmt_g(e.y as f64)
            );
        }
        (EventType::DROP_FILE, Event::Drop(e)) => {
            sdl_log!(
                "SDL EVENT: Drag and drop file in window {}: '{}'",
                e.window_id,
                e.data.as_deref().unwrap_or("(null)")
            );
        }
        (EventType::DROP_TEXT, Event::Drop(e)) => {
            sdl_log!(
                "SDL EVENT: Drag and drop text in window {}: '{}'",
                e.window_id,
                e.data.as_deref().unwrap_or("(null)")
            );
        }
        (EventType::DROP_COMPLETE, _) => {
            sdl_log!("SDL EVENT: Drag and drop ending");
        }
        (EventType::AUDIO_DEVICE_ADDED, Event::AudioDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Audio {} device {} available",
                if e.recording { "recording" } else { "playback" },
                e.which
            );
        }
        (EventType::AUDIO_DEVICE_REMOVED, Event::AudioDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Audio {} device {} removed",
                if e.recording { "recording" } else { "playback" },
                e.which
            );
        }
        (EventType::AUDIO_DEVICE_FORMAT_CHANGED, Event::AudioDevice(e)) => {
            sdl_log!(
                "SDL EVENT: Audio {} device {} format changed",
                if e.recording { "recording" } else { "playback" },
                e.which
            );
        }
        (EventType::CAMERA_DEVICE_ADDED, Event::CameraDevice(e)) => {
            sdl_log!("SDL EVENT: Camera device {} available", e.which);
        }
        (EventType::CAMERA_DEVICE_REMOVED, Event::CameraDevice(e)) => {
            sdl_log!("SDL EVENT: Camera device {} removed", e.which);
        }
        (EventType::CAMERA_DEVICE_APPROVED, Event::CameraDevice(e)) => {
            sdl_log!("SDL EVENT: Camera device {} permission granted", e.which);
        }
        (EventType::CAMERA_DEVICE_DENIED, Event::CameraDevice(e)) => {
            sdl_log!("SDL EVENT: Camera device {} permission denied", e.which);
        }
        (EventType::NOTIFICATION_ACTION_INVOKED, Event::Notification(e)) => {
            sdl_log!(
                "SDL EVENT: Notification action for {} button_id={}",
                e.which,
                e.action_id
            );
        }
        (EventType::SENSOR_UPDATE, Event::Sensor(e)) => {
            sdl_log!("SDL EVENT: Sensor update for {}", e.which);
        }
        (EventType::PEN_PROXIMITY_IN, Event::PenProximity(e)) => {
            sdl_log!("SDL EVENT: Pen {} entered proximity", e.which);
        }
        (EventType::PEN_PROXIMITY_OUT, Event::PenProximity(e)) => {
            sdl_log!("SDL EVENT: Pen {} left proximity", e.which);
        }
        (EventType::PEN_DOWN, Event::PenTouch(e)) => {
            sdl_log!(
                "SDL EVENT: Pen {} touched down at {},{}",
                e.which,
                fmt_g(e.x as f64),
                fmt_g(e.y as f64)
            );
        }
        (EventType::PEN_UP, Event::PenTouch(e)) => {
            sdl_log!(
                "SDL EVENT: Pen {} lifted off at {},{}",
                e.which,
                fmt_g(e.x as f64),
                fmt_g(e.y as f64)
            );
        }
        (EventType::PEN_BUTTON_DOWN, Event::PenButton(e)) => {
            sdl_log!(
                "SDL EVENT: Pen {} button {} pressed at {},{}",
                e.which,
                e.button,
                fmt_g(e.x as f64),
                fmt_g(e.y as f64)
            );
        }
        (EventType::PEN_BUTTON_UP, Event::PenButton(e)) => {
            sdl_log!(
                "SDL EVENT: Pen {} button {} released at {},{}",
                e.which,
                e.button,
                fmt_g(e.x as f64),
                fmt_g(e.y as f64)
            );
        }
        (EventType::PEN_MOTION, Event::PenMotion(e)) => {
            sdl_log!(
                "SDL EVENT: Pen {} moved to {},{}",
                e.which,
                fmt_g(e.x as f64),
                fmt_g(e.y as f64)
            );
        }
        (EventType::PEN_AXIS, Event::PenAxis(e)) => {
            sdl_log!(
                "SDL EVENT: Pen {} axis {} changed to {:.2}",
                e.which,
                e.axis as i32,
                e.value
            );
        }
        (EventType::LOCALE_CHANGED, _) => {
            sdl_log!("SDL EVENT: Locale changed");
        }
        (EventType::QUIT, _) => {
            sdl_log!("SDL EVENT: Quit requested");
        }
        (EventType::USER, Event::User(e)) => {
            sdl_log!("SDL EVENT: User event {}", e.code);
        }
        _ => {
            sdl_log!("Unknown event 0x{:04x}", event_type.0);
        }
    }
}

/// The window events of [`print_event`]; `false` for the types that
/// aren't window events (which then print as unknown).
fn print_window_event(event: &Event, e: &sdl3::events::WindowEvent) -> bool {
    let id = e.window_id;
    match e.event_type {
        EventType::WINDOW_SHOWN => sdl_log!("SDL EVENT: Window {} shown", id),
        EventType::WINDOW_HIDDEN => sdl_log!("SDL EVENT: Window {} hidden", id),
        EventType::WINDOW_EXPOSED => sdl_log!("SDL EVENT: Window {} exposed", id),
        EventType::WINDOW_MOVED => {
            sdl_log!("SDL EVENT: Window {} moved to {},{}", id, e.data1, e.data2)
        }
        EventType::WINDOW_RESIZED => {
            sdl_log!(
                "SDL EVENT: Window {} resized to {}x{}",
                id,
                e.data1,
                e.data2
            )
        }
        EventType::WINDOW_PIXEL_SIZE_CHANGED => sdl_log!(
            "SDL EVENT: Window {} changed pixel size to {}x{}",
            id,
            e.data1,
            e.data2
        ),
        EventType::WINDOW_METAL_VIEW_RESIZED => {
            sdl_log!("SDL EVENT: Window {} changed metal view size", id)
        }
        EventType::WINDOW_SAFE_AREA_CHANGED => {
            let rect = window_from_event(event)
                .and_then(|window| window.safe_area().ok())
                .unwrap_or_default();
            sdl_log!(
                "SDL EVENT: Window {} changed safe area to: {},{} {}x{}",
                id,
                rect.x,
                rect.y,
                rect.w,
                rect.h
            );
        }
        EventType::WINDOW_MINIMIZED => sdl_log!("SDL EVENT: Window {} minimized", id),
        EventType::WINDOW_MAXIMIZED => sdl_log!("SDL EVENT: Window {} maximized", id),
        EventType::WINDOW_RESTORED => sdl_log!("SDL EVENT: Window {} restored", id),
        EventType::WINDOW_MOUSE_ENTER => sdl_log!("SDL EVENT: Mouse entered window {}", id),
        EventType::WINDOW_MOUSE_LEAVE => sdl_log!("SDL EVENT: Mouse left window {}", id),
        EventType::WINDOW_FOCUS_GAINED => {
            sdl_log!("SDL EVENT: Window {} gained keyboard focus", id)
        }
        EventType::WINDOW_FOCUS_LOST => sdl_log!("SDL EVENT: Window {} lost keyboard focus", id),
        EventType::WINDOW_CLOSE_REQUESTED => sdl_log!("SDL EVENT: Window {} closed", id),
        EventType::WINDOW_HIT_TEST => sdl_log!("SDL EVENT: Window {} hit test", id),
        EventType::WINDOW_ICCPROF_CHANGED => {
            sdl_log!("SDL EVENT: Window {} ICC profile changed", id)
        }
        EventType::WINDOW_DISPLAY_CHANGED => {
            sdl_log!("SDL EVENT: Window {} display changed to {}", id, e.data1)
        }
        EventType::WINDOW_DISPLAY_SCALE_CHANGED => {
            let scale = window_from_event(event)
                .and_then(|window| window.display_scale().ok())
                .unwrap_or(0.0);
            sdl_log!(
                "SDL EVENT: Window {} display scale changed to {}%",
                id,
                (scale * 100.0) as i32
            );
        }
        EventType::WINDOW_OCCLUDED => sdl_log!("SDL EVENT: Window {} occluded", id),
        EventType::WINDOW_ENTER_FULLSCREEN => {
            sdl_log!("SDL EVENT: Window {} entered fullscreen", id)
        }
        EventType::WINDOW_LEAVE_FULLSCREEN => sdl_log!("SDL EVENT: Window {} left fullscreen", id),
        EventType::WINDOW_DESTROYED => sdl_log!("SDL EVENT: Window {} destroyed", id),
        EventType::WINDOW_HDR_STATE_CHANGED => sdl_log!(
            "SDL EVENT: Window {} HDR {}",
            id,
            if e.data1 != 0 { "enabled" } else { "disabled" }
        ),
        _ => return false,
    }
    true
}

const SCREENSHOT_FILE: &str = "screenshot.bmp";

/// The screenshot offered in the clipboard: read from the file when it's
/// first asked for. Translation of `SDLTest_ClipboardData`; dropping it is
/// `SDLTest_ScreenShotClipboardCleanup()`.
#[derive(Debug, Default)]
struct ClipboardData {
    image: Mutex<Option<Vec<u8>>>,
}

impl Drop for ClipboardData {
    fn drop(&mut self) {
        sdl_log!("Cleaning up screenshot image data");
    }
}

/// Translation of `SDLTest_ScreenShotClipboardProvider()`.
fn screen_shot_clipboard_provider(data: &ClipboardData, mime_type: &str) -> Option<Vec<u8>> {
    if mime_type.starts_with("text") {
        sdl_log!("Providing screenshot title to clipboard!");

        /* Return "Test screenshot" */
        return Some(b"Test screenshot (but this isn't part of it)"[..15].to_vec());
    }

    sdl_log!("Providing screenshot image to clipboard!");

    let mut image = data.image.lock().unwrap_or_else(|e| e.into_inner());
    if image.is_none() {
        match IoStream::from_file(SCREENSHOT_FILE, "r") {
            Ok(mut file) => {
                let length = file.size().unwrap_or(0).max(0) as usize;
                let mut buffer = vec![0u8; length];
                if file.read(&mut buffer) != length {
                    sdl_log!("Couldn't read {}: {}", SCREENSHOT_FILE, "short read");
                } else {
                    *image = Some(buffer);
                }
            }
            Err(error) => {
                sdl_log!("Couldn't load {}: {}", SCREENSHOT_FILE, error.message());
            }
        }
    }

    image.clone()
}

/// Translation of `SDLTest_CopyScreenShot()`.
fn copy_screen_shot(renderer: Option<&mut Renderer>) {
    let image_formats = ["text/plain;charset=utf-8", "image/bmp"];

    let Some(renderer) = renderer else {
        return;
    };

    let mut surface = match renderer.read_pixels(None) {
        Ok(surface) => surface,
        Err(error) => {
            sdl_log!("Couldn't read screen: {}", error.message());
            return;
        }
    };

    if let Err(error) = surface.save_bmp(SCREENSHOT_FILE) {
        sdl_log!("Couldn't save {}: {}", SCREENSHOT_FILE, error.message());
        return;
    }
    drop(surface);

    let clipboard_data = Arc::new(ClipboardData::default());
    let _ = sdl3::video::clipboard::set_clipboard_data(
        Some(Arc::new(move |mime_type: &str| {
            screen_shot_clipboard_provider(&clipboard_data, mime_type)
        })),
        &image_formats,
    );
    sdl_log!("Saved screenshot to {} and clipboard", SCREENSHOT_FILE);
}

/// Translation of `SDLTest_PasteScreenShot()`.
fn paste_screen_shot() {
    let image_formats = ["image/bmp", "image/png", "image/tiff"];

    for image_format in image_formats {
        if let Ok(Some(data)) = sdl3::video::clipboard::clipboard_data(image_format) {
            let filename = format!("clipboard.{}", &image_format[6..]);
            if let Ok(mut file) = IoStream::from_file(&filename, "w") {
                sdl_log!("Writing clipboard image to {}", filename);
                file.write(&data);
            }
            return;
        }
    }
    sdl_log!("No supported screenshot data in the clipboard");
}

/// Translation of `FullscreenTo()`.
fn fullscreen_to(state: &CommonState, index: usize, window_id: WindowID) {
    let Ok(displays) = video::displays() else {
        return;
    };
    if index < displays.len() {
        if let Ok(window) = Window::from_id(window_id) {
            let rect = video::display_bounds(displays[index]).unwrap_or_default();

            let flags = window.flags().unwrap_or_default();
            if flags.contains(WindowFlags::FULLSCREEN) {
                let _ = window.set_fullscreen(false);
                sdl3::timer::delay(Duration::from_millis(15));
            }

            let mode = window.fullscreen_mode().ok().flatten();
            if let Some(mode) = mode {
                /* Try to set the existing mode on the new display */
                let mut new_mode = mode;
                new_mode.display_id = displays[index];
                if window.set_fullscreen_mode(Some(&new_mode)).is_err() {
                    /* Try again with a default mode */
                    let include_high_density_modes =
                        state.window_flags.contains(WindowFlags::HIGH_PIXEL_DENSITY);
                    if let Ok(new_mode) = video::closest_fullscreen_display_mode(
                        displays[index],
                        state.window_w,
                        state.window_h,
                        state.refresh_rate,
                        include_high_density_modes,
                    ) {
                        let _ = window.set_fullscreen_mode(Some(&new_mode));
                    }
                }
            }
            if mode.is_none() {
                let _ = window.set_position(rect.x, rect.y);
            }
            let _ = window.set_fullscreen(true);
        }
    }
}

impl CommonState {
    /// The renderer of a window of the state's.
    fn renderer_for(&mut self, window: Window) -> Option<&mut Renderer> {
        let i = self.windows.iter().position(|w| *w == window)?;
        self.renderers.get_mut(i)?.as_mut()
    }

    /// Common event handler for test windows if you use the main callbacks
    /// ([`sdl3::app`]): logs the event (with `--info event`), handles the
    /// window events and the hotkeys every test window has, and returns
    /// what to return from the event callback ([`AppResult::Success`] for
    /// quit and Escape). This does _not_ free anything in `event`.
    /// Translation of `SDLTest_CommonEventMainCallbacks()`.
    pub fn event_main_callbacks(&mut self, event: &Event) -> AppResult {
        let event_type = event.event_type();
        if self.verbose.intersects(VerboseFlags::EVENT)
            && ((event_type != EventType::MOUSE_MOTION
                && event_type != EventType::FINGER_MOTION
                && event_type != EventType::PEN_MOTION
                && event_type != EventType::PEN_AXIS
                && event_type != EventType::PINCH_UPDATE
                && event_type != EventType::JOYSTICK_AXIS_MOTION)
                || self.verbose.intersects(VerboseFlags::MOTION))
        {
            print_event(event);
        }

        match (event_type, event) {
            (EventType::WINDOW_DISPLAY_SCALE_CHANGED, _) => {
                if self.auto_scale_content {
                    if let Some(window) = window_from_event(event) {
                        let scale = window
                            .display()
                            .and_then(video::display_content_scale)
                            .unwrap_or(0.0);
                        let w = (self.window_w as f32 * scale).ceil() as i32;
                        let h = (self.window_h as f32 * scale).ceil() as i32;
                        let _ = window.set_size(w, h);
                    }
                }
            }
            (EventType::WINDOW_FOCUS_LOST, _) => {
                if self.flash_on_focus_loss {
                    if let Some(window) = window_from_event(event) {
                        let _ = window.flash(FlashOperation::UntilFocused);
                    }
                }
            }
            (EventType::WINDOW_CLOSE_REQUESTED, _) => {
                if let Some(window) = window_from_event(event) {
                    let _ = window.hide();
                }
            }
            (EventType::KEY_DOWN, Event::Key(key)) => {
                if let Some(result) = self.key_down(event, key) {
                    return result;
                }
            }
            (EventType::QUIT, _) => {
                return AppResult::Success;
            }
            _ => {}
        }

        AppResult::Continue
    }

    /// The hotkeys of [`event_main_callbacks`](Self::event_main_callbacks):
    /// `Some` to return from it.
    fn key_down(&mut self, event: &Event, key: &sdl3::events::KeyboardEvent) -> Option<AppResult> {
        let with_control = key.modifiers.intersects(Keymod::CTRL);
        let with_shift = key.modifiers.intersects(Keymod::SHIFT);
        let with_alt = key.modifiers.intersects(Keymod::ALT);
        let window = window_from_event(event);

        match key.key {
            /* Add hotkeys here */
            Keycode::PRINTSCREEN => {
                if let Some(window) = window {
                    copy_screen_shot(self.renderer_for(window));
                }
            }
            Keycode::EQUALS => {
                if with_control {
                    /* Ctrl-+ double the size of the window */
                    if let Some(window) = window {
                        let (w, h) = window.size().unwrap_or((0, 0));
                        let _ = window.set_size(w * 2, h * 2);
                    }
                }
            }
            Keycode::MINUS => {
                if with_control {
                    /* Ctrl-- half the size of the window */
                    if let Some(window) = window {
                        let (w, h) = window.size().unwrap_or((0, 0));
                        let _ = window.set_size(w / 2, h / 2);
                    }
                }
            }
            Keycode::UP | Keycode::DOWN | Keycode::LEFT | Keycode::RIGHT => {
                if with_alt {
                    /* Alt-Up/Down/Left/Right switches between displays */
                    if let Some(window) = window {
                        if let Ok(displays) = video::displays() {
                            let display_id = window.display().unwrap_or(0);
                            let num_displays = displays.len();
                            if let Some(current_index) =
                                displays.iter().position(|&d| d == display_id)
                            {
                                let dest = if key.key == Keycode::UP || key.key == Keycode::LEFT {
                                    displays[(current_index + num_displays - 1) % num_displays]
                                } else {
                                    displays[(current_index + num_displays + 1) % num_displays]
                                };
                                sdl_log!("Centering on display ({})", dest);
                                let _ = window.set_position(
                                    video::windowpos_centered_display(dest),
                                    video::windowpos_centered_display(dest),
                                );
                            }
                        }
                    }
                }
                if with_shift {
                    /* Shift-Up/Down/Left/Right shift the window by 100px */
                    if let Some(window) = window {
                        const DELTA: i32 = 100;
                        let (mut x, mut y) = window.position().unwrap_or((0, 0));

                        if key.key == Keycode::UP {
                            y -= DELTA;
                        }
                        if key.key == Keycode::DOWN {
                            y += DELTA;
                        }
                        if key.key == Keycode::LEFT {
                            x -= DELTA;
                        }
                        if key.key == Keycode::RIGHT {
                            x += DELTA;
                        }

                        sdl_log!("Setting position to ({}, {})", x, y);
                        let _ = window.set_position(x, y);
                    }
                }
            }
            Keycode::O => {
                if with_control {
                    /* Ctrl-O (or Ctrl-Shift-O) changes window opacity. */
                    if let Some(window) = window {
                        let mut opacity = window.opacity().unwrap_or(-1.0);
                        if with_shift {
                            opacity += 0.20;
                        } else {
                            opacity -= 0.20;
                        }
                        let _ = window.set_opacity(opacity);
                    }
                }
            }
            Keycode::H => {
                if with_control {
                    /* Ctrl-H changes cursor visibility. */
                    if sdl3::events::mouse::cursor_visible() {
                        sdl3::events::mouse::hide_cursor();
                    } else {
                        sdl3::events::mouse::show_cursor();
                    }
                }
            }
            Keycode::C => {
                if with_alt {
                    /* Alt-C copy awesome text to the primary selection! */
                    let _ = sdl3::video::clipboard::set_primary_selection_text(
                        "SDL rocks!\nYou know it!",
                    );
                    sdl_log!("Copied text to primary selection");
                } else if with_control {
                    if with_shift {
                        /* Ctrl-Shift-C copy screenshot! */
                        if let Some(window) = window {
                            copy_screen_shot(self.renderer_for(window));
                        }
                    } else {
                        /* Ctrl-C copy awesome text! */
                        let _ =
                            sdl3::video::clipboard::set_clipboard_text("SDL rocks!\nYou know it!");
                        sdl_log!("Copied text to clipboard");
                    }
                }
            }
            Keycode::V => {
                if with_alt {
                    /* Alt-V paste awesome text from the primary selection! */
                    let text = sdl3::video::clipboard::primary_selection_text().unwrap_or_default();
                    if !text.is_empty() {
                        sdl_log!("Primary selection: {}", text);
                    } else {
                        sdl_log!("Primary selection is empty");
                    }
                } else if with_control {
                    if with_shift {
                        /* Ctrl-Shift-V paste screenshot! */
                        paste_screen_shot();
                    } else {
                        /* Ctrl-V paste awesome text! */
                        let text = sdl3::video::clipboard::clipboard_text().unwrap_or_default();
                        if !text.is_empty() {
                            sdl_log!("Clipboard: {}", text);
                        } else {
                            sdl_log!("Clipboard is empty");
                        }
                    }
                }
            }
            Keycode::F => {
                if with_control {
                    /* Ctrl-F flash the window */
                    if let Some(window) = window {
                        let _ = window.flash(FlashOperation::Briefly);
                    }
                }
            }
            Keycode::D => {
                if with_control {
                    /* Ctrl-D toggle fill-document */
                    if let Some(window) = window {
                        let flags = window.flags().unwrap_or_default();
                        let _ =
                            window.set_fill_document(!flags.contains(WindowFlags::FILL_DOCUMENT));
                    }
                }
            }
            Keycode::P => {
                if with_alt {
                    /* Alt-P cycle through progress states */
                    if let Some(window) = window {
                        // (SDL_PROGRESS_STATE_INVALID, -1, on failure: the
                        // next state is NONE)
                        let progress_state = match window.progress_state() {
                            Ok(ProgressState::None) => ProgressState::Indeterminate,
                            Ok(ProgressState::Indeterminate) => ProgressState::Normal,
                            Ok(ProgressState::Normal) => ProgressState::Paused,
                            Ok(ProgressState::Paused) => ProgressState::Error,
                            Ok(ProgressState::Error) | Err(_) => ProgressState::None,
                        };
                        let name = match progress_state {
                            ProgressState::None => "NONE",
                            ProgressState::Indeterminate => "INDETERMINATE",
                            ProgressState::Normal => "NORMAL",
                            ProgressState::Paused => "PAUSED",
                            ProgressState::Error => "ERROR",
                        };
                        sdl_log!("Setting progress state to {}", name);
                        let _ = window.set_progress_state(progress_state);
                    }
                } else if with_control {
                    /* Ctrl-P increase progress value */
                    if let Some(window) = window {
                        let mut progress_value = window.progress_value().unwrap_or(-1.0);
                        if with_shift {
                            progress_value -= 0.1;
                        } else {
                            progress_value += 0.1;
                        }
                        sdl_log!("Setting progress value to {:.1}", progress_value);
                        let _ = window.set_progress_value(progress_value);
                    }
                }
            }
            Keycode::G => {
                if with_control {
                    /* Ctrl-G toggle mouse grab */
                    if let Some(window) = window {
                        let grabbed = window.mouse_grab().unwrap_or(false);
                        let _ = window.set_mouse_grab(!grabbed);
                    }
                }
            }
            Keycode::K => {
                if with_control {
                    /* Ctrl-K toggle keyboard grab */
                    if let Some(window) = window {
                        let grabbed = window.keyboard_grab().unwrap_or(false);
                        let _ = window.set_keyboard_grab(!grabbed);
                    }
                }
            }
            Keycode::M => {
                if with_control {
                    /* Ctrl-M maximize */
                    if let Some(window) = window {
                        let flags = window.flags().unwrap_or_default();
                        if !flags.contains(WindowFlags::RESIZABLE) {
                            let _ = window.set_resizable(true);
                        }
                        if flags.contains(WindowFlags::MAXIMIZED) {
                            let _ = window.restore();
                        } else {
                            let _ = window.maximize();
                        }
                        if !flags.contains(WindowFlags::RESIZABLE) {
                            let _ = window.set_resizable(false);
                        }
                    }
                }
                if with_shift {
                    if let Some(window) = window {
                        let should_capture = !window
                            .flags()
                            .unwrap_or_default()
                            .contains(WindowFlags::MOUSE_CAPTURE);
                        let rc = sdl3::events::mouse::capture_mouse(should_capture).is_ok();
                        sdl_log!(
                            "{}apturing mouse {}!",
                            if should_capture { "C" } else { "Unc" },
                            if rc { "succeeded" } else { "failed" }
                        );
                    }
                }
            }
            Keycode::R => {
                if with_control {
                    /* Ctrl-R toggle mouse relative mode */
                    if let Some(window) = window {
                        let relative = window.relative_mouse_mode().unwrap_or(false);
                        let _ = window.set_relative_mouse_mode(!relative);
                    }
                }
            }
            Keycode::T => {
                if with_control {
                    /* Ctrl-T toggle topmost mode */
                    if let Some(window) = window {
                        let flags = window.flags().unwrap_or_default();
                        if flags.contains(WindowFlags::ALWAYS_ON_TOP) {
                            let _ = window.set_always_on_top(false);
                        } else {
                            let _ = window.set_always_on_top(true);
                        }
                    }
                }
            }
            Keycode::Z => {
                if with_control {
                    /* Ctrl-Z minimize */
                    if let Some(window) = window {
                        let _ = window.minimize();
                    }
                }
            }
            Keycode::RETURN => {
                if with_control {
                    /* Ctrl-Enter toggle fullscreen */
                    if let Some(window) = window {
                        let flags = window.flags().unwrap_or_default();
                        if !flags.contains(WindowFlags::FULLSCREEN)
                            || window.fullscreen_mode().ok().flatten().is_none()
                        {
                            let _ = window.set_fullscreen_mode(Some(&self.fullscreen_mode));
                            let _ = window.set_fullscreen(true);
                        } else {
                            let _ = window.set_fullscreen(false);
                        }
                    }
                } else if with_alt {
                    /* Alt-Enter toggle fullscreen desktop */
                    if let Some(window) = window {
                        let flags = window.flags().unwrap_or_default();
                        if !flags.contains(WindowFlags::FULLSCREEN)
                            || window.fullscreen_mode().ok().flatten().is_some()
                        {
                            let _ = window.set_fullscreen_mode(None);
                            let _ = window.set_fullscreen(true);
                        } else {
                            let _ = window.set_fullscreen(false);
                        }
                    }
                }
            }
            Keycode::B => {
                if with_control {
                    /* Ctrl-B toggle window border */
                    if let Some(window) = window {
                        let flags = window.flags().unwrap_or_default();
                        let b = flags.contains(WindowFlags::BORDERLESS);
                        let _ = window.set_bordered(b);
                    }
                }
            }
            Keycode::A => {
                if with_control {
                    /* Ctrl-A toggle aspect ratio */
                    if let Some(window) = window {
                        let (mut min_aspect, mut max_aspect) =
                            window.aspect_ratio().unwrap_or((0.0, 0.0));
                        if min_aspect > 0.0 || max_aspect > 0.0 {
                            min_aspect = 0.0;
                            max_aspect = 0.0;
                        } else {
                            min_aspect = 1.0;
                            max_aspect = 1.0;
                        }
                        let _ = window.set_aspect_ratio(min_aspect, max_aspect);
                    }
                }
            }
            Keycode::N0 => {
                if with_control {
                    let _ = show_simple_message_box(
                        MessageBoxFlags::INFORMATION,
                        "Test Message",
                        "You're awesome!",
                        window.as_ref(),
                    );
                }
            }
            Keycode::N1 => {
                if with_control {
                    fullscreen_to(self, 0, key.window_id);
                }
            }
            Keycode::N2 => {
                if with_control {
                    fullscreen_to(self, 1, key.window_id);
                }
            }
            Keycode::N9 => {
                if with_control {
                    sdl3::sdl_assert_always!(!"Test Assertion");
                }
            }
            Keycode::ESCAPE => return Some(AppResult::Success),
            _ => {}
        }
        None
    }

    /// Common event handler for test windows if you use a standard main
    /// loop: as [`event_main_callbacks`](Self::event_main_callbacks), with
    /// `done` set when the program should quit. Translation of
    /// `SDLTest_CommonEvent()`.
    pub fn event(&mut self, event: &Event, done: &mut bool) {
        if self.event_main_callbacks(event) != AppResult::Continue {
            *done = true;
        }
    }

    /// Close test window: destroys the targets, renderers and windows,
    /// cancels the quit timer, quits SDL and destroys the state (logging
    /// the outstanding allocations). Translation of `SDLTest_CommonQuit()`.
    pub fn quit(mut self) {
        for (i, target) in self.targets.drain(..).enumerate() {
            if let Some(target) = target {
                if let Some(Some(renderer)) = self.renderers.get_mut(i) {
                    renderer.destroy_texture(target);
                }
            }
        }
        self.renderers.clear();
        for window in self.windows.drain(..) {
            window.destroy();
        }

        if let Some(timer) = self.quit_after_ms_timer.take() {
            timer.cancel();
        }
        sdl3::init::quit();
        drop(self);
    }
}

impl Drop for CommonState {
    /// Free the common state object: logs the outstanding allocations when
    /// they are tracked. Call SDL's `quit()` before (as
    /// [`quit`](CommonState::quit) does). Translation of
    /// `SDLTest_CommonDestroyState()`.
    fn drop(&mut self) {
        // (the renderers before the windows, as the C code destroys them)
        self.renderers.clear();
        crate::memory::log_allocations();
    }
}

/// Draws various window information (position, size, etc.) to the
/// renderer, with the debug font, and returns the height used, so the
/// caller can draw more below. Translation of
/// `SDLTest_CommonDrawWindowInfo()`.
pub fn draw_window_info(renderer: &mut Renderer, window: &Window) -> f32 {
    let mut text_y = 0.0f32;
    const LINE_HEIGHT: f32 = 10.0;
    let window_display_id = window.display().unwrap_or(0);

    let mode_text = |mode: &DisplayMode| {
        format!(
            "{}x{}@{}x {}Hz, ({})",
            mode.w,
            mode.h,
            fmt_g(mode.pixel_density as f64),
            fmt_g(mode.refresh_rate as f64),
            mode.format.name()
        )
    };

    /* Video */

    renderer.set_draw_color(255, 255, 255, 255);
    let _ = draw_string(renderer, 0.0, text_y, "-- Video --");
    text_y += LINE_HEIGHT;

    renderer.set_draw_color(170, 170, 170, 255);

    let text = format!(
        "SDL_GetCurrentVideoDriver: {}",
        video::current_video_driver().unwrap_or("(null)")
    );
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    /* Renderer */

    renderer.set_draw_color(255, 255, 255, 255);
    let _ = draw_string(renderer, 0.0, text_y, "-- Renderer --");
    text_y += LINE_HEIGHT;

    renderer.set_draw_color(170, 170, 170, 255);

    let name = renderer.name();
    let text = format!("SDL_GetRendererName: {name}");
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    if let Ok((w, h)) = renderer.output_size() {
        let text = format!("SDL_GetRenderOutputSize: {w}x{h}");
        let _ = draw_string(renderer, 0.0, text_y, &text);
        text_y += LINE_HEIGHT;
    }

    {
        let (w, h) = renderer.current_output_size();
        let text = format!("SDL_GetCurrentRenderOutputSize: {w}x{h}");
        let _ = draw_string(renderer, 0.0, text_y, &text);
        text_y += LINE_HEIGHT;
    }

    let rect = renderer.viewport();
    let text = format!(
        "SDL_GetRenderViewport: {},{}, {}x{}",
        rect.x, rect.y, rect.w, rect.h
    );
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    let (scale_x, scale_y) = renderer.scale();
    let text = format!(
        "SDL_GetRenderScale: {},{}",
        fmt_g(scale_x as f64),
        fmt_g(scale_y as f64)
    );
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    let (w, h, logical_presentation) = renderer.logical_presentation();
    let mut text = format!("SDL_GetRenderLogicalPresentation: {w}x{h} ");
    print_logical_presentation(&mut text, logical_presentation);
    // (upstream doesn't draw this line, but leaves room for it)
    let _ = text;
    text_y += LINE_HEIGHT;

    /* Window */

    renderer.set_draw_color(255, 255, 255, 255);
    let _ = draw_string(renderer, 0.0, text_y, "-- Window --");
    text_y += LINE_HEIGHT;

    renderer.set_draw_color(170, 170, 170, 255);

    let (x, y) = window.position().unwrap_or((0, 0));
    let text = format!("SDL_GetWindowPosition: {x},{y}");
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    let (w, h) = window.size().unwrap_or((0, 0));
    let text = format!("SDL_GetWindowSize: {w}x{h}");
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    let rect = window.safe_area().unwrap_or_default();
    let text = format!(
        "SDL_GetWindowSafeArea: {},{} {}x{}",
        rect.x, rect.y, rect.w, rect.h
    );
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    let mut text = "SDL_GetWindowFlags: ".to_owned();
    print_window_flags(&mut text, window.flags().unwrap_or_default());
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    if let Ok(Some(mode)) = window.fullscreen_mode() {
        let text = format!("SDL_GetWindowFullscreenMode: {}", mode_text(&mode));
        let _ = draw_string(renderer, 0.0, text_y, &text);
        text_y += LINE_HEIGHT;
    }

    /* Display */

    renderer.set_draw_color(255, 255, 255, 255);
    let _ = draw_string(renderer, 0.0, text_y, "-- Display --");
    text_y += LINE_HEIGHT;

    renderer.set_draw_color(170, 170, 170, 255);

    let text = format!("SDL_GetDisplayForWindow: {window_display_id}");
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    let text = format!(
        "SDL_GetDisplayName: {}",
        or_null(video::display_name(window_display_id))
    );
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    if let Ok(rect) = video::display_bounds(window_display_id) {
        let text = format!(
            "SDL_GetDisplayBounds: {},{}, {}x{}",
            rect.x, rect.y, rect.w, rect.h
        );
        let _ = draw_string(renderer, 0.0, text_y, &text);
        text_y += LINE_HEIGHT;
    }

    if let Ok(mode) = video::current_display_mode(window_display_id) {
        let text = format!("SDL_GetCurrentDisplayMode: {}", mode_text(&mode));
        let _ = draw_string(renderer, 0.0, text_y, &text);
        text_y += LINE_HEIGHT;
    }

    if let Ok(mode) = video::desktop_display_mode(window_display_id) {
        let text = format!("SDL_GetDesktopDisplayMode: {}", mode_text(&mode));
        let _ = draw_string(renderer, 0.0, text_y, &text);
        text_y += LINE_HEIGHT;
    }

    let mut text = "SDL_GetNaturalDisplayOrientation: ".to_owned();
    print_display_orientation(
        &mut text,
        video::natural_display_orientation(window_display_id),
    );
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    let mut text = "SDL_GetCurrentDisplayOrientation: ".to_owned();
    print_display_orientation(
        &mut text,
        video::current_display_orientation(window_display_id),
    );
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    let text = format!(
        "SDL_GetDisplayContentScale: {}",
        fmt_g(video::display_content_scale(window_display_id).unwrap_or(0.0) as f64)
    );
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    /* Mouse */

    renderer.set_draw_color(255, 255, 255, 255);
    let _ = draw_string(renderer, 0.0, text_y, "-- Mouse --");
    text_y += LINE_HEIGHT;

    renderer.set_draw_color(170, 170, 170, 255);

    let (fx, fy, flags) = sdl3::events::mouse::mouse_state();
    let mut text = format!(
        "SDL_GetMouseState: {},{} ",
        fmt_g(fx as f64),
        fmt_g(fy as f64)
    );
    print_button_mask(&mut text, flags);
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    let (fx, fy, flags) = sdl3::events::mouse::global_mouse_state();
    let mut text = format!(
        "SDL_GetGlobalMouseState: {},{} ",
        fmt_g(fx as f64),
        fmt_g(fy as f64)
    );
    print_button_mask(&mut text, flags);
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    /* Keyboard */

    renderer.set_draw_color(255, 255, 255, 255);
    let _ = draw_string(renderer, 0.0, text_y, "-- Keyboard --");
    text_y += LINE_HEIGHT;

    renderer.set_draw_color(170, 170, 170, 255);

    let mut text = "SDL_GetModState: ".to_owned();
    print_mod_state(&mut text, sdl3::events::keyboard::mod_state());
    let _ = draw_string(renderer, 0.0, text_y, &text);
    text_y += LINE_HEIGHT;

    text_y
}

#[cfg(test)]
mod tests;
