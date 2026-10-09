#!/usr/bin/env python3
# Generate the AVIF test images of sdl3-image/src/testdata/images/ (the
# avif_*.avif and avif_*.avifs files) for the translation of libavif
# (sdl3-image/src/avif/).
#
# Usage: AVIFENC=/path/to/avifenc tools/gen_avif_testdata.py [OUTDIR]
#
# avifenc is libavif's encoder app, built from the libavif SDL_image's
# external/libavif pins with the aom of its external/aom (no other codec
# is needed). The inputs are small synthetic Y4M and PNG pictures made
# here; the outputs cover the bit depths (8, 10, 12), the chroma layouts
# (4:2:0, 4:2:2, 4:4:4, 4:0:0), limited and full range, the matrix
# coefficients the YUV to RGB conversion handles differently (BT.601,
# BT.709, BT.2020, identity, YCgCo, chromaticity-derived), HDR PQ (the
# 10-bit surfaces SDL_image makes for it), alpha (straight and
# premultiplied, 8 and 10 bits), grids (with an alpha grid), the clap,
# irot, imir, pasp and clli properties, ICC and nclx colr boxes, Exif and
# XMP metadata, a layered (progressive) image and image sequences (with
# alpha, frame durations and repetition counts), and copies whose ispe
# boxes give other sizes, to which the decoder scales the pictures. The
# images are tiny (debug builds of the AV1 decoder are slow) and checked
# in, since encoders aren't deterministic across versions (and the image
# sequences record their creation time); the expected results in
# testdata/reference.txt come from upstream SDL_image's C with libavif and
# dav1d, not from this script.

import os
import struct
import subprocess
import sys
import tempfile
import zlib


def pattern(w, h, seed, maxval, noise=48):
    # three smooth gradients with some texture, scaled to maxval
    x = seed * 2654435761 & 0xffffffff
    planes = []
    for c in range(3):
        p = []
        for j in range(h):
            for i in range(w):
                x = (x * 1103515245 + 12345) & 0x7fffffff
                v = ((i * (37 + 11 * c) + j * (23 + 7 * c) + seed * 31) % 256) * 3 // 4
                v += (x >> 16) % noise if noise else 0
                p.append(min(255, v) * maxval // 255)
        planes.append(p)
    return planes


def alpha_pattern(w, h, maxval):
    a = []
    for j in range(h):
        for i in range(w):
            if (i // 4 + j // 4) % 3 == 0:
                v = 0
            elif (i + j) % 5 == 0:
                v = 255
            else:
                v = (i * 255) // max(1, w - 1)
            a.append(v * maxval // 255)
    return a


def subsample(p, w, h, sx, sy):
    cw, ch = (w + sx) >> sx, (h + sy) >> sy
    out = []
    for j in range(ch):
        for i in range(cw):
            out.append(p[min(h - 1, j << sy) * w + min(w - 1, i << sx)])
    return out


def y4m(path, w, h, frames, layout, depth=8, full=False, alpha=False, noise=48):
    # layout: "420", "422", "444" or "mono"; frames: seeds
    names = {
        ("420", 8): "C420jpeg", ("422", 8): "C422", ("444", 8): "C444",
        ("mono", 8): "Cmono", ("420", 10): "C420p10", ("422", 10): "C422p10",
        ("444", 10): "C444p10", ("mono", 10): "Cmono10", ("420", 12): "C420p12",
        ("422", 12): "C422p12", ("444", 12): "C444p12", ("mono", 12): "Cmono12",
    }
    cs = "C444alpha" if alpha else names[(layout, depth)]
    header = "YUV4MPEG2 W%d H%d F10:1 Ip A1:1 %s" % (w, h, cs)
    if full:
        header += " XCOLORRANGE=FULL"
    maxval = (1 << depth) - 1
    sx, sy = {"420": (1, 1), "422": (1, 0), "444": (0, 0), "mono": (1, 1)}[layout]
    with open(path, "wb") as f:
        f.write((header + "\n").encode())
        for seed in frames:
            y, u, v = pattern(w, h, seed, maxval, noise)
            planes = [y]
            if layout != "mono":
                planes += [subsample(u, w, h, sx, sy), subsample(v, w, h, sx, sy)]
            if alpha:
                planes.append(alpha_pattern(w, h, maxval))
            f.write(b"FRAME\n")
            for p in planes:
                if depth > 8:
                    f.write(struct.pack("<%dH" % len(p), *p))
                else:
                    f.write(bytes(p))


def png(path, w, h, seed, depth=8, alpha=True, noise=48):
    maxval = (1 << depth) - 1
    r, g, b = pattern(w, h, seed, maxval, noise)
    a = alpha_pattern(w, h, maxval)
    channels = 4 if alpha else 3
    raw = b""
    for j in range(h):
        row = []
        for i in range(w):
            k = j * w + i
            px = [r[k], g[k], b[k]] + ([a[k]] if alpha else [])
            row += px
        raw += b"\0" + (struct.pack(">%dH" % len(row), *row) if depth == 16 else bytes(row))

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)

    color_type = 6 if alpha else 2
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, color_type, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 9)))
        f.write(chunk(b"IEND", b""))


def with_ispe(src, dst, w, h):
    # a copy of an image whose image spatial extents (all its ispe boxes:
    # size, type, version and flags, then the width and height) say w x h,
    # which the decoder scales the decoded pictures to
    data = bytearray(open(src, "rb").read())
    i = data.find(b"ispe")
    while i >= 0:
        struct.pack_into(">II", data, i + 8, w, h)
        i = data.find(b"ispe", i + 4)
    with open(dst, "wb") as f:
        f.write(data)


def main(out):
    avifenc = os.environ.get("AVIFENC", "avifenc")
    os.makedirs(out, exist_ok=True)
    tmp = tempfile.mkdtemp()
    t = lambda name: os.path.join(tmp, name)

    def enc(name, inputs, *args, quality=60, speed=9):
        cmd = [avifenc, "-j", "1", "-s", str(speed), "-q", str(quality), *args, *inputs,
               os.path.join(out, name)]
        subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)

    # 8-bit, the chroma layouts, limited and full range
    y4m(t("420.y4m"), 24, 18, [1], "420")
    enc("avif_8bit_420.avif", [t("420.y4m")])
    y4m(t("420f.y4m"), 23, 17, [2], "420", full=True)
    enc("avif_8bit_420_full_odd.avif", [t("420f.y4m")], "--cicp", "1/13/1")
    y4m(t("422.y4m"), 21, 16, [3], "422")
    enc("avif_8bit_422.avif", [t("422.y4m")], "--cicp", "9/14/9")
    y4m(t("444.y4m"), 20, 12, [4], "444", full=True)
    enc("avif_8bit_444.avif", [t("444.y4m")], "--cicp", "1/13/5")
    y4m(t("mono.y4m"), 19, 13, [5], "mono")
    enc("avif_8bit_400.avif", [t("mono.y4m")])
    # 8-bit identity (GBR), YCgCo and chromaticity-derived matrices
    y4m(t("444i.y4m"), 16, 16, [6], "444", full=True)
    enc("avif_8bit_identity.avif", [t("444i.y4m")], "--cicp", "1/13/0")
    enc("avif_8bit_ycgco.avif", [t("444i.y4m")], "--cicp", "1/13/8")
    enc("avif_8bit_420_chromaderived.avif", [t("420.y4m")], "--cicp", "9/16/12")
    # 10 and 12 bits
    y4m(t("420_10.y4m"), 22, 14, [7], "420", depth=10)
    enc("avif_10bit_420.avif", [t("420_10.y4m")], "--cicp", "9/16/9", "--clli", "800,300")
    y4m(t("444_10.y4m"), 18, 10, [8], "444", depth=10, full=True)
    enc("avif_10bit_444_pq_identity.avif", [t("444_10.y4m")], "--cicp", "9/16/0")
    enc("avif_10bit_444_hlg.avif", [t("444_10.y4m")], "--cicp", "9/18/9")
    y4m(t("422_12.y4m"), 17, 11, [9], "422", depth=12)
    enc("avif_12bit_422.avif", [t("422_12.y4m")])
    y4m(t("mono_12.y4m"), 15, 9, [10], "mono", depth=12, full=True)
    enc("avif_12bit_400.avif", [t("mono_12.y4m")])
    # alpha: 8-bit straight and premultiplied, 10-bit
    y4m(t("alpha.y4m"), 20, 14, [11], "444", alpha=True, full=True)
    enc("avif_alpha.avif", [t("alpha.y4m")])
    png(t("alpha8.png"), 18, 12, 12)
    enc("avif_alpha_premultiplied.avif", [t("alpha8.png")], "--premultiply", "-y", "420")
    png(t("alpha16.png"), 16, 10, 13, depth=16)
    enc("avif_alpha_10bit.avif", [t("alpha16.png")], "-d", "10", "-y", "422", "--qalpha", "70")
    # grids (the cells must be at least 64x64): color only, and with alpha
    y4m(t("cell1.y4m"), 64, 64, [14], "420", noise=0)
    y4m(t("cell2.y4m"), 64, 64, [15], "420", noise=0)
    enc("avif_grid.avif", [t("cell1.y4m"), t("cell2.y4m")], "-g", "2x1", quality=20, speed=10)
    png(t("cella1.png"), 64, 64, 16, noise=0)
    png(t("cella2.png"), 64, 64, 17, noise=0)
    enc("avif_grid_alpha.avif", [t("cella1.png"), t("cella2.png")], "-g", "1x2", "-y", "420",
        quality=10, speed=10)
    # transformation properties, colr boxes and metadata
    enc("avif_clap_irot_imir.avif", [t("420.y4m")], "--crop", "2,2,18,12", "--irot", "1",
        "--imir", "1", "--pasp", "4,3")
    with open(t("icc.bin"), "wb") as f:
        f.write(b"\0\0\0\x80fake" + bytes(range(120)))
    enc("avif_icc.avif", [t("444.y4m")], "--icc", t("icc.bin"))
    with open(t("exif.bin"), "wb") as f:
        f.write(b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0")
    with open(t("xmp.xml"), "wb") as f:
        f.write(b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF '
                b'xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
                b'<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/">'
                b'<dc:title><rdf:Alt><rdf:li xml:lang="x-default">an avif</rdf:li></rdf:Alt>'
                b'</dc:title><dc:creator><rdf:Seq><rdf:li>someone</rdf:li></rdf:Seq></dc:creator>'
                b'</rdf:Description></rdf:RDF></x:xmpmeta>')
    enc("avif_exif_xmp.avif", [t("420.y4m")], "--exif", t("exif.bin"), "--xmp", t("xmp.xml"))
    # a layered (progressive) image
    y4m(t("prog.y4m"), 32, 24, [18], "420")
    enc("avif_progressive.avif", [t("prog.y4m")], "--progressive", quality=40)
    # image sequences: 8-bit 4:2:0 with frame durations and a repetition
    # count, with alpha, 10-bit
    for k in range(3):
        y4m(t("seq%d.y4m" % k), 24, 16, [20 + k], "420")
    enc("avif_anim.avifs", [t("seq0.y4m"), "--duration:u", "3", t("seq1.y4m"), "--duration:u", "7",
                            t("seq2.y4m")], "--timescale", "25", "--repetition-count", "2",
        "--xmp", t("xmp.xml"), quality=50)
    for k in range(3):
        png(t("seqa%d.png" % k), 16, 12, 30 + k)
    enc("avif_anim_alpha.avifs", [t("seqa%d.png" % k) for k in range(3)], "--timescale", "10",
        "-y", "420", quality=50)
    y4m(t("seq10.y4m"), 18, 12, [40, 41], "444", depth=10)
    enc("avif_anim_10bit.avifs", [t("seq10.y4m")], "--timescale", "30", "--repetition-count",
        "infinite", "-k", "1", quality=50)
    # pictures scaled to their ispe size (libyuv's box filter, bilinear
    # down, up and up by 2, horizontal only and vertical only, at 8, 10 and
    # 12 bits, with alpha)
    o = lambda name: os.path.join(out, name)
    with_ispe(o("avif_8bit_420.avif"), o("avif_scaled_box.avif"), 8, 6)
    with_ispe(o("avif_8bit_400.avif"), o("avif_scaled_box_odd.avif"), 6, 4)
    with_ispe(o("avif_8bit_422.avif"), o("avif_scaled_up.avif"), 40, 30)
    with_ispe(o("avif_alpha.avif"), o("avif_scaled_up2_alpha.avif"), 40, 28)
    with_ispe(o("avif_10bit_420.avif"), o("avif_scaled_10bit.avif"), 15, 14)
    with_ispe(o("avif_12bit_422.avif"), o("avif_scaled_12bit_vertical.avif"), 17, 5)


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else
         os.path.join(os.path.dirname(__file__), "..", "sdl3-image", "src", "testdata", "images"))
