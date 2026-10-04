#!/usr/bin/env python3
"""Write the PNG files the image codec tests decode (sdl3/src/video/testdata/images/).

Each file exercises a decoder path: every color type and bit depth, the five
row filters, Adam7 interlacing, tRNS transparency, stored/fixed/dynamic deflate
blocks, split IDAT chunks, ancillary chunks to skip, edge sizes, and a few
corrupt files. The pixels come from a fixed LCG, so the output is
reproducible. The expected decodes come from upstream SDL's C, not from here.
"""

import os
import struct
import zlib

OUT = os.path.join(os.path.dirname(__file__), "..", "sdl3", "src", "video", "testdata", "images")


class Lcg:
    def __init__(self, seed):
        self.s = seed

    def next(self):
        self.s = (self.s * 6364136223846793005 + 1442695040888963407) & ((1 << 64) - 1)
        return self.s >> 33


def chunk(kind, data):
    c = struct.pack(">I", len(data)) + kind + data
    return c + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


def channels(ct):
    return {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ct]


def pack_row(samples, depth):
    """Pack one row of samples (ints) at `depth` bits."""
    if depth == 8:
        return bytes(samples)
    if depth == 16:
        return b"".join(struct.pack(">H", v) for v in samples)
    out = bytearray()
    per = 8 // depth
    for i in range(0, len(samples), per):
        b = 0
        for j in range(per):
            v = samples[i + j] if i + j < len(samples) else 0
            b |= v << (8 - depth * (j + 1))
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
    """Apply PNG filters (one per row, cycling through `filters`)."""
    out = bytearray()
    prior = bytes(len(rows[0])) if rows else b""
    for y, row in enumerate(rows):
        f = filters[y % len(filters)]
        out.append(f)
        for x in range(len(row)):
            a = row[x - bpp] if x >= bpp else 0
            b = prior[x]
            c = prior[x - bpp] if x >= bpp else 0
            pred = [0, a, b, (a + b) // 2, paeth(a, b, c)][f]
            out.append((row[x] - pred) & 0xFF)
        prior = row
    return bytes(out)


ADAM7 = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]


def make_png(name, w, h, ct, depth, *, seed=1, filters=(0, 1, 2, 3, 4), interlace=False,
             plte=None, trns=None, level=6, strategy=zlib.Z_DEFAULT_STRATEGY, idat_split=0,
             extra_chunks=(), samples=None, tamper=None):
    rng = Lcg(seed)
    n = channels(ct)
    maxv = (1 << depth) - 1
    if samples is None:
        limit = (len(plte) // 3 - 1) if ct == 3 else maxv
        samples = [[rng.next() % (limit + 1) for _ in range(w * n)] for _ in range(h)]
    bits = n * depth
    bpp = max(1, bits // 8)

    if interlace:
        raw = b""
        for (x0, y0, dx, dy) in ADAM7:
            xs = list(range(x0, w, dx))
            ys = list(range(y0, h, dy))
            if not xs or not ys:
                continue
            rows = []
            for y in ys:
                s = []
                for x in xs:
                    s.extend(samples[y][x * n:(x + 1) * n])
                rows.append(pack_row(s, depth))
            raw += filter_rows(rows, bpp, filters)
    else:
        rows = [pack_row(samples[y], depth) for y in range(h)]
        raw = filter_rows(rows, bpp, filters)

    comp = zlib.compressobj(level, zlib.DEFLATED, 15, 9, strategy)
    data = comp.compress(raw) + comp.flush()

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, ct, 0, 0, 1 if interlace else 0))
    for kind, payload in extra_chunks:
        png += chunk(kind, payload)
    if plte is not None:
        png += chunk(b"PLTE", bytes(plte))
    if trns is not None:
        png += chunk(b"tRNS", bytes(trns))
    if idat_split:
        for i in range(0, len(data), idat_split):
            png += chunk(b"IDAT", data[i:i + idat_split])
    else:
        png += chunk(b"IDAT", data)
    png += chunk(b"IEND", b"")
    if tamper:
        png = tamper(png)
    with open(os.path.join(OUT, name), "wb") as f:
        f.write(png)


def palette(count, seed):
    rng = Lcg(seed)
    return [rng.next() & 0xFF for _ in range(count * 3)]


def main():
    os.makedirs(OUT, exist_ok=True)
    W, H = 37, 23

    # Grayscale at every depth
    for d in (1, 2, 4, 8, 16):
        make_png(f"g{d}.png", W, H, 0, d, seed=d)
    # Gray with tRNS: a gray value that appears
    make_png("g8trns.png", W, H, 0, 8, seed=3, samples=[[(x * 7 + y) % 4 * 60 for x in range(W)] for y in range(H)],
             trns=[0, 60])
    make_png("g4trns.png", W, H, 0, 4, seed=4, samples=[[(x + y) % 3 for x in range(W)] for y in range(H)],
             trns=[0, 2])
    make_png("g16trns.png", W, H, 0, 16, seed=5, samples=[[((x + y) % 3) * 30000 for x in range(W)] for y in range(H)],
             trns=[0x75, 0x30])
    # Gray + alpha
    make_png("ga8.png", W, H, 4, 8, seed=6)
    make_png("ga16.png", W, H, 4, 16, seed=7)
    # RGB, RGB with tRNS, RGBA
    make_png("rgb8.png", W, H, 2, 8, seed=8)
    make_png("rgb16.png", W, H, 2, 16, seed=9)
    rgb_key = [[[10, 20, 30][(x * 3 + c) % 3] if (x + y) % 5 == 0 else (x * 13 + y * 7 + c * 50) & 0xFF
                for x in range(W) for c in range(3)] for y in range(H)]
    make_png("rgb8trns.png", W, H, 2, 8, samples=rgb_key, trns=[0, 10, 0, 20, 0, 30])
    make_png("rgb16trns.png", W, H, 2, 16, seed=10,
             samples=[[(1234 if (x + y) % 4 == 0 else (x * 999 + y * 77 + c) & 0xFFFF)
                       for x in range(W) for c in range(3)] for y in range(H)],
             trns=[0x04, 0xD2, 0x04, 0xD2, 0x04, 0xD2])
    make_png("rgba8.png", W, H, 6, 8, seed=11)
    make_png("rgba16.png", W, H, 6, 16, seed=12)
    # Paletted at every depth, with and without tRNS
    for d in (1, 2, 4, 8):
        n = min(1 << d, 200)
        make_png(f"p{d}.png", W, H, 3, d, seed=20 + d, plte=palette(n, d))
    make_png("p8trns.png", W, H, 3, 8, seed=30, plte=palette(64, 30), trns=[(i * 37) & 0xFF for i in range(40)])
    make_png("p8key.png", W, H, 3, 8, seed=31, plte=palette(32, 31), trns=[255, 255, 0])
    make_png("p4key2.png", W, H, 3, 4, seed=32, plte=palette(16, 32), trns=[0, 255, 0])
    # Filters one at a time
    for f in range(5):
        make_png(f"rgb8f{f}.png", W, H, 2, 8, seed=40 + f, filters=(f,))
        make_png(f"g2f{f}.png", W, H, 0, 2, seed=45 + f, filters=(f,))
        make_png(f"rgba16f{f}.png", W, H, 6, 16, seed=50 + f, filters=(f,))
    # Adam7 interlacing, including images smaller than a pass
    make_png("rgb8i.png", W, H, 2, 8, seed=60, interlace=True)
    make_png("g1i.png", W, H, 0, 1, seed=61, interlace=True)
    make_png("p4i.png", W, H, 3, 4, seed=62, interlace=True, plte=palette(16, 62))
    make_png("rgba16i.png", W, H, 6, 16, seed=63, interlace=True)
    make_png("ga8i3x3.png", 3, 3, 4, 8, seed=64, interlace=True)
    make_png("rgb8i1x1.png", 1, 1, 2, 8, seed=65, interlace=True)
    make_png("g8i5x1.png", 5, 1, 0, 8, seed=66, interlace=True)
    # Deflate block kinds and stream layout
    make_png("stored.png", W, H, 2, 8, seed=70, level=0)
    make_png("fixed.png", W, H, 2, 8, seed=71, strategy=zlib.Z_FIXED)
    make_png("rle.png", W, H, 0, 8, seed=72, strategy=zlib.Z_RLE,
             samples=[[(x // 9) * 40 for x in range(W)] for y in range(H)])
    make_png("best.png", W, H, 6, 8, seed=73, level=9)
    make_png("split.png", W, H, 2, 8, seed=74, idat_split=7)
    make_png("ancillary.png", W, H, 2, 8, seed=75,
             extra_chunks=[(b"tEXt", b"Comment\0hello"), (b"gAMA", struct.pack(">I", 45455)), (b"zzZz", b"private")])
    # Edge sizes
    make_png("one.png", 1, 1, 6, 8, seed=80)
    make_png("tall.png", 1, 40, 2, 8, seed=81)
    make_png("wide.png", 40, 1, 2, 8, seed=82)
    make_png("wide1bit.png", 77, 2, 0, 1, seed=83)
    # Corrupt files
    make_png("bad_filter.png", W, H, 2, 8, seed=90, filters=(1,),
             tamper=None)
    with open(os.path.join(OUT, "bad_filter.png"), "rb") as f:
        good = f.read()
    raw = bytearray(zlib.decompress(good[8 + 25 + 8:-12 - 4]))
    raw[(1 + W * 3) * 5] = 7  # filter type 7 on row 5
    data = zlib.compress(bytes(raw))
    png = good[:8 + 25] + chunk(b"IDAT", data) + chunk(b"IEND", b"")
    with open(os.path.join(OUT, "bad_filter.png"), "wb") as f:
        f.write(png)
    make_png("truncated.png", W, H, 2, 8, seed=91, tamper=lambda p: p[:len(p) * 2 // 3])
    make_png("unknown_critical.png", W, H, 2, 8, seed=92, extra_chunks=[(b"ABCD", b"x")])
    make_png("bad_zlib.png", W, H, 2, 8, seed=93,
             tamper=lambda p: p[:8 + 25 + 8] + b"\x78\x02" + p[8 + 25 + 10:])
    make_png("no_idat.png", W, H, 2, 8, seed=94, tamper=lambda p: p[:8 + 25] + chunk(b"IEND", b""))
    make_png("bad_depth.png", W, H, 2, 8, seed=95,
             tamper=lambda p: p[:24] + b"\x03" + p[25:])


if __name__ == "__main__":
    main()
