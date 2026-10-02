// Rust translation of src/video/SDL_blit_copy.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

use super::BlitInfo;

/// Row-by-row copy for identical formats. Translation of `SDL_BlitCopy()`.
///
/// Upstream also handles overlapping source and destination (a surface
/// blitted onto itself) with `memmove`, and uses SSE streaming stores for
/// 16-byte aligned rows. Source and destination are distinct buffers here,
/// and the streaming stores produce the same bytes as a plain copy.
pub(crate) fn blit_copy(info: &mut BlitInfo<'_>) {
    let w = (info.dst_w * info.dst_fmt.bytes_per_pixel as i32) as usize;
    let h = info.dst_h as usize;
    let srcskip = info.src_pitch as usize;
    let dstskip = info.dst_pitch as usize;

    let (mut src, mut dst) = (0usize, 0usize);
    for _ in 0..h {
        info.dst[dst..dst + w].copy_from_slice(&info.src[src..src + w]);
        src += srcskip;
        dst += dstskip;
    }
}
