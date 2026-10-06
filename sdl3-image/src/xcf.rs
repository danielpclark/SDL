// Rust translation of src/IMG_xcf.c from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! This is a XCF image file loading framework
//!
//! GIMP's native format: the visible layers are composited (with their
//! offsets) over the image, then its visible channels. Uncompressed and
//! RLE tiles of RGB, grayscale and indexed images, with or without alpha,
//! of file versions up to 11 (64-bit offsets) at 8 bits per channel.
//!
//! Upstream's `DEBUG` logging is left out.

// The RLE decoder's checks are kept as upstream writes them.
#![allow(clippy::if_same_then_else)]

use sdl3::error::{Error, Result};
use sdl3::io::{IoStream, IoWhence};
use sdl3::video::{PixelFormat, Rect, Surface};

use crate::util::{read_ok, read_up_to};

const MAX_XCF_SIZE: u32 = 20000; /* arbitrary limit to avoid integer overflow. */

// xcf_prop_type
const PROP_END: u32 = 0;
const PROP_COLORMAP: u32 = 1;
const PROP_SELECTION: u32 = 4;
const PROP_OPACITY: u32 = 6;
const PROP_VISIBLE: u32 = 8;
const PROP_OFFSETS: u32 = 15;
const PROP_COLOR: u32 = 16;
const PROP_COMPRESSION: u32 = 17;

// xcf_compr_type
const COMPR_NONE: u8 = 0;
const COMPR_RLE: u8 = 1;

// xcf_image_type
const IMAGE_RGB: u32 = 0;
const IMAGE_GREYSCALE: u32 = 1;
const IMAGE_INDEXED: u32 = 2;

/// `sizeof(prop->data)`: the union's largest member is the parasite
/// (two pointers and two `Uint32`s).
const PROP_DATA_SIZE: usize = 24;

/// Translation of `xcf_prop`. The `data` union is kept as its bytes (in
/// the machine's byte order, as C stores them): a property that doesn't
/// fill it leaves what the previous one put there.
struct XcfProp {
    id: u32,
    length: u32,
    data: [u8; PROP_DATA_SIZE],
    /// `data.colormap.cmap`.
    cmap: Vec<u8>,
}

impl XcfProp {
    fn new() -> XcfProp {
        // FIXME (upstream): the property is an uninitialized local there, so
        // a short COMPRESSION or COLOR property reads stack garbage; zeros
        // here.
        XcfProp {
            id: 0,
            length: 0,
            data: [0; PROP_DATA_SIZE],
            cmap: Vec::new(),
        }
    }
    fn u32_at(&self, i: usize) -> u32 {
        u32::from_ne_bytes([
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ])
    }
    fn set_u32_at(&mut self, i: usize, value: u32) {
        self.data[i..i + 4].copy_from_slice(&value.to_ne_bytes());
    }
    /// `data.colormap.num`.
    fn colormap_num(&self) -> u32 {
        self.u32_at(0)
    }
    /// `data.offset.x`, `data.offset.y`.
    fn offset(&self) -> (i32, i32) {
        (self.u32_at(0) as i32, self.u32_at(4) as i32)
    }
    /// `data.opacity`.
    fn opacity(&self) -> u32 {
        self.u32_at(0)
    }
    /// `data.visible`.
    fn visible(&self) -> u32 {
        self.u32_at(0)
    }
    /// `data.color`.
    fn color(&self) -> [u8; 3] {
        [self.data[0], self.data[1], self.data[2]]
    }
    /// `data.compression`.
    fn compression(&self) -> u8 {
        self.data[0]
    }
}

/// Translation of `xcf_header` (the parts the loader uses).
struct XcfHeader {
    _sign: [u8; 14],
    file_version: u32,
    width: u32,
    height: u32,
    image_type: u32,
    _precision: u32,

    layer_file_offsets: Vec<u64>,

    compr: u8,
    cm_num: u32,
    cm_map: Vec<u8>,
}

/// Translation of `xcf_layer`.
struct XcfLayer {
    width: u32,
    height: u32,
    _layer_type: u32,

    hierarchy_file_offset: u64,
    _layer_mask_offset: u64,

    offset_x: u32,
    offset_y: u32,
    visible: bool,
}

/// Translation of `xcf_channel`.
struct XcfChannel {
    _width: u32,
    _height: u32,

    _hierarchy_file_offset: u64,

    color: u32,
    opacity: u32,
    selection: bool,
    visible: bool,
}

/// Translation of `xcf_hierarchy`.
struct XcfHierarchy {
    width: u32,
    height: u32,
    bpp: u32,

    level_file_offsets: Vec<u64>,
}

/// Translation of `xcf_level`.
struct XcfLevel {
    width: u32,
    height: u32,

    tile_file_offsets: Vec<u64>,
}

/// The loader's state: the stream, and `SDL_GetError()`'s message (the
/// errors the decoding functions set, which the loader reports).
struct Xcf<'s, 'a> {
    src: &'s mut IoStream<'a>,
    error: String,
}

/// A tile loader: `load_xcf_tile_none()` or `load_xcf_tile_rle()`.
/// Translation of `load_tile_type`.
type LoadTile = fn(&mut Xcf<'_, '_>, usize, i32, i32, i32) -> Option<Vec<u8>>;

/* See if an image is contained in a data source */

/// Whether `src` holds a GIMP image; the stream position is unchanged.
/// Translation of `IMG_isXCF()`.
pub fn is_xcf(src: &mut IoStream<'_>) -> bool {
    let mut is_xcf = false;
    let mut magic = [0u8; 14];

    let start = src.tell().unwrap_or(-1);
    if read_ok(src, &mut magic) && &magic[..9] == b"gimp xcf " {
        is_xcf = true;
    }
    let _ = src.seek(start, IoWhence::Set);
    is_xcf
}

impl Xcf<'_, '_> {
    /// `SDL_SetError()`.
    fn set_error(&mut self, message: impl Into<String>) {
        self.error = message.into();
    }

    /// `SDL_ReadU32BE()`, `None` for a short read.
    fn read_u32_be(&mut self) -> Option<u32> {
        self.src.read_u32_be().ok()
    }

    /// Translation of `read_string()` (the string, which is only skipped).
    fn read_string(&mut self) -> Option<Vec<u8>> {
        let tmp = self.read_u32_be()?;
        if tmp == 0 {
            return Some(Vec::new());
        }
        let size = self.src.size().unwrap_or(-1);
        let tell = self.src.tell().unwrap_or(-1);
        let remaining = size - tell;
        if (tmp as i32) > 0 && (tmp as i64) <= remaining {
            let Some(mut data) = read_up_to(self.src, tmp as usize) else {
                self.set_error("Out of memory");
                return None;
            };
            if data.len() == tmp as usize {
                data[tmp as usize - 1] = 0;
                return Some(data);
            }
        }
        None
    }

    /// Translation of `read_offset()`.
    fn read_offset(&mut self, h: &XcfHeader) -> u64 {
        let mut offset: u64 = 0; /* starting with version 11, offsets are 64 bits */

        if h.file_version >= 11 {
            if let Some(offset32) = self.read_u32_be() {
                offset |= offset32 as u64;
                offset <<= 32;
            }
        }
        if let Some(offset32) = self.read_u32_be() {
            offset |= offset32 as u64;
        }
        offset
    }

    /// Translation of `xcf_read_property()` (`false` for 0).
    fn read_property(&mut self, prop: &mut XcfProp) -> bool {
        let (Some(id), Some(length)) = (self.read_u32_be(), self.read_u32_be()) else {
            return false;
        };
        prop.id = id;
        prop.length = length;

        match prop.id {
            PROP_COLORMAP => {
                let Some(num) = self.read_u32_be() else {
                    return false;
                };
                prop.set_u32_at(0, num);
                let len = num as usize * 3;
                let Some(cmap) = read_up_to(self.src, len) else {
                    self.set_error("Out of memory");
                    return false;
                };
                if cmap.len() != len {
                    return false;
                }
                prop.cmap = cmap;
            }

            PROP_OFFSETS => {
                let (Some(x), Some(y)) = (self.read_u32_be(), self.read_u32_be()) else {
                    return false;
                };
                prop.set_u32_at(0, x);
                prop.set_u32_at(4, y);
            }
            PROP_OPACITY => {
                let Some(opacity) = self.read_u32_be() else {
                    return false;
                };
                prop.set_u32_at(0, opacity);
            }
            PROP_COMPRESSION | PROP_COLOR => {
                let len = (prop.length as usize).min(PROP_DATA_SIZE);
                if !read_ok(self.src, &mut prop.data[..len]) {
                    return false;
                }
            }
            PROP_VISIBLE => {
                let Some(visible) = self.read_u32_be() else {
                    return false;
                };
                prop.set_u32_at(0, visible);
            }
            _ => {
                if self.src.seek(prop.length as i64, IoWhence::Cur).is_err() {
                    return false; /* ERROR */
                }
            }
        }
        true /* OK */
    }

    /// Translation of `read_xcf_header()`.
    fn read_header(&mut self) -> Option<XcfHeader> {
        let mut sign = [0u8; 14];
        if !read_ok(self.src, &mut sign) {
            return None;
        }
        let width = self.read_u32_be()?;
        let height = self.read_u32_be()?;
        let image_type = self.read_u32_be()?;

        if width > MAX_XCF_SIZE || height > MAX_XCF_SIZE {
            self.set_error(format!("Gimp image too large ({width}x{height})"));
            return None;
        }

        let digit = |c: u8| c.is_ascii_digit();
        let file_version =
            if sign[9] == b'v' && digit(sign[10]) && digit(sign[11]) && digit(sign[12]) {
                (sign[10] - b'0') as u32 * 100
                    + (sign[11] - b'0') as u32 * 10
                    + (sign[12] - b'0') as u32
            } else {
                0
            };
        let precision = if file_version >= 4 {
            self.read_u32_be()?
        } else {
            150
        };

        let mut h = XcfHeader {
            _sign: sign,
            file_version,
            width,
            height,
            image_type,
            _precision: precision,
            layer_file_offsets: Vec::new(),
            compr: COMPR_NONE,
            cm_num: 0,
            cm_map: Vec::new(),
        };

        /* Just read, don't save */
        let mut prop = XcfProp::new();
        loop {
            if !self.read_property(&mut prop) {
                return None;
            }
            if prop.id == PROP_COMPRESSION {
                h.compr = prop.compression();
            } else if prop.id == PROP_COLORMAP {
                h.cm_num = prop.colormap_num();
                h.cm_map = std::mem::take(&mut prop.cmap);
            }
            if prop.id == PROP_END {
                break;
            }
        }

        Some(h)
    }

    /// Translation of `read_xcf_layer()`.
    fn read_layer(&mut self, h: &XcfHeader) -> Option<XcfLayer> {
        let width = self.read_u32_be()?;
        let height = self.read_u32_be()?;
        let layer_type = self.read_u32_be()?;

        if width > MAX_XCF_SIZE || height > MAX_XCF_SIZE {
            self.set_error(format!("Gimp layer too large ({width}x{height})"));
            return None;
        }

        let _name = self.read_string();

        let mut l = XcfLayer {
            width,
            height,
            _layer_type: layer_type,
            hierarchy_file_offset: 0,
            _layer_mask_offset: 0,
            offset_x: 0,
            offset_y: 0,
            visible: false,
        };
        let mut prop = XcfProp::new();
        loop {
            if !self.read_property(&mut prop) {
                return None;
            }
            if prop.id == PROP_OFFSETS {
                let (x, y) = prop.offset();
                l.offset_x = x as u32;
                l.offset_y = y as u32;
            } else if prop.id == PROP_VISIBLE {
                l.visible = prop.visible() != 0;
            } else if prop.id == PROP_COLORMAP {
                prop.cmap = Vec::new();
            }
            if prop.id == PROP_END {
                break;
            }
        }

        l.hierarchy_file_offset = self.read_offset(h);
        l._layer_mask_offset = self.read_offset(h);

        Some(l)
    }

    /// Translation of `read_xcf_channel()`.
    fn read_channel(&mut self, h: &XcfHeader) -> Option<XcfChannel> {
        let width = self.read_u32_be()?;
        let height = self.read_u32_be()?;

        if width > MAX_XCF_SIZE || height > MAX_XCF_SIZE {
            self.set_error(format!("Gimp channel too large ({width}x{height})"));
            return None;
        }

        let _name = self.read_string();

        let mut l = XcfChannel {
            _width: width,
            _height: height,
            _hierarchy_file_offset: 0,
            color: 0,
            opacity: 0,
            selection: false,
            visible: false,
        };
        let mut prop = XcfProp::new();
        loop {
            if !self.read_property(&mut prop) {
                return None;
            }
            match prop.id {
                PROP_OPACITY => l.opacity = prop.opacity() << 24,
                PROP_COLOR => {
                    let c = prop.color();
                    l.color = ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | (c[2] as u32);
                }
                PROP_SELECTION => l.selection = true,
                PROP_VISIBLE => l.visible = prop.visible() != 0,
                _ => {}
            }
            if prop.id == PROP_END {
                break;
            }
        }

        l._hierarchy_file_offset = self.read_offset(h);

        Some(l)
    }

    /// Translation of `read_xcf_hierarchy()`.
    fn read_hierarchy(&mut self, head: &XcfHeader) -> Option<XcfHierarchy> {
        let width = self.read_u32_be()?;
        let height = self.read_u32_be()?;
        let bpp = self.read_u32_be()?;

        if width > MAX_XCF_SIZE || height > MAX_XCF_SIZE {
            self.set_error(format!("Gimp image too large ({width}x{height})"));
            return None;
        }

        let mut level_file_offsets = Vec::new();
        loop {
            let offset = self.read_offset(head);
            level_file_offsets.push(offset);
            if offset == 0 {
                break;
            }
        }

        Some(XcfHierarchy {
            width,
            height,
            bpp,
            level_file_offsets,
        })
    }

    /// Translation of `read_xcf_level()`.
    fn read_level(&mut self, h: &XcfHeader) -> Option<XcfLevel> {
        let width = self.read_u32_be()?;
        let height = self.read_u32_be()?;

        if width > MAX_XCF_SIZE || height > MAX_XCF_SIZE {
            self.set_error(format!("Gimp level too large ({width}x{height})"));
            return None;
        }

        let mut tile_file_offsets = Vec::new();
        loop {
            let offset = self.read_offset(h);
            tile_file_offsets.push(offset);
            if offset == 0 {
                break;
            }
        }

        Some(XcfLevel {
            width,
            height,
            tile_file_offsets,
        })
    }

    /// Translation of `do_layer_surface()` (`false` for 1).
    fn do_layer_surface(
        &mut self,
        surface: &mut Surface<'_>,
        head: &XcfHeader,
        layer: &XcfLayer,
        load_tile: LoadTile,
    ) -> bool {
        if let Err(e) = self
            .src
            .seek(layer.hierarchy_file_offset as i64, IoWhence::Set)
        {
            self.set_error(e.to_string());
            return false;
        }
        let Some(hierarchy) = self.read_hierarchy(head) else {
            self.set_error("Failed to read XCF image hierarchy");
            return false;
        };

        if hierarchy.bpp > 4 {
            /* unsupported. */
            self.set_error(format!("Unknown Gimp image bpp ({})", hierarchy.bpp));
            return false;
        }

        let (sw, sh, pitch) = (surface.width(), surface.height(), surface.pitch() as usize);
        let pixels: &mut [u8] = surface.pixels_mut().unwrap_or(&mut []);

        let mut i = 0;
        while hierarchy.level_file_offsets[i] != 0 {
            if self
                .src
                .seek(hierarchy.level_file_offsets[i] as i64, IoWhence::Set)
                .is_err()
            {
                break;
            }

            if i > 0 {
                /* skip level except the 1st one, just like GIMP does */
                i += 1;
                continue;
            }

            let Some(level) = self.read_level(head) else {
                return false;
            };

            let mut ty: u32 = 0;
            let mut tx: u32 = 0;
            let mut j = 0;
            while level.tile_file_offsets[j] != 0 {
                let _ = self
                    .src
                    .seek(level.tile_file_offsets[j] as i64, IoWhence::Set);
                let ox: u32 = if tx + 64 > level.width {
                    level.width % 64
                } else {
                    64
                };
                let oy: u32 = if ty + 64 > level.height {
                    level.height % 64
                } else {
                    64
                };
                // FIXME (upstream): the last tile, with no next offset to
                // measure it, is read as the largest RLE data could be (6
                // bytes a pixel); an uncompressed one near the end of the
                // file reads short and fails the load.
                let mut length: u64 = (ox * oy * 6) as u64;

                if level.tile_file_offsets[j + 1] > level.tile_file_offsets[j] {
                    length = level.tile_file_offsets[j + 1] - level.tile_file_offsets[j];
                }
                let tile = if length <= usize::MAX as u64 {
                    load_tile(
                        self,
                        length as usize,
                        hierarchy.bpp as i32,
                        ox as i32,
                        oy as i32,
                    )
                } else {
                    self.set_error("Gimp image invalid tile offsets");
                    None
                };
                let Some(tile) = tile else {
                    return false;
                };

                let mut p = 0usize;
                let mut p8 = 0usize;

                /* Bounds check: reject layer if tile data exceeds buffer */
                if ox as u64 * oy as u64 * hierarchy.bpp as u64
                    > hierarchy
                        .width
                        .wrapping_mul(hierarchy.height)
                        .wrapping_mul(hierarchy.bpp) as u64
                {
                    self.set_error("Gimp image invalid tile");
                    return false;
                }

                for y in ty..ty + oy {
                    if y >= sh as u32 || (tx + ox) > sw as u32 {
                        break;
                    }
                    let mut row = y as usize * pitch + tx as usize * 4;
                    let mut put = |row: &mut usize, value: u32| {
                        pixels[*row..*row + 4].copy_from_slice(&value.to_ne_bytes());
                        *row += 4;
                    };
                    match hierarchy.bpp {
                        4 => {
                            for _x in tx..tx + ox {
                                let v = u32::from_ne_bytes([
                                    tile[p],
                                    tile[p + 1],
                                    tile[p + 2],
                                    tile[p + 3],
                                ]);
                                p += 4;
                                put(&mut row, v.swap_bytes());
                            }
                        }
                        3 => {
                            for _x in tx..tx + ox {
                                let mut v = 0xFF000000u32;
                                v |= (tile[p8] as u32) << 16;
                                v |= (tile[p8 + 1] as u32) << 8;
                                v |= tile[p8 + 2] as u32;
                                p8 += 3;
                                put(&mut row, v);
                            }
                        }
                        2 => {
                            /* Indexed / Greyscale + Alpha */
                            match head.image_type {
                                IMAGE_INDEXED => {
                                    for _x in tx..tx + ox {
                                        let c = tile[p8];
                                        let a = tile[p8 + 1];
                                        p8 += 2;
                                        if (c as u32) < head.cm_num {
                                            let c = c as usize;
                                            put(
                                                &mut row,
                                                ((head.cm_map[c * 3] as u32) << 16)
                                                    | ((head.cm_map[c * 3 + 1] as u32) << 8)
                                                    | (head.cm_map[c * 3 + 2] as u32)
                                                    | ((a as u32) << 24),
                                            );
                                        } else {
                                            put(&mut row, 0);
                                        }
                                    }
                                }
                                IMAGE_GREYSCALE => {
                                    for _x in tx..tx + ox {
                                        let c = tile[p8] as u32;
                                        let a = tile[p8 + 1] as u32;
                                        p8 += 2;
                                        put(&mut row, (c << 16) | (c << 8) | c | (a << 24));
                                    }
                                }
                                _ => {
                                    self.set_error(format!(
                                        "Unknown Gimp image type ({})",
                                        head.image_type
                                    ));
                                    return false;
                                }
                            }
                        }
                        1 => {
                            /* Indexed / Greyscale */
                            match head.image_type {
                                IMAGE_INDEXED => {
                                    for _x in tx..tx + ox {
                                        let c = tile[p8];
                                        p8 += 1;
                                        if (c as u32) < head.cm_num {
                                            let c = c as usize;
                                            put(
                                                &mut row,
                                                0xFF000000
                                                    | ((head.cm_map[c * 3] as u32) << 16)
                                                    | ((head.cm_map[c * 3 + 1] as u32) << 8)
                                                    | (head.cm_map[c * 3 + 2] as u32),
                                            );
                                        } else {
                                            put(&mut row, 0);
                                        }
                                    }
                                }
                                IMAGE_GREYSCALE => {
                                    for _x in tx..tx + ox {
                                        let c = tile[p8] as u32;
                                        p8 += 1;
                                        put(&mut row, 0xFF000000 | (c << 16) | (c << 8) | c);
                                    }
                                }
                                _ => {
                                    self.set_error(format!(
                                        "Unknown Gimp image type ({})\n",
                                        head.image_type
                                    ));
                                    return false;
                                }
                            }
                        }
                        _ => {}
                    }
                }

                tx += 64;
                if tx >= level.width {
                    tx = 0;
                    ty += 64;
                }
                if ty >= level.height {
                    break;
                }
                j += 1;
            }
            i += 1;
        }

        true
    }
}

/// Translation of `load_xcf_tile_none()`.
fn load_xcf_tile_none(
    xcf: &mut Xcf<'_, '_>,
    len: usize,
    bpp: i32,
    x: i32,
    y: i32,
) -> Option<Vec<u8>> {
    if len < (x * y * bpp) as usize {
        xcf.set_error("Gimp image invalid tile offsets");
        return None;
    }

    let Some(load) = read_up_to(xcf.src, len) else {
        xcf.set_error("Out of memory");
        return None;
    };
    if load.len() != len {
        return None;
    }
    Some(load)
}

/// Translation of `load_xcf_tile_rle()`.
fn load_xcf_tile_rle(
    xcf: &mut Xcf<'_, '_>,
    len: usize,
    bpp: i32,
    x: i32,
    y: i32,
) -> Option<Vec<u8>> {
    if len == 0 {
        /* probably bogus data. */
        return None;
    }

    // (`SDL_calloc(1, len)` and a read of what there is: the bytes past
    // what was read are zero)
    let Some(load) = read_up_to(xcf.src, len) else {
        xcf.set_error("Out of memory");
        return None;
    };
    let amount_read = load.len();
    if amount_read == 0 {
        return None;
    }
    // FIXME (upstream): the run headers are read without checking for the
    // end of the buffer, up to 3 bytes past it; those read as 0 here.
    let at = |t: usize| load.get(t).copied().unwrap_or(0);
    let mut t = 0usize;

    let data_len = (x * y * bpp) as usize;
    let mut data = vec![0u8; data_len];
    let data_end = data_len;
    for i in 0..bpp as usize {
        let mut d = i;
        let mut size: i32 = x * y;

        while size > 0 {
            let val = at(t);
            t += 1;

            let mut length = val as i32;
            if length >= 128 {
                length = 255 - (length - 1);
                if length == 128 {
                    length = ((at(t) as i32) << 8) + at(t + 1) as i32;
                    t += 2;
                }

                if (t + length as usize) >= amount_read {
                    break; /* bogus data */
                } else if length > size {
                    break; /* bogus data */
                }

                size -= length;

                while length > 0 {
                    length -= 1;
                    if d >= data_end {
                        break;
                    }
                    data[d] = at(t);
                    t += 1;
                    d += bpp as usize;
                }
            } else {
                length += 1;
                if length == 128 {
                    length = ((at(t) as i32) << 8) + at(t + 1) as i32;
                    t += 2;
                }

                if t >= amount_read {
                    break; /* bogus data */
                } else if length > size {
                    break; /* bogus data */
                }

                size -= length;

                let val = at(t);
                t += 1;

                for _j in 0..length {
                    if d >= data_end {
                        break;
                    }
                    data[d] = val;
                    d += bpp as usize;
                }
            }
        }

        if size > 0 {
            break; /* just drop out, untouched data initialized to zero. */
        }
    }

    Some(data)
}

/// Translation of `rgb2grey()`.
fn rgb2grey(a: u32) -> u32 {
    let l = (0.2990 * ((a & 0x00FF0000) >> 16) as f64
        + 0.5870 * ((a & 0x0000FF00) >> 8) as f64
        + 0.1140 * (a & 0x000000FF) as f64) as u8 as u32;
    (l << 16) | (l << 8) | l
}

/// Translation of `create_channel_surface()`.
fn create_channel_surface(surf: &mut Surface<'_>, itype: u32, color: u32, opacity: u32) {
    let c = match itype {
        IMAGE_RGB | IMAGE_INDEXED => opacity | color,
        IMAGE_GREYSCALE => opacity | rgb2grey(color),
        _ => 0,
    };
    let _ = surf.fill_rect(None, c);
}

/// Load a GIMP image: its visible layers and channels composited, as an
/// ARGB8888 surface. Translation of `IMG_LoadXCF_IO()`.
pub fn load_xcf_io(src: &mut IoStream<'_>) -> Result<Surface<'static>> {
    let start = src.tell().unwrap_or(-1);
    let mut xcf = Xcf {
        src,
        error: String::new(),
    };
    let result = load_xcf(&mut xcf);
    result.map_err(|error| {
        let _ = xcf.src.seek(start, IoWhence::Set);
        Error::new(error)
    })
}

fn load_xcf(xcf: &mut Xcf<'_, '_>) -> std::result::Result<Surface<'static>, String> {
    let Some(mut head) = xcf.read_header() else {
        return Err("Couldn't read header".into());
    };

    let load_tile: LoadTile = match head.compr {
        COMPR_NONE => load_xcf_tile_none,
        COMPR_RLE => load_xcf_tile_rle,
        _ => return Err("Unsupported compression".into()),
    };

    /* Create the surface of the appropriate type */
    let Ok(mut surface) =
        Surface::new(head.width as i32, head.height as i32, PixelFormat::ARGB8888)
    else {
        return Err("Out of memory".into());
    };

    loop {
        let offset = xcf.read_offset(&head) as i64;
        if offset == 0 {
            break;
        }
        head.layer_file_offsets.push(offset as u64);
    }
    let fp = xcf.src.tell().unwrap_or(-1);

    let Ok(mut lays) = Surface::new(head.width as i32, head.height as i32, PixelFormat::ARGB8888)
    else {
        return Err("Out of memory".into());
    };

    /* Blit layers backwards, because Gimp saves them highest first */
    for i in (1..=head.layer_file_offsets.len()).rev() {
        let _ = xcf
            .src
            .seek(head.layer_file_offsets[i - 1] as i64, IoWhence::Set);

        if let Some(layer) = xcf.read_layer(&head) {
            if layer.visible {
                if !xcf.do_layer_surface(&mut lays, &head, &layer, load_tile) {
                    return Err(xcf.error.clone());
                }
                let rs = Rect::new(0, 0, layer.width as i32, layer.height as i32);
                let rd = Rect::new(
                    layer.offset_x as i32,
                    layer.offset_y as i32,
                    layer.width as i32,
                    layer.height as i32,
                );

                let _ = lays.blit(Some(&rs), &mut surface, Some(&rd));
            }
        }
    }
    drop(lays);

    let _ = xcf.src.seek(fp, IoWhence::Set);

    /* read channels */
    let mut channel = Vec::new();
    loop {
        let offset = xcf.read_offset(&head) as i64;
        if offset == 0 {
            break;
        }
        let fp = xcf.src.tell().unwrap_or(-1);
        if xcf.src.seek(offset, IoWhence::Set).is_err() {
            return Err("invalid channel offset".into());
        }
        if let Some(c) = xcf.read_channel(&head) {
            channel.push(c);
        }
        let _ = xcf.src.seek(fp, IoWhence::Set);
    }

    if !channel.is_empty() {
        let Ok(mut chs) =
            Surface::new(head.width as i32, head.height as i32, PixelFormat::ARGB8888)
        else {
            return Err("Out of memory".into());
        };
        for c in &channel {
            if !c.selection && c.visible {
                create_channel_surface(&mut chs, head.image_type, c.color, c.opacity);
                let _ = chs.blit(None, &mut surface, None);
            }
        }
    }

    Ok(surface)
}
