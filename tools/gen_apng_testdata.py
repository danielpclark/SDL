#!/usr/bin/env python3
"""Write the animated PNG test files of sdl3-image (sdl3-image/src/testdata/images/apng_*.png).

The files are built byte by byte, small (7x5 canvases, odd sizes for the
partial-byte paths), and cover what SDL_image's APNG decoder does with
them: several frames with offsets, the three dispose and two blend
operations (and unknown ones), every color type and bit depth (palette with
and without tRNS, gray, gray and alpha, RGB, RGBA, 16-bit), the five row
filters, Adam7 interlacing, loop counts, tEXt metadata, delays (with zero
denominators), a default image outside the animation, frame data split
over several chunks, fewer or more frame control chunks than acTL says,
and broken frame data. The pixels come from fixed formulas, so the output
is reproducible. The expected decodes in sdl3-image/src/testdata/reference.txt
come from upstream SDL_image's C (with libpng and zlib at its pins), not
from this script.

Usage: tools/gen_apng_testdata.py [OUTDIR]
"""

import os
import struct
import sys
import zlib

OUT = os.path.join(os.path.dirname(__file__), "..", "sdl3-image", "src", "testdata", "images")

SIG = b"\x89PNG\r\n\x1a\n"


def chunk(kind, data, crc=None):
    c = struct.pack(">I", len(data)) + kind + data
    if crc is None:
        crc = zlib.crc32(kind + data) & 0xFFFFFFFF
    return c + struct.pack(">I", crc)


def channels(ct):
    return {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ct]


def pack_row(samples, depth):
    """Pack one row of samples (ints) at `depth` bits."""
    if depth == 8:
        return bytes(samples)
    if depth == 16:
        return b"".join(struct.pack(">H", s) for s in samples)
    out = bytearray()
    per = 8 // depth
    for i in range(0, len(samples), per):
        b = 0
        for j in range(per):
            s = samples[i + j] if i + j < len(samples) else 0
            b |= s << (8 - depth * (j + 1))
        out.append(b)
    return bytes(out)


def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def filter_rows(rows, bpp, filters):
    """Filter packed rows, row i with filters[i % len(filters)]."""
    out = bytearray()
    prev = bytes(len(rows[0])) if rows else b""
    for i, row in enumerate(rows):
        f = filters[i % len(filters)]
        out.append(f)
        for x in range(len(row)):
            a = row[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 0:
                v = row[x]
            elif f == 1:
                v = row[x] - a
            elif f == 2:
                v = row[x] - b
            elif f == 3:
                v = row[x] - (a + b) // 2
            elif f == 4:
                v = row[x] - paeth(a, b, c)
            else:
                v = row[x]
            out.append(v & 0xFF)
        prev = row
    return bytes(out)


def frame_data(pixels, w, h, ct, depth, filters=(0, 1, 2, 3, 4), interlace=False):
    """The zlib stream of a frame: `pixels[y][x]` a tuple of samples."""
    bpp = max(1, channels(ct) * depth // 8)

    def rows_of(xs, ys):
        rows = []
        for y in ys:
            samples = []
            for x in xs:
                samples.extend(pixels[y][x])
            rows.append(pack_row(samples, depth))
        return rows

    if not interlace:
        raw = filter_rows(rows_of(range(w), range(h)), bpp, filters)
    else:
        raw = b""
        passes = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
        for x0, y0, dx, dy in passes:
            xs = range(x0, w, dx)
            ys = range(y0, h, dy)
            if len(xs) and len(ys):
                raw += filter_rows(rows_of(xs, ys), bpp, filters)
    return zlib.compress(raw, 9)


def ihdr(w, h, depth, ct, interlace=0):
    return chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, ct, 0, 0, interlace))


def actl(num_frames, num_plays):
    return chunk(b"acTL", struct.pack(">II", num_frames, num_plays))


def fctl(seq, w, h, x, y, num, den, dispose=0, blend=0):
    return chunk(b"fcTL", struct.pack(">IIIIIHHBB", seq, w, h, x, y, num, den, dispose, blend))


def fdat(seq, data):
    return chunk(b"fdAT", struct.pack(">I", seq) + data)


def text(key, value):
    return chunk(b"tEXt", key + b"\0" + value)


def iend():
    return chunk(b"IEND", b"")


def rgba_pixels(w, h, k, alpha=True, depth=8):
    px = []
    for y in range(h):
        row = []
        for x in range(w):
            r = (x * 37 + k * 50 + 11) & 255
            g = (y * 53 + k * 20 + 3) & 255
            b = ((x ^ y) * 29 + k * 90) & 255
            a = (255 - x * 30 - y * 20 + k * 7) & 255 if alpha else None
            s = [r, g, b] + ([a] if alpha else [])
            if depth == 16:
                s = [v * 257 ^ ((x * 7 + y * 13 + k * 5 + i) & 0xFF) for i, v in enumerate(s)]
            row.append(tuple(s))
        px.append(row)
    return px


def gray_pixels(w, h, k, depth=8, alpha=False):
    px = []
    top = (1 << depth) - 1
    for y in range(h):
        row = []
        for x in range(w):
            v = (x * 30 + y * 17 + k * 40) % (top + 1)
            s = [v]
            if alpha:
                s.append((255 - x * 25 - y * 30) & 255 if depth == 8 else (65535 - x * 2500 - y * 3000) & 0xFFFF)
            row.append(tuple(s))
        px.append(row)
    return px


def index_pixels(w, h, k, depth):
    n = 1 << depth
    return [[(((x * 3 + y * 5 + k) % n),) for x in range(w)] for y in range(h)]


def palette(n):
    return bytes(
        v for i in range(n) for v in ((i * 47 + 20) & 255, (i * 91 + 5) & 255, (255 - i * 33) & 255)
    )


def write(out, name, chunks):
    with open(os.path.join(out, name), "wb") as f:
        f.write(SIG + b"".join(chunks))


def main(out):
    os.makedirs(out, exist_ok=True)
    W, H = 7, 5

    # RGBA, 4 frames: the default image is the first frame; offsets, the
    # three dispose operations, both blend operations; a loop count and the
    # metadata (a lowercase "title" after "Title": the last one counts)
    frames = [
        (W, H, 0, 0, 10, 100, 0, 0),
        (3, 3, 2, 1, 1, 30, 1, 1),
        (4, 2, 1, 2, 250, 1000, 2, 1),
        (W, 2, 0, 3, 7, 7, 0, 1),
        (2, 2, 5, 3, 3, 20, 0, 0),
    ]
    c = [ihdr(W, H, 8, 6), actl(5, 3), text(b"Title", b"first title"), text(b"Author", b"an \"author\""),
         text(b"Description", b"line 1\nline 2\ttab"), text(b"Copyright", b"(c) nobody"),
         text(b"Creation Time", b"2025-01-02T03:04:05Z"), text(b"Comment", b"not metadata"),
         text(b"title", b"second title"), text(b"nokeyword", b"")]
    seq = 0
    for k, (w, h, x, y, num, den, dop, bop) in enumerate(frames):
        c.append(fctl(seq, w, h, x, y, num, den, dop, bop))
        seq += 1
        data = frame_data(rgba_pixels(w, h, k), w, h, 6, 8)
        if k == 0:
            c.append(chunk(b"IDAT", data))
        else:
            c.append(fdat(seq, data))
            seq += 1
    c.append(iend())
    write(out, "apng_rgba.png", c)

    # RGB (the filler), a default image outside the animation, 3 frames
    c = [ihdr(W, H, 8, 2), actl(3, 0), chunk(b"IDAT", frame_data(rgba_pixels(W, H, 9, False), W, H, 2, 8, (1,)))]
    seq = 0
    for k, (w, h, x, y, dop, bop) in enumerate([(W, H, 0, 0, 0, 0), (5, 4, 1, 1, 1, 0), (2, 3, 4, 0, 0, 1)]):
        c.append(fctl(seq, w, h, x, y, k + 1, 10, dop, bop))
        c.append(fdat(seq + 1, frame_data(rgba_pixels(w, h, k, False), w, h, 2, 8, (k % 5, 4))))
        seq += 2
    c.append(iend())
    write(out, "apng_rgb.png", c)

    # Frame data split over chunks: the default image over two IDATs (both
    # count), frame 1 over two fdATs (the second's sequence number matches no
    # frame control chunk upstream)
    d0 = frame_data(rgba_pixels(W, H, 0), W, H, 6, 8)
    d1 = frame_data(rgba_pixels(W, H, 1), W, H, 6, 8)
    c = [ihdr(W, H, 8, 6), actl(2, 2), fctl(0, W, H, 0, 0, 1, 10), chunk(b"IDAT", d0[:20]), chunk(b"IDAT", d0[20:]),
         fctl(1, W, H, 0, 0, 2, 10), fdat(2, d1[:15]), fdat(3, d1[15:]), iend()]
    write(out, "apng_split.png", c)

    # Palette with tRNS, 8 bits, offsets, blend over, dispose background
    pal = palette(16)
    trns = bytes([0, 128, 200, 255, 64, 32])
    c = [ihdr(W, H, 8, 3), actl(3, 1), chunk(b"PLTE", pal), chunk(b"tRNS", trns)]
    seq = 0
    for k, (w, h, x, y, dop, bop) in enumerate([(W, H, 0, 0, 1, 0), (4, 3, 2, 1, 1, 1), (3, 5, 4, 0, 0, 1)]):
        c.append(fctl(seq, w, h, x, y, 4, 25, dop, bop))
        d = frame_data(index_pixels(w, h, k, 4), w, h, 3, 8, (0, 2))
        c.append(chunk(b"IDAT", d) if k == 0 else fdat(seq + 1, d))
        seq += 1 if k == 0 else 2
    c.append(iend())
    write(out, "apng_pal.png", c)

    # Palette, 4 bits, no tRNS; and 1 bit (partial bytes)
    for depth, name in ((4, "apng_pal4.png"), (1, "apng_pal1.png")):
        n = 1 << depth
        c = [ihdr(W, H, depth, 3), actl(2, 0), chunk(b"PLTE", palette(n))]
        c.append(fctl(0, W, H, 0, 0, 1, 2))
        c.append(chunk(b"IDAT", frame_data(index_pixels(W, H, 0, depth), W, H, 3, depth)))
        c.append(fctl(1, 5, 3, 1, 1, 1, 2, 0, 1))
        c.append(fdat(2, frame_data(index_pixels(5, 3, 1, depth), 5, 3, 3, depth)))
        c.append(iend())
        write(out, name, c)

    # A broken tRNS CRC (an ancillary chunk: discarded with a warning)
    c = [ihdr(W, H, 8, 3), actl(1, 0), chunk(b"PLTE", palette(8)), chunk(b"tRNS", bytes([0, 100]), crc=0x12345678),
         fctl(0, W, H, 0, 0, 1, 10), chunk(b"IDAT", frame_data(index_pixels(W, H, 0, 3), W, H, 3, 8)), iend()]
    write(out, "apng_badcrc.png", c)

    # Gray (with a tRNS), gray and alpha: upstream reads them 2 bytes a pixel
    c = [ihdr(W, H, 8, 0), actl(2, 0), chunk(b"tRNS", struct.pack(">H", 40)),
         fctl(0, W, H, 0, 0, 3, 100), chunk(b"IDAT", frame_data(gray_pixels(W, H, 0), W, H, 0, 8)),
         fctl(1, 4, 4, 2, 1, 3, 100, 0, 1), fdat(2, frame_data(gray_pixels(4, 4, 1), 4, 4, 0, 8)), iend()]
    write(out, "apng_gray.png", c)
    c = [ihdr(W, H, 8, 4), actl(2, 0),
         fctl(0, W, H, 0, 0, 3, 100), chunk(b"IDAT", frame_data(gray_pixels(W, H, 0, alpha=True), W, H, 4, 8)),
         fctl(1, 3, 2, 3, 3, 3, 100, 2, 1), fdat(2, frame_data(gray_pixels(3, 2, 1, alpha=True), 3, 2, 4, 8)), iend()]
    write(out, "apng_graya.png", c)

    # Gray under 8 bits: 4 bits (the filler fails), and 2 bits 1 pixel wide
    c = [ihdr(W, H, 4, 0), actl(1, 0), fctl(0, W, H, 0, 0, 1, 10),
         chunk(b"IDAT", frame_data(gray_pixels(W, H, 0, 4), W, H, 0, 4)), iend()]
    write(out, "apng_gray4.png", c)
    c = [ihdr(1, 3, 2, 0), actl(2, 0), fctl(0, 1, 3, 0, 0, 1, 10),
         chunk(b"IDAT", frame_data(gray_pixels(1, 3, 0, 2), 1, 3, 0, 2)),
         fctl(1, 1, 2, 0, 1, 1, 10), fdat(2, frame_data(gray_pixels(1, 2, 1, 2), 1, 2, 0, 2)), iend()]
    write(out, "apng_gray2w1.png", c)

    # 16 bits: RGBA, RGB with a tRNS, gray and alpha
    c = [ihdr(W, H, 16, 6), actl(2, 1), fctl(0, W, H, 0, 0, 1, 4),
         chunk(b"IDAT", frame_data(rgba_pixels(W, H, 0, True, 16), W, H, 6, 16)),
         fctl(1, 5, 3, 1, 2, 1, 4, 0, 1), fdat(2, frame_data(rgba_pixels(5, 3, 1, True, 16), 5, 3, 6, 16)), iend()]
    write(out, "apng_rgba16.png", c)
    c = [ihdr(W, H, 16, 2), actl(2, 0), chunk(b"tRNS", struct.pack(">HHH", 1, 2, 3)), fctl(0, W, H, 0, 0, 1, 4),
         chunk(b"IDAT", frame_data(rgba_pixels(W, H, 0, False, 16), W, H, 2, 16)),
         fctl(1, 3, 3, 2, 2, 1, 4, 1, 0), fdat(2, frame_data(rgba_pixels(3, 3, 1, False, 16), 3, 3, 2, 16)), iend()]
    write(out, "apng_rgb16.png", c)
    c = [ihdr(W, H, 16, 4), actl(1, 0), fctl(0, W, H, 0, 0, 1, 4),
         chunk(b"IDAT", frame_data(gray_pixels(W, H, 0, 16, True), W, H, 4, 16)), iend()]
    write(out, "apng_graya16.png", c)

    # Adam7 interlaced frames (upstream reads them as not interlaced)
    c = [ihdr(W, H, 8, 6, 1), actl(2, 0), fctl(0, W, H, 0, 0, 1, 10),
         chunk(b"IDAT", frame_data(rgba_pixels(W, H, 0), W, H, 6, 8, interlace=True)),
         fctl(1, W, H, 0, 0, 1, 10), fdat(2, frame_data(rgba_pixels(W, H, 1), W, H, 6, 8, interlace=True)), iend()]
    write(out, "apng_interlaced.png", c)

    # Zero delay denominators (upstream divides by zero), and the same with
    # the APNG specification's 100 for them
    for name, dens in (("apng_zeroden.png", (0, 0, 10)), ("apng_den100.png", (100, 100, 10))):
        c = [ihdr(W, H, 8, 6), actl(3, 0)]
        seq = 0
        for k, (num, den) in enumerate(zip((5, 0, 3), dens)):
            c.append(fctl(seq, W, H, 0, 0, num, den))
            d = frame_data(rgba_pixels(W, H, k), W, H, 6, 8, (k,))
            c.append(chunk(b"IDAT", d) if k == 0 else fdat(seq + 1, d))
            seq += 1 if k == 0 else 2
        c.append(iend())
        write(out, name, c)

    # acTL says more frames than there are frame control chunks (not an
    # APNG to upstream), or fewer (the rest aren't decoded), with the
    # largest loop count
    c = [ihdr(W, H, 8, 6), actl(3, 0), fctl(0, W, H, 0, 0, 1, 10),
         chunk(b"IDAT", frame_data(rgba_pixels(W, H, 0), W, H, 6, 8)),
         fctl(1, W, H, 0, 0, 1, 10), fdat(2, frame_data(rgba_pixels(W, H, 1), W, H, 6, 8)), iend()]
    write(out, "apng_short.png", c)
    c = [ihdr(W, H, 8, 6), actl(1, 0xFFFFFFFF), fctl(0, W, H, 0, 0, 1, 10),
         chunk(b"IDAT", frame_data(rgba_pixels(W, H, 0), W, H, 6, 8)),
         fctl(1, W, H, 0, 0, 1, 10), fdat(2, frame_data(rgba_pixels(W, H, 1), W, H, 6, 8)),
         fctl(3, W, H, 0, 0, 1, 10), fdat(4, frame_data(rgba_pixels(W, H, 2), W, H, 6, 8)), iend()]
    write(out, "apng_fewer.png", c)

    # Unknown dispose and blend operations, and frames past the canvas
    c = [ihdr(W, H, 8, 6), actl(3, 0), fctl(0, W, H, 0, 0, 1, 10, 3, 2),
         chunk(b"IDAT", frame_data(rgba_pixels(W, H, 0), W, H, 6, 8)),
         fctl(1, 4, 4, 5, 3, 1, 10, 1, 1), fdat(2, frame_data(rgba_pixels(4, 4, 1), 4, 4, 6, 8)),
         fctl(3, 3, 2, 0x80000000, 1, 1, 10, 2, 0), fdat(4, frame_data(rgba_pixels(3, 2, 2), 3, 2, 6, 8)), iend()]
    write(out, "apng_oddops.png", c)

    # Broken frames: a bad filter byte, bad zlib data, no data
    good = frame_data(rgba_pixels(W, H, 0), W, H, 6, 8)
    badfilter = zlib.compress(b"".join(bytes([7 if y == 2 else 0]) + bytes(4 * W) for y in range(H)))
    c = [ihdr(W, H, 8, 6), actl(2, 0), fctl(0, W, H, 0, 0, 1, 10), chunk(b"IDAT", good),
         fctl(1, W, H, 0, 0, 1, 10), fdat(2, badfilter)]
    write(out, "apng_badfilter.png", c + [iend()])
    badzlib = bytearray(good)
    badzlib[2] ^= 0xFF
    c = [ihdr(W, H, 8, 6), actl(2, 0), fctl(0, W, H, 0, 0, 1, 10), chunk(b"IDAT", good),
         fctl(1, W, H, 0, 0, 1, 10), fdat(2, bytes(badzlib)), iend()]
    write(out, "apng_badzlib.png", c)
    c = [ihdr(W, H, 8, 6), actl(2, 0), fctl(0, W, H, 0, 0, 1, 10), chunk(b"IDAT", good),
         fctl(1, W, H, 0, 0, 1, 10), iend()]
    write(out, "apng_nodata.png", c)


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else OUT)
