// Rust translation of src/IMG_anim_encoder.c and src/IMG_anim_encoder.h
// from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Animation encoders: frames one at a time to a file or a stream, by
//! format (GIF and ANI here; APNG, AVIF and WebP need their libraries,
//! which this crate doesn't have, and report so as an upstream build
//! without them), and whole [`Animation`](crate::Animation)s.

use std::path::Path;

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::stdlib::string::strcasecmp;
use sdl3::video::Surface;

use crate::ani::{create_ani_animation_encoder, AniEncoderContext};
use crate::anim_decoder::Stream;
use crate::gif::{create_gif_animation_encoder, GifEncoderContext};
use crate::img::{timebase_duration, Animation};
use crate::webp::{create_webp_animation_encoder, WebpEncoderContext};

/// The file to write (a string). Translation of
/// `IMG_PROP_ANIMATION_ENCODER_CREATE_FILENAME_STRING`.
pub const PROP_ANIMATION_ENCODER_CREATE_FILENAME_STRING: &str =
    "SDL_image.animation_encoder.create.filename";
/// Translation of `IMG_PROP_ANIMATION_ENCODER_CREATE_IOSTREAM_POINTER`
/// (here the stream is an argument of [`AnimationEncoder::with_properties`]).
pub const PROP_ANIMATION_ENCODER_CREATE_IOSTREAM_POINTER: &str =
    "SDL_image.animation_encoder.create.iostream";
/// Translation of
/// `IMG_PROP_ANIMATION_ENCODER_CREATE_IOSTREAM_AUTOCLOSE_BOOLEAN` (here the
/// caller owns the stream).
pub const PROP_ANIMATION_ENCODER_CREATE_IOSTREAM_AUTOCLOSE_BOOLEAN: &str =
    "SDL_image.animation_encoder.create.iostream.autoclose";
/// The format, a file extension such as `"gif"` (a string). Translation of
/// `IMG_PROP_ANIMATION_ENCODER_CREATE_TYPE_STRING`.
pub const PROP_ANIMATION_ENCODER_CREATE_TYPE_STRING: &str =
    "SDL_image.animation_encoder.create.type";
/// The quality, 0 to 100 (a number). Translation of
/// `IMG_PROP_ANIMATION_ENCODER_CREATE_QUALITY_NUMBER`.
pub const PROP_ANIMATION_ENCODER_CREATE_QUALITY_NUMBER: &str =
    "SDL_image.animation_encoder.create.quality";
/// The numerator of the time base of the durations (a number, default 1).
/// Translation of `IMG_PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_NUMERATOR_NUMBER`.
pub const PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_NUMERATOR_NUMBER: &str =
    "SDL_image.animation_encoder.create.timebase.numerator";
/// The denominator of the time base (a number, default 1000). Translation
/// of `IMG_PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER`.
pub const PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER: &str =
    "SDL_image.animation_encoder.create.timebase.denominator";
/// Translation of `IMG_PROP_ANIMATION_ENCODER_CREATE_AVIF_MAX_THREADS_NUMBER`
/// (for the AVIF encoder, not in this crate).
pub const PROP_ANIMATION_ENCODER_CREATE_AVIF_MAX_THREADS_NUMBER: &str =
    "SDL_image.animation_encoder.create.avif.max_threads";
/// Translation of `IMG_PROP_ANIMATION_ENCODER_CREATE_AVIF_KEYFRAME_INTERVAL_NUMBER`
/// (for the AVIF encoder, not in this crate).
pub const PROP_ANIMATION_ENCODER_CREATE_AVIF_KEYFRAME_INTERVAL_NUMBER: &str =
    "SDL_image.animation_encoder.create.avif.keyframe_interval";
/// Whether the GIF encoder maps the frames after the first to the first
/// frame's palette (a boolean). Translation of
/// `IMG_PROP_ANIMATION_ENCODER_CREATE_GIF_USE_LUT_BOOLEAN`.
pub const PROP_ANIMATION_ENCODER_CREATE_GIF_USE_LUT_BOOLEAN: &str =
    "SDL_image.animation_encoder.create.gif.use_lut";

/// The part of `IMG_AnimationEncoder` the format encoders share: the
/// stream, its start, the quality, time base and accumulated presentation
/// time.
pub(crate) struct EncoderCore<'s, 'a> {
    pub(crate) dst: Stream<'s, 'a>,
    pub(crate) start: i64,
    pub(crate) quality: i32,
    pub(crate) timebase_numerator: i32,
    pub(crate) timebase_denominator: i32,
    pub(crate) accumulated_pts: u64,
}

impl<'a> EncoderCore<'_, 'a> {
    /// The stream.
    pub(crate) fn dst(&mut self) -> &mut IoStream<'a> {
        self.dst.io()
    }

    /// A duration in the encoder's time base in `1 / timebase_denominator`
    /// seconds, advancing its presentation time. Translation of
    /// `IMG_GetEncoderDuration()`.
    pub(crate) fn encoder_duration(&mut self, duration: u64, timebase_denominator: u64) -> u64 {
        let value = timebase_duration(
            self.accumulated_pts,
            duration,
            self.timebase_numerator as u64,
            self.timebase_denominator as u64,
            1,
            timebase_denominator,
        );
        self.accumulated_pts = self.accumulated_pts.wrapping_add(duration);
        value
    }
}

/// Translation of `struct IMG_AnimationEncoderContext`, one per encoder.
pub(crate) enum EncoderContext {
    None,
    Gif(Box<GifEncoderContext>),
    Ani(Box<AniEncoderContext>),
    Webp(Box<WebpEncoderContext>),
}

/// An encoder of the frames of an animation, one at a time. Translation of
/// `IMG_AnimationEncoder`. Dropping it closes it (see
/// [`close`](Self::close)), ignoring errors.
pub struct AnimationEncoder<'s, 'a> {
    core: EncoderCore<'s, 'a>,
    ctx: EncoderContext,
}

impl std::fmt::Debug for AnimationEncoder<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format = match &self.ctx {
            EncoderContext::None => "closed",
            EncoderContext::Gif(_) => "gif",
            EncoderContext::Ani(_) => "ani",
            EncoderContext::Webp(_) => "webp",
        };
        f.debug_struct("AnimationEncoder")
            .field("format", &format)
            .field("quality", &self.core.quality)
            .finish_non_exhaustive()
    }
}

impl AnimationEncoder<'static, 'static> {
    /// Create an encoder writing an animation file, of the format its
    /// extension names. Translation of `IMG_CreateAnimationEncoder()`.
    pub fn new(file: impl AsRef<Path>) -> Result<AnimationEncoder<'static, 'static>> {
        let file = file.as_ref();
        if file.as_os_str().is_empty() {
            return Err(Error::invalid_param("file"));
        }

        let props = Properties::new();
        props.set(
            PROP_ANIMATION_ENCODER_CREATE_FILENAME_STRING,
            file.to_string_lossy().into_owned(),
        )?;
        AnimationEncoder::with_properties(None, &props)
    }
}

impl<'s, 'a> AnimationEncoder<'s, 'a> {
    /// Create an encoder writing an animation to `dst` (from its position),
    /// of the format `type_` names (`"gif"`, `"ani"`, ...). Translation of
    /// `IMG_CreateAnimationEncoder_IO()`.
    pub fn from_io(dst: &'s mut IoStream<'a>, type_: &str) -> Result<AnimationEncoder<'s, 'a>> {
        if type_.is_empty() {
            return Err(Error::invalid_param("type"));
        }

        let props = Properties::new();
        props.set(PROP_ANIMATION_ENCODER_CREATE_TYPE_STRING, type_)?;
        AnimationEncoder::with_properties(Some(dst), &props)
    }

    /// Create an encoder with properties: `dst`, or the file named by
    /// [`PROP_ANIMATION_ENCODER_CREATE_FILENAME_STRING`]; the type (else the
    /// file's extension); the quality and time base; metadata; and the
    /// format encoders' own properties. Translation of
    /// `IMG_CreateAnimationEncoderWithProperties()` (the stream, a pointer
    /// property there, is an argument here).
    pub fn with_properties(
        dst: Option<&'s mut IoStream<'a>>,
        props: &Properties,
    ) -> Result<AnimationEncoder<'s, 'a>> {
        let file = props.get_string(PROP_ANIMATION_ENCODER_CREATE_FILENAME_STRING);
        let mut type_ = props.get_string(PROP_ANIMATION_ENCODER_CREATE_TYPE_STRING);
        let timebase_numerator = props
            .get_number(PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_NUMERATOR_NUMBER)
            .unwrap_or(1) as i32;
        let timebase_denominator = props
            .get_number(PROP_ANIMATION_ENCODER_CREATE_TIMEBASE_DENOMINATOR_NUMBER)
            .unwrap_or(1000) as i32;

        if type_.as_deref().is_none_or(str::is_empty) {
            // Skip the '.' in the file extension
            type_ = file
                .as_deref()
                .and_then(|f| f.rfind('.').map(|i| f[i + 1..].to_owned()));
            if type_.as_deref().is_none_or(str::is_empty) {
                return Err(Error::new("Couldn't determine file type"));
            }
        }
        let type_ = type_.unwrap_or_default();

        if timebase_numerator <= 0 {
            return Err(Error::new("Time base numerator must be > 0"));
        }

        if timebase_denominator <= 0 {
            return Err(Error::new("Time base denominator must be > 0"));
        }

        let dst = match dst {
            Some(dst) => Stream::Borrowed(dst),
            None => {
                let Some(file) = &file else {
                    return Err(Error::new("No output properties set"));
                };
                Stream::Owned(IoStream::from_file(file, "wb")?)
            }
        };

        let mut encoder = AnimationEncoder {
            core: EncoderCore {
                dst,
                start: 0,
                quality: props
                    .get_number(PROP_ANIMATION_ENCODER_CREATE_QUALITY_NUMBER)
                    .unwrap_or(-1) as i32,
                timebase_numerator,
                timebase_denominator,
                accumulated_pts: 0,
            },
            ctx: EncoderContext::None,
        };
        encoder.core.start = encoder.core.dst().tell().unwrap_or(-1);

        let is = |name: &str| strcasecmp(&type_, name).is_eq();
        let e = &mut encoder.core;
        let result = if is("ani") {
            create_ani_animation_encoder(e, props).map(EncoderContext::Ani)
        } else if is("apng") || is("png") {
            Err(Error::new("SDL_image not built against libpng."))
        } else if is("avifs") || is("avif") {
            Err(Error::new(
                "SDL_image built without AVIF animation save support",
            ))
        } else if is("gif") {
            create_gif_animation_encoder(e, props).map(EncoderContext::Gif)
        } else if is("webp") {
            create_webp_animation_encoder(e, props).map(EncoderContext::Webp)
        } else {
            Err(Error::new("Unrecognized output type"))
        };

        match result {
            Ok(ctx) => {
                encoder.ctx = ctx;
                Ok(encoder)
            }
            Err(e) => {
                // (a stream it opened closes as it is dropped)
                if let Stream::Borrowed(dst) = &mut encoder.core.dst {
                    if encoder.core.start >= 0 {
                        let _ = dst.seek(encoder.core.start, IoWhence::Set);
                    }
                }
                Err(e)
            }
        }
    }

    /// Add a frame, shown for `duration` (in the encoder's time base).
    /// Translation of `IMG_AddAnimationEncoderFrame()`.
    pub fn add_frame(&mut self, surface: &mut Surface<'_>, duration: u64) -> Result<()> {
        if surface.width() <= 0 || surface.height() <= 0 {
            return Err(Error::invalid_param("surface"));
        }

        let e = &mut self.core;
        match &mut self.ctx {
            EncoderContext::None => Err(Error::invalid_param("encoder")),
            EncoderContext::Gif(ctx) => ctx.add_frame(e, surface, duration),
            EncoderContext::Ani(ctx) => ctx.add_frame(surface, duration),
            EncoderContext::Webp(ctx) => ctx.add_frame(e, surface, duration),
        }
    }

    /// Finish the animation (writing what the format writes at the end)
    /// and close the encoder, and the stream it opened, if any.
    /// Translation of `IMG_CloseAnimationEncoder()`.
    pub fn close(mut self) -> Result<()> {
        self.close_internal()
    }

    fn close_internal(&mut self) -> Result<()> {
        let e = &mut self.core;
        let result = match std::mem::replace(&mut self.ctx, EncoderContext::None) {
            EncoderContext::None => return Ok(()),
            EncoderContext::Gif(mut ctx) => ctx.end(e),
            EncoderContext::Ani(mut ctx) => ctx.end(e),
            EncoderContext::Webp(mut ctx) => ctx.end(e),
        };
        let dst = std::mem::replace(
            &mut self.core.dst,
            Stream::Owned(IoStream::from_const_mem(&[])),
        );
        let closed = match dst {
            Stream::Owned(io) => io.close(),
            Stream::Borrowed(_) => Ok(()),
        };
        result.and(closed)
    }
}

impl Drop for AnimationEncoder<'_, '_> {
    fn drop(&mut self) {
        let _ = self.close_internal();
    }
}

/// Translation of `HasMetadataCallback()` and `IMG_HasMetadata()`: whether
/// a property group has any `SDL_image.metadata.` property.
pub(crate) fn has_metadata(props: &Properties) -> bool {
    props
        .names()
        .iter()
        .any(|name| name.starts_with("SDL_image.metadata."))
}

/// Translation of `IMG_EncodeAnimation()`.
fn encode_animation(
    anim: &mut Animation,
    dst: &mut IoStream<'_>,
    type_: &str,
    quality: i32,
) -> Result<()> {
    if anim.frames.is_empty() || anim.delays.len() < anim.frames.len() {
        return Err(Error::invalid_param("anim"));
    }

    let props = Properties::new();
    props.set(PROP_ANIMATION_ENCODER_CREATE_TYPE_STRING, type_)?;
    props.set(PROP_ANIMATION_ENCODER_CREATE_QUALITY_NUMBER, quality as i64)?;
    let mut encoder = AnimationEncoder::with_properties(Some(dst), &props)?;

    let mut result = Ok(());
    for (frame, &delay) in anim.frames.iter_mut().zip(&anim.delays) {
        // (the delay, an int, converted to Uint64)
        if let Err(e) = encoder.add_frame(frame, delay as i64 as u64) {
            result = Err(e);
            break;
        }
    }

    let closed = encoder.close();
    result.and(closed)
}

/// Save an animation as a Windows animated cursor (each frame a cursor).
/// Translation of `IMG_SaveANIAnimation_IO()`.
pub fn save_ani_animation_io(anim: &mut Animation, dst: &mut IoStream<'_>) -> Result<()> {
    encode_animation(anim, dst, "ani", -1)
}

/// Save an animation as an animated PNG: not without libpng, as upstream
/// built without it. Translation of `IMG_SaveAPNGAnimation_IO()`.
pub fn save_apng_animation_io(anim: &mut Animation, dst: &mut IoStream<'_>) -> Result<()> {
    encode_animation(anim, dst, "png", -1)
}

/// Save an animation as an AVIF image sequence: not without libavif, as
/// upstream built without it. Translation of `IMG_SaveAVIFAnimation_IO()`.
pub fn save_avif_animation_io(
    anim: &mut Animation,
    dst: &mut IoStream<'_>,
    quality: i32,
) -> Result<()> {
    encode_animation(anim, dst, "avifs", quality)
}

/// Save an animation as a GIF. Translation of `IMG_SaveGIFAnimation_IO()`.
pub fn save_gif_animation_io(anim: &mut Animation, dst: &mut IoStream<'_>) -> Result<()> {
    encode_animation(anim, dst, "gif", -1)
}

/// Save an animation as a WebP animation: not without libwebp, as
/// upstream built without it. Translation of `IMG_SaveWEBPAnimation_IO()`.
pub fn save_webp_animation_io(
    anim: &mut Animation,
    dst: &mut IoStream<'_>,
    quality: i32,
) -> Result<()> {
    encode_animation(anim, dst, "webp", quality)
}
