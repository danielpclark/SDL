// Rust translation of src/IMG_ani.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Windows animated cursors (`.ani`, RIFF `ACON` files): the detector, and
//! the decoder and encoder of the animation API. Each frame is an icon or
//! cursor image (written as cursors).

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::properties::Properties;
use sdl3::video::Surface;

use crate::anim_decoder::{
    AnimationDecoderStatus, DecoderCore, PROP_METADATA_AUTHOR_STRING,
    PROP_METADATA_FRAME_COUNT_NUMBER, PROP_METADATA_IGNORE_PROPS_BOOLEAN,
    PROP_METADATA_TITLE_STRING,
};
use crate::anim_encoder::EncoderCore;
use crate::util::read_up_to;

/// Translation of `RIFF_FOURCC()`.
const fn riff_fourcc(c0: u8, c1: u8, c2: u8, c3: u8) -> u32 {
    c0 as u32 | (c1 as u32) << 8 | (c2 as u32) << 16 | (c3 as u32) << 24
}

const ANI_FLAG_ICON: u32 = 0x1;
const ANI_FLAG_SEQUENCE: u32 = 0x2;

/// `sizeof(ANIHEADER)`: nine `Uint32`s.
const ANIHEADER_SIZE: u32 = 36;

/// Translation of `ANIHEADER`.
#[derive(Clone, Copy, Default)]
struct AniHeader {
    cb_sizeof: u32, // sizeof(ANIHEADER) = 36 bytes.
    frames: u32,    // Number of frames in the frame list.
    steps: u32,     // Number of steps in the animation loop.
    _width: u32,    // Width
    _height: u32,   // Height
    _bpp: u32,      // bpp
    _planes: u32,   // Not used
    jif_rate: u32,  // Default display rate, in jiffies (1/60s)
    fl: u32,        // AF_ICON should be set. AF_SEQUENCE is optional
}

/// Whether `src` holds a Windows animated cursor (a RIFF `ACON` file); the
/// stream position is unchanged. Translation of `IMG_isANI()`.
pub fn is_ani(src: &mut IoStream<'_>) -> bool {
    let start = src.tell().unwrap_or(-1);
    let mut is_ani = false;
    // RIFFHEADER: riffID, cbSize, chunkID
    if let (Ok(riff_id), Ok(_cb_size), Ok(chunk_id)) =
        (src.read_u32_le(), src.read_u32_le(), src.read_u32_le())
    {
        if riff_id == riff_fourcc(b'R', b'I', b'F', b'F')
            && chunk_id == riff_fourcc(b'A', b'C', b'O', b'N')
        {
            is_ani = true;
        }
    }
    let _ = src.seek(start, IoWhence::Set);
    is_ani
}

/// Translation of `struct IMG_AnimationDecoderContext` (the ANI one).
///
/// Upstream allocates the frame offsets, durations and sequence at the
/// sizes the header gives; here the offsets grow as frames are found (the
/// missing ones being 0, as `SDL_calloc()`'s), and the durations and
/// sequence keep only what `rate` and `seq ` chunks give, the rest being
/// the header's rate and the frames in order. A corrupt header then costs
/// no memory.
pub(crate) struct AniDecoderContext {
    frame_index: u32,
    frame_count: u32,
    frame_offsets: Vec<i64>,
    frame_durations: Vec<u32>,
    frame_sequence: Vec<u32>,
    /// The header's frame count and rate (the defaults of the sequence and
    /// durations).
    frames: u32,
    jif_rate: u32,
}

impl AniDecoderContext {
    fn duration(&self, i: u32) -> u32 {
        self.frame_durations
            .get(i as usize)
            .copied()
            .unwrap_or(self.jif_rate)
    }

    fn sequence(&self, i: u32) -> u32 {
        self.frame_sequence
            .get(i as usize)
            .copied()
            .unwrap_or_else(|| i % self.frames)
    }

    /// Translation of `IMG_AnimationDecoderReset_Internal()`.
    pub(crate) fn reset(&mut self) {
        self.frame_index = 0;
    }

    /// Translation of `IMG_AnimationDecoderGetNextFrame_Internal()`.
    pub(crate) fn get_next_frame(
        &mut self,
        d: &mut DecoderCore<'_, '_>,
    ) -> Result<Option<(Surface<'static>, u64)>> {
        if self.frame_index == self.frame_count {
            d.status = AnimationDecoderStatus::Complete;
            return Ok(None);
        }

        let duration = d.decoder_duration(self.duration(self.frame_index) as u64, 60);

        let offset = self
            .frame_offsets
            .get(self.sequence(self.frame_index) as usize)
            .copied()
            .unwrap_or(0);
        if d.src().seek(offset, IoWhence::Set).is_err() {
            return Err(Error::new("Failed to seek to frame offset"));
        }
        let src = d.src();
        let frame = if crate::is_cur(src) {
            crate::load_cur_io(src)
        } else if crate::is_ico(src) {
            crate::load_ico_io(src)
        } else {
            Err(Error::new("Unrecognized frame type"))
        };
        self.frame_index += 1;

        frame.map(|frame| Some((frame, duration)))
    }
}

/// Translation of `IMG_AnimationParseContext`.
struct ParseContext {
    has_anih: bool,
    anih: AniHeader,
    author: Option<Vec<u8>>,
    title: Option<Vec<u8>>,
}

/// A C string's bytes.
fn cstr(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

/// `SDL_ReadU32LE()`, `None` for a short read.
fn read_u32(src: &mut IoStream<'_>) -> Option<u32> {
    src.read_u32_le().ok()
}

/// Translation of `ParseANIHeader()`.
fn parse_ani_header(
    src: &mut IoStream<'_>,
    parse: &mut ParseContext,
    ctx: &mut AniDecoderContext,
    size: u32,
) -> Result<()> {
    if size != ANIHEADER_SIZE {
        return Err(Error::new("Invalid ANI header"));
    }

    if parse.has_anih {
        // Ignore duplicate 'anih' chunk
        return Ok(());
    }

    let mut fields = [0u32; 9];
    for field in &mut fields {
        let Some(v) = read_u32(src) else {
            return Err(Error::new("Couldn't read ANI header"));
        };
        *field = v;
    }
    let anih = AniHeader {
        cb_sizeof: fields[0],
        frames: fields[1],
        steps: fields[2],
        _width: fields[3],
        _height: fields[4],
        _bpp: fields[5],
        _planes: fields[6],
        jif_rate: fields[7],
        fl: fields[8],
    };
    parse.anih = anih;
    parse.has_anih = true;

    if anih.cb_sizeof != ANIHEADER_SIZE || anih.frames == 0 || anih.steps == 0 {
        return Err(Error::new("Invalid ANI header"));
    }

    // We could support raw frames if we get an example of this
    if anih.fl & ANI_FLAG_ICON == 0 {
        return Err(Error::new("Raw ANI frames are unsupported"));
    }

    ctx.frame_count = anih.steps;
    // (the arrays SDL_calloc() would fail to allocate fail here too)
    let fits = |n: u32, size: usize| {
        Vec::<u8>::new()
            .try_reserve_exact(n as usize * size)
            .is_ok()
    };
    if !fits(anih.frames, 8) || !fits(ctx.frame_count, 4) || !fits(ctx.frame_count, 4) {
        return Err(Error::out_of_memory());
    }
    ctx.frame_offsets.clear();
    ctx.frame_durations.clear();
    ctx.frame_sequence.clear();
    ctx.frames = anih.frames;
    ctx.jif_rate = anih.jif_rate;
    Ok(())
}

/// Translation of `ParseInfoList()`.
fn parse_info_list(src: &mut IoStream<'_>, parse: &mut ParseContext, list_size: u32) -> Result<()> {
    let mut offset: i64 = 0;
    while offset <= list_size.wrapping_sub(8) as i64 {
        let (Some(chunk), Some(mut size)) = (read_u32(src), read_u32(src)) else {
            return Err(Error::new("Couldn't read the ANI info list"));
        };
        offset += 8;

        if (offset + size as i64) > list_size as i64 {
            return Err(Error::new("Corrupt ANI data while reading info list"));
        }

        let string = if chunk == riff_fourcc(b'I', b'N', b'A', b'M') && size > 0 {
            Some(&mut parse.title)
        } else if chunk == riff_fourcc(b'I', b'A', b'R', b'T') && size > 0 {
            Some(&mut parse.author)
        } else {
            None
        };
        if let Some(string) = string {
            let Some(mut data) = read_up_to(src, size as usize) else {
                return Err(Error::out_of_memory());
            };
            if data.is_empty() {
                return Err(Error::new("Couldn't read the ANI info list"));
            }
            // FIXME (upstream): a short read leaves the rest of the string
            // uninitialized there; it ends with what was read here.
            if data.len() == size as usize {
                data[size as usize - 1] = 0;
            }
            *string = Some(cstr(&data).to_vec());
        } else if src.seek(size as i64, IoWhence::Cur).is_err() {
            return Err(Error::new("Couldn't seek in the ANI info list"));
        }
        if size & 1 != 0 {
            size += 1;
            if src.seek(1, IoWhence::Cur).is_err() {
                return Err(Error::new("Couldn't seek in the ANI info list"));
            }
        }
        offset += size as i64;
    }
    Ok(())
}

/// Translation of `ParseFrameList()`.
fn parse_frame_list(
    src: &mut IoStream<'_>,
    parse: &mut ParseContext,
    ctx: &mut AniDecoderContext,
    list_size: u32,
) -> Result<()> {
    if !parse.has_anih {
        return Err(Error::new("Missing ANI header"));
    }
    let anih = parse.anih;

    let mut frame_count: u32 = 0;
    let mut offset: i64 = 0;
    while offset <= list_size.wrapping_sub(8) as i64 && frame_count < anih.frames {
        let (Some(chunk), Some(mut size)) = (read_u32(src), read_u32(src)) else {
            return Err(Error::new("Couldn't read the ANI frame list"));
        };
        offset += 8;

        if (offset + size as i64) > list_size as i64 {
            return Err(Error::new("Corrupt ANI data while reading frame list"));
        }

        if chunk == riff_fourcc(b'i', b'c', b'o', b'n') {
            let Ok(frame_offset) = src.tell() else {
                return Err(Error::new("Couldn't find the ANI frame"));
            };
            if frame_offset < 0 {
                return Err(Error::new("Couldn't find the ANI frame"));
            }
            let i = frame_count as usize;
            if ctx.frame_offsets.len() <= i {
                ctx.frame_offsets.resize(i + 1, 0);
            }
            ctx.frame_offsets[i] = frame_offset;
            frame_count += 1;
        }
        if src.seek(size as i64, IoWhence::Cur).is_err() {
            return Err(Error::new("Couldn't seek in the ANI frame list"));
        }
        if size & 1 != 0 {
            size += 1;
            if src.seek(1, IoWhence::Cur).is_err() {
                return Err(Error::new("Couldn't seek in the ANI frame list"));
            }
        }
        offset += size as i64;
    }
    if frame_count < anih.frames {
        return Err(Error::new(format!(
            "Missing frames, read {} of {}",
            frame_count as i32, anih.frames as i32
        )));
    }
    Ok(())
}

/// Translation of `ParseList()`.
fn parse_list(
    src: &mut IoStream<'_>,
    parse: &mut ParseContext,
    ctx: &mut AniDecoderContext,
    mut size: u32,
) -> Result<()> {
    let Some(type_) = read_u32(src) else {
        return Err(Error::new("Couldn't read the ANI list type"));
    };
    size = size.wrapping_sub(4);

    if type_ == riff_fourcc(b'I', b'N', b'F', b'O') {
        parse_info_list(src, parse, size)
    } else if type_ == riff_fourcc(b'f', b'r', b'a', b'm') {
        parse_frame_list(src, parse, ctx, size)
    } else {
        // Unknown list chunk, ignore it
        if src.seek(size as i64, IoWhence::Cur).is_err() {
            return Err(Error::new("Couldn't seek in the ANI data"));
        }
        Ok(())
    }
}

/// Translation of `ParseSequenceChunk()`.
fn parse_sequence_chunk(
    src: &mut IoStream<'_>,
    parse: &mut ParseContext,
    ctx: &mut AniDecoderContext,
    size: u32,
) -> Result<()> {
    if !parse.has_anih {
        return Err(Error::new("Missing ANI header"));
    }
    let anih = parse.anih;

    if anih.fl & ANI_FLAG_SEQUENCE == 0 {
        // The header says we don't use sequence data, ignore it
        if src.seek(size as i64, IoWhence::Cur).is_err() {
            return Err(Error::new("Couldn't seek in the ANI data"));
        }
        return Ok(());
    }

    if size as u64 != ctx.frame_count as u64 * 4 {
        return Err(Error::new("Invalid sequence chunk"));
    }
    ctx.frame_sequence.clear();
    for _ in 0..ctx.frame_count {
        let Some(step) = read_u32(src) else {
            return Err(Error::new("Couldn't read the ANI sequence"));
        };
        ctx.frame_sequence.push(step);
        if step >= anih.frames {
            return Err(Error::new("Invalid sequence chunk"));
        }
    }
    Ok(())
}

/// Translation of `ParseRateChunk()`.
fn parse_rate_chunk(
    src: &mut IoStream<'_>,
    parse: &mut ParseContext,
    ctx: &mut AniDecoderContext,
    size: u32,
) -> Result<()> {
    if !parse.has_anih {
        return Err(Error::new("Missing ANI header"));
    }

    if size as u64 != ctx.frame_count as u64 * 4 {
        return Err(Error::new("Invalid rate chunk"));
    }
    ctx.frame_durations.clear();
    for _ in 0..ctx.frame_count {
        let Some(rate) = read_u32(src) else {
            return Err(Error::new("Couldn't read the ANI rates"));
        };
        ctx.frame_durations.push(rate);
    }
    Ok(())
}

/// Create the ANI decoder of an animation decoder: parse the chunks (the
/// header, the frame list, the sequence and rates, the title and author,
/// which unless ignored go to the decoder's properties with the frame
/// count). Translation of `IMG_CreateANIAnimationDecoder()`.
pub(crate) fn create_ani_animation_decoder(
    d: &mut DecoderCore<'_, '_>,
    props: &Properties,
) -> Result<Box<AniDecoderContext>> {
    let mut ctx = Box::new(AniDecoderContext {
        frame_index: 0,
        frame_count: 0,
        frame_offsets: Vec::new(),
        frame_durations: Vec::new(),
        frame_sequence: Vec::new(),
        frames: 1,
        jif_rate: 0,
    });

    let mut parse = ParseContext {
        has_anih: false,
        anih: AniHeader::default(),
        author: None,
        title: None,
    };

    let src = d.src();
    let (Some(riff_id), Some(cb_size), Some(chunk_id)) =
        (read_u32(src), read_u32(src), read_u32(src))
    else {
        return Err(Error::new("Couldn't read RIFF header"));
    };

    if riff_id != riff_fourcc(b'R', b'I', b'F', b'F')
        || chunk_id != riff_fourcc(b'A', b'C', b'O', b'N')
    {
        return Err(Error::new("Invalid RIFF header"));
    }
    let mut offset: i64 = 4;
    let end = cb_size as i64;

    while offset < end {
        let (Some(chunk), Some(mut size)) = (read_u32(src), read_u32(src)) else {
            break;
        };
        offset += 8;

        if (offset + size as i64) > end {
            return Err(Error::new("Truncated ANI data"));
        }

        if chunk == riff_fourcc(b'a', b'n', b'i', b'h') {
            parse_ani_header(src, &mut parse, &mut ctx, size)?;
        } else if chunk == riff_fourcc(b'L', b'I', b'S', b'T') {
            parse_list(src, &mut parse, &mut ctx, size)?;
        } else if chunk == riff_fourcc(b's', b'e', b'q', b' ') {
            parse_sequence_chunk(src, &mut parse, &mut ctx, size)?;
        } else if chunk == riff_fourcc(b'r', b'a', b't', b'e') {
            parse_rate_chunk(src, &mut parse, &mut ctx, size)?;
        } else if src.seek(size as i64, IoWhence::Cur).is_err() {
            return Err(Error::new("Couldn't seek in the ANI data"));
        }
        if size & 1 != 0 {
            size = size.wrapping_add(1);
            if src.seek(1, IoWhence::Cur).is_err() {
                return Err(Error::new("Couldn't seek in the ANI data"));
            }
        }
        offset += size as i64;
    }

    // Make sure we have a valid animation
    if !parse.has_anih {
        return Err(Error::new("Incomplete ANI data"));
    }

    let ignore_props = props
        .get_bool(PROP_METADATA_IGNORE_PROPS_BOOLEAN)
        .unwrap_or(false);
    if !ignore_props {
        // Allow implicit properties to be set which are not globalized but specific to the decoder.
        let _ = d
            .props
            .set(PROP_METADATA_FRAME_COUNT_NUMBER, ctx.frame_count as i64);

        if let Some(title) = parse.title.as_deref().filter(|t| !t.is_empty()) {
            let _ = d.props.set(
                PROP_METADATA_TITLE_STRING,
                String::from_utf8_lossy(title).into_owned(),
            );
        }
        if let Some(author) = parse.author.as_deref().filter(|a| !a.is_empty()) {
            let _ = d.props.set(
                PROP_METADATA_AUTHOR_STRING,
                String::from_utf8_lossy(author).into_owned(),
            );
        }
    }

    Ok(ctx)
}

/// Translation of `struct IMG_AnimationEncoderContext` (the ANI one): the
/// frames (copies of the surfaces, where upstream keeps references) and
/// their durations, written when the encoder closes.
pub(crate) struct AniEncoderContext {
    author: Option<String>,
    title: Option<String>,
    frames: Vec<Surface<'static>>,
    durations: Vec<u64>,
}

/// A copy of a surface with its alternate images (a cursor's other sizes),
/// standing for upstream's reference to it.
fn copy_surface(surface: &Surface<'_>) -> Result<Surface<'static>> {
    // Duplicating keeps the alternate images, like the reference upstream
    // takes on the surface.
    surface.duplicate()
}

/// Translation of `SaveChunkSize()`: the size of the chunk whose size field
/// is at `offset`, from it to here (and a pad byte for an odd size).
fn save_chunk_size(dst: &mut IoStream<'_>, offset: i64) -> bool {
    let Ok(here) = dst.tell() else {
        return false;
    };
    if here < 0 {
        return false;
    }
    if dst.seek(offset, IoWhence::Set).is_err() {
        return false;
    }

    let size = (here - (offset + 4)) as u32;
    if dst.write_u32_le(size).is_err() {
        return false;
    }
    // FIXME (upstream): the result of SDL_SeekIO() is tested as a bool, so
    // only seeking to offset 0 counts as a failure.
    if dst.seek(here, IoWhence::Set).ok() == Some(0) {
        return false;
    }
    if size & 1 != 0 && dst.write_u8(0).is_err() {
        return false;
    }
    true
}

/// Translation of `WriteIconFrame()`.
fn write_icon_frame(surface: &mut Surface<'_>, dst: &mut IoStream<'_>) -> bool {
    let mut result = true;

    result &= dst
        .write_u32_le(riff_fourcc(b'i', b'c', b'o', b'n'))
        .is_ok();
    let icon_size_offset = dst.tell().unwrap_or(-1);
    result &= dst.write_u32_le(0).is_ok();
    // Technically this could be ICO format, but it's generally animated cursors
    result &= crate::save_cur_io(surface, dst).is_ok();
    result &= save_chunk_size(dst, icon_size_offset);

    result
}

/// Write an `INAM` or `IART` string, NUL included.
fn write_info_string(dst: &mut IoStream<'_>, id: u32, s: &str) -> bool {
    let mut result = true;
    let mut bytes = cstr(s.as_bytes()).to_vec();
    bytes.push(0);
    let size = bytes.len() as u32;
    result &= dst.write_u32_le(id).is_ok();
    result &= dst.write_u32_le(size).is_ok();
    result &= dst.write(&bytes) == bytes.len();
    if size & 1 != 0 {
        result &= dst.write_u8(0).is_ok();
    }
    result
}

impl AniEncoderContext {
    /// Translation of `AnimationEncoder_AddFrame()`.
    pub(crate) fn add_frame(&mut self, surface: &mut Surface<'_>, duration: u64) -> Result<()> {
        if self.frames.len() == self.frames.capacity() {
            let max_frames = self.frames.capacity() + 8;
            if self
                .frames
                .try_reserve_exact(max_frames - self.frames.len())
                .is_err()
                || self
                    .durations
                    .try_reserve_exact(max_frames - self.durations.len())
                    .is_err()
            {
                return Err(Error::out_of_memory());
            }
        }

        self.frames.push(copy_surface(surface)?);
        self.durations.push(duration);

        Ok(())
    }

    /// Translation of `WriteAnimInfo()`.
    fn write_anim_info(&self, dst: &mut IoStream<'_>) -> bool {
        let mut result = true;

        result &= dst
            .write_u32_le(riff_fourcc(b'L', b'I', b'S', b'T'))
            .is_ok();
        let list_size_offset = dst.tell().unwrap_or(-1);
        result &= dst.write_u32_le(0).is_ok();
        result &= dst
            .write_u32_le(riff_fourcc(b'I', b'N', b'F', b'O'))
            .is_ok();

        if let Some(title) = &self.title {
            result &= write_info_string(dst, riff_fourcc(b'I', b'N', b'A', b'M'), title);
        }

        if let Some(author) = &self.author {
            result &= write_info_string(dst, riff_fourcc(b'I', b'A', b'R', b'T'), author);
        }

        result &= save_chunk_size(dst, list_size_offset);

        result
    }

    /// Translation of `WriteAnimation()`.
    fn write_animation(&mut self, e: &mut EncoderCore<'_, '_>) -> bool {
        let mut result = true;
        let num_frames = self.frames.len() as u32;

        // RIFF header
        result &= e
            .dst()
            .write_u32_le(riff_fourcc(b'R', b'I', b'F', b'F'))
            .is_ok();
        let riff_size_offset = e.dst().tell().unwrap_or(-1);
        result &= e.dst().write_u32_le(0).is_ok();
        result &= e
            .dst()
            .write_u32_le(riff_fourcc(b'A', b'C', b'O', b'N'))
            .is_ok();

        // anih header chunk
        result &= e
            .dst()
            .write_u32_le(riff_fourcc(b'a', b'n', b'i', b'h'))
            .is_ok();
        result &= e.dst().write_u32_le(ANIHEADER_SIZE).is_ok();

        let mut anih = [0u8; ANIHEADER_SIZE as usize];
        let fields = [
            ANIHEADER_SIZE,
            num_frames,
            num_frames,
            0,
            0,
            0,
            0,
            1,
            ANI_FLAG_ICON,
        ];
        for (i, v) in fields.iter().enumerate() {
            anih[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        result &= e.dst().write(&anih) == anih.len();

        // Info list
        if self.author.is_some() || self.title.is_some() {
            // (its result is ignored there)
            self.write_anim_info(e.dst());
        }

        // Rate chunk
        result &= e
            .dst()
            .write_u32_le(riff_fourcc(b'r', b'a', b't', b'e'))
            .is_ok();
        result &= e.dst().write_u32_le(4 * num_frames).is_ok();
        for i in 0..self.frames.len() {
            let duration = e.encoder_duration(self.durations[i], 60) as u32;
            result &= e.dst().write_u32_le(duration).is_ok();
        }

        // Frame list
        result &= e
            .dst()
            .write_u32_le(riff_fourcc(b'L', b'I', b'S', b'T'))
            .is_ok();
        let frame_list_size_offset = e.dst().tell().unwrap_or(-1);
        result &= e.dst().write_u32_le(0).is_ok();
        result &= e
            .dst()
            .write_u32_le(riff_fourcc(b'f', b'r', b'a', b'm'))
            .is_ok();

        for frame in &mut self.frames {
            result &= write_icon_frame(frame, e.dst());
        }
        result &= save_chunk_size(e.dst(), frame_list_size_offset);

        // All done!
        result &= save_chunk_size(e.dst(), riff_size_offset);

        result
    }

    /// Translation of `AnimationEncoder_End()`.
    pub(crate) fn end(&mut self, e: &mut EncoderCore<'_, '_>) -> Result<()> {
        let mut result = true;

        if !self.frames.is_empty() {
            result = self.write_animation(e);

            self.frames.clear();
            self.durations.clear();
        }

        if result {
            Ok(())
        } else {
            Err(crate::util::write_error(e.dst()))
        }
    }
}

/// Create the ANI encoder of an animation encoder, with the title and
/// author metadata. Translation of `IMG_CreateANIAnimationEncoder()`.
pub(crate) fn create_ani_animation_encoder(
    _e: &mut EncoderCore<'_, '_>,
    props: &Properties,
) -> Result<Box<AniEncoderContext>> {
    let nonempty = |s: Option<String>| s.filter(|s| !s.is_empty());
    Ok(Box::new(AniEncoderContext {
        author: nonempty(props.get_string(PROP_METADATA_AUTHOR_STRING)),
        title: nonempty(props.get_string(PROP_METADATA_TITLE_STRING)),
        frames: Vec::new(),
        durations: Vec::new(),
    }))
}
