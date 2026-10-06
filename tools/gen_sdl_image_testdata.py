#!/usr/bin/env python3
# Generate the synthetic test images of sdl3-image/src/testdata/gen/.
#
# Usage: tools/gen_sdl_image_testdata.py OUTDIR
#
# The images are small (23x13, odd sizes for the row padding paths) and made
# with ImageMagick (`convert`), except the variants ImageMagick doesn't write
# (16-bit and colormapped TGAs, multi-image cursors, broken GIFs), which are
# built byte by byte here. The expected results in
# sdl3-image/src/testdata/reference.txt come from upstream SDL_image's C,
# not from this script.

import os
import struct
import subprocess
import sys


def run(*args):
    subprocess.run(["convert", *args], check=True)


def main(out):
    os.makedirs(out, exist_ok=True)
    p = lambda name: os.path.join(out, name)
    tmp = p("_base.png")
    tmpa = p("_basea.png")

    # A gradient, and the same with an alpha ramp
    run("-size", "23x13", "xc:black", "-channel", "R", "-fx", "i/w",
        "-channel", "G", "-fx", "j/h", "-channel", "B", "-fx", "(i*j%7)/7",
        "+channel", "-depth", "8", tmp)
    run(tmp, "-alpha", "set", "-channel", "A", "-fx", "(i+j)/(w+h)",
        "+channel", "-depth", "8", tmpa)

    # PCX: 8-bit palette, 24-bit planes, 1-bit, 4 planes of 1 bit
    run(tmp, "-colors", "50", "-type", "Palette", p("pcx8.pcx"))
    run(tmp, "-type", "TrueColor", p("pcx24.pcx"))
    run(tmp, "-monochrome", p("pcx1.pcx"))
    write_pcx_planes(p("pcx4planes.pcx"), 4, rle=True)
    write_pcx_planes(p("pcx2planes.pcx"), 2, rle=False)

    # PNM: ASCII and binary, with a maximum value below 255
    run(tmp, "-monochrome", "-compress", "none", p("p1.pbm"))
    run(tmp, "-monochrome", p("p4.pbm"))
    run(tmp, "-colorspace", "Gray", "-compress", "none", p("p2.pgm"))
    run(tmp, "-colorspace", "Gray", p("p5.pgm"))
    run(tmp, "-colorspace", "Gray", "-depth", "4", p("p5d4.pgm"))
    run(tmp, "-compress", "none", p("p3.ppm"))
    run(tmp, p("p6.ppm"))
    run(tmp, "-depth", "5", "-compress", "none", p("p3d5.ppm"))

    # TGA: true color, with alpha, palette, gray; raw and RLE
    run(tmp, p("tga24.tga"))
    run(tmp, "-compress", "RLE", p("tga24rle.tga"))
    run(tmpa, p("tga32.tga"))
    run(tmpa, "-compress", "RLE", p("tga32rle.tga"))
    run(tmp, "-colors", "40", "-type", "Palette", p("tga8.tga"))
    run(tmp, "-colors", "40", "-type", "Palette", "-compress", "RLE", p("tga8rle.tga"))
    run(tmp, "-colorspace", "Gray", "-type", "Grayscale", p("tgagrey.tga"))
    run(tmp, "-colorspace", "Gray", "-type", "Grayscale", "-compress", "RLE",
        p("tgagreyrle.tga"))
    write_tga16(p("tga16.tga"))
    write_tga_cmap(p("tgacmap15.tga"), 15)
    write_tga_cmap(p("tgacmap32.tga"), 32)

    # ICO/CUR: BMP entries (32-bit, 8-bit), several sizes, PNG entry
    run(tmpa, "-resize", "16x16!", p("_16.png"))
    run(tmpa, "-resize", "32x32!", p("_32.png"))
    run(tmp, "-resize", "24x24!", "-colors", "8", "-type", "Palette", p("_24.png"))
    run(p("_16.png"), p("_32.png"), p("ico_multi.ico"))
    run(p("_24.png"), p("ico_pal.ico"))
    run(tmp, "-resize", "11x9!", "-monochrome", p("ico_mono.ico"))
    run(tmp, "-colors", "100", p("ico_8.ico"))
    run("-size", "256x256", "xc:navy", "-fill", "yellow", "-draw", "circle 128,128 128,40",
        "-alpha", "set", "-channel", "A", "-fx", "i<16?0:1", "+channel", p("ico_png.ico"))
    write_ico24(p("ico_24.ico"))
    write_cur_from_ico(p("ico_multi.ico"), p("cur_multi.cur"), [(3, 4), (9, 10)])

    # GIF: plain, interlaced, transparent, two frames, an offset frame
    run(tmp, "-colors", "64", p("gif_plain.gif"))
    run(tmp, "-colors", "64", "-interlace", "GIF", p("gif_interlace.gif"))
    run(tmpa, "-channel", "A", "-threshold", "50%", "+channel", "-colors", "32",
        p("gif_trans.gif"))
    run("-delay", "20", tmp, "-delay", "30", "-negate", tmp, "-colors", "32",
        p("gif_anim.gif"))
    run("-size", "10x5", "xc:red", "-set", "page", "23x13+3+2", p("gif_offset.gif"))

    # JPEG: 4:4:4, 4:2:0, gray, progressive, CMYK
    run(tmp, "-quality", "90", "-sampling-factor", "1x1", p("jpg444.jpg"))
    run(tmp, "-quality", "75", "-sampling-factor", "2x2", p("jpg420.jpg"))
    run(tmp, "-colorspace", "Gray", p("jpggray.jpg"))
    run(tmp, "-interlace", "JPEG", p("jpgprog.jpg"))
    run(tmp, "-colorspace", "CMYK", p("jpgcmyk.jpg"))

    # PNG: palette (opaque, color key, alpha), gray, gray+alpha, 16-bit, interlaced
    run(tmp, "-colors", "20", "PNG8:" + p("png_pal.png"))
    run(tmpa, "-channel", "A", "-threshold", "50%", "+channel", "-colors", "20",
        "PNG8:" + p("png_palkey.png"))
    run(tmpa, "-colors", "20", "-define", "png:format=png8", p("png_palalpha.png"))
    run(tmp, "-colorspace", "Gray", "-define", "png:color-type=0", p("png_gray.png"))
    run(tmpa, "-colorspace", "Gray", "-define", "png:color-type=4", p("png_graya.png"))
    run(tmpa, "-depth", "16", "PNG64:" + p("png_rgba16.png"))
    run(tmp, "-interlace", "PNG", "PNG24:" + p("png_interlace.png"))

    # The detectors of the formats not decoded yet
    write_lbm(p("lbm_pbm.lbm"))
    write_xv(p("thumb.xv"))

    for f in (tmp, tmpa, p("_16.png"), p("_32.png"), p("_24.png")):
        os.remove(f)


def write_tga16(path):
    """An uncompressed, top-left 16-bit TGA (XRGB1555)."""
    w, h = 7, 5
    hdr = struct.pack("<BBBHHBHHHHBB", 0, 0, 2, 0, 0, 0, 0, 0, w, h, 16, 0x20)
    data = b"".join(
        struct.pack("<H", ((x * 4) << 10) | ((y * 6) << 5) | ((x + y) * 2))
        for y in range(h) for x in range(w))
    open(path, "wb").write(hdr + data)


def write_tga_cmap(path, cmap_bits):
    """A bottom-left 8-bit colormapped TGA with a 15- or 32-bit colormap;
    the 32-bit one has transparent entries (the last becomes the color key)."""
    w, h, ncols = 6, 4, 12
    hdr = struct.pack("<BBBHHBHHHHBB", 0, 1, 1, 0, ncols, cmap_bits, 0, 0, w, h, 8, 0)
    cmap = b""
    for i in range(ncols):
        if cmap_bits == 15:
            cmap += struct.pack("<H", ((i * 2) << 10) | ((31 - i) << 5) | (i + 3))
        else:
            cmap += bytes([i * 20, 255 - i * 20, i * 7, 0 if i in (4, 9) else 255])
    data = bytes((x + y * 3) % ncols for y in range(h) for x in range(w))
    open(path, "wb").write(hdr + cmap + data)


def rle_pcx(row):
    """PCX run-length encoding of one scan line."""
    out = b""
    i = 0
    while i < len(row):
        n = 1
        while i + n < len(row) and row[i + n] == row[i] and n < 63:
            n += 1
        if n > 1 or row[i] >= 0xc0:
            out += bytes([0xc0 | n, row[i]])
        else:
            out += bytes([row[i]])
        i += n
    return out


def write_pcx_planes(path, nplanes, rle):
    """A 1 bit per plane PCX of `nplanes` planes (2**nplanes colors)."""
    w, h = 21, 6
    bpl = ((w + 15) // 16) * 2
    colormap = b"".join(bytes([i * 16, 255 - i * 16, (i * 37) & 255]) for i in range(16))
    hdr = struct.pack("<BBBBhhhhhh48sBBhhhh54s", 10, 5, 1 if rle else 0, 1,
                      0, 0, w - 1, h - 1, 72, 72, colormap, 0, nplanes, bpl, 1, 0, 0, b"")
    data = b""
    for y in range(h):
        line = b""
        for plane in range(nplanes):
            bits = [((x * 3 + y) % (1 << nplanes)) >> plane & 1 for x in range(w)]
            bits += [0] * (bpl * 8 - w)
            line += bytes(sum(b << (7 - k) for k, b in enumerate(bits[i:i + 8]))
                          for i in range(0, len(bits), 8))
        data += rle_pcx(line) if rle else line
    open(path, "wb").write(hdr + data)


def write_ico24(path):
    """An icon with one 24-bit BMP image and an AND mask."""
    w, h = 5, 3
    pad = lambda n: b"\0" * (-n % 4)
    xor = b""
    for y in range(h):
        row = b"".join(bytes([x * 50, y * 100, 200 - x * 30]) for x in range(w))
        xor += row + pad(len(row))
    mask = b""
    for y in range(h):
        row = bytes([0b01000000 if y == 1 else 0])
        mask += row + pad(1)
    bih = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 24, 0, len(xor) + len(mask), 0, 0, 0, 0)
    image = bih + xor + mask
    dir_ = struct.pack("<HHH", 0, 1, 1) + struct.pack("<BBBBHHII", w, h, 0, 0, 1, 24, len(image), 22)
    open(path, "wb").write(dir_ + image)


def write_lbm(path):
    """An uncompressed IFF PBM picture (chunky 8-bit pixels)."""
    w, h = 4, 2
    bmhd = struct.pack(">HHhhBBBBHBBhh", w, h, 0, 0, 8, 0, 0, 0, 0, 1, 1, w, h)
    cmap = bytes(range(0, 3 * 256, 3)[i] % 256 for i in range(256 * 3 // 3)) * 3
    cmap = cmap[:768]
    body = bytes(range(w * h))
    chunks = b""
    for name, data in ((b"BMHD", bmhd), (b"CMAP", cmap), (b"BODY", body)):
        chunks += name + struct.pack(">I", len(data)) + data + (b"\0" if len(data) % 2 else b"")
    open(path, "wb").write(b"FORM" + struct.pack(">I", 4 + len(chunks)) + b"PBM " + chunks)


def write_xv(path):
    """An XV thumbnail: 3-3-2 RGB pixels after a text header."""
    w, h = 3, 2
    header = b"P7 332\n#XVVERSION:Version 2.28  Rev: 9/26/92\n#END_OF_COMMENTS\n%d %d 255\n" % (w, h)
    open(path, "wb").write(header + bytes([0x00, 0xe0, 0x1c, 0x03, 0xff, 0x92]))


def write_cur_from_ico(src, dst, hotspots):
    """The icon as a cursor: type 2, with hotspots in the directory."""
    data = bytearray(open(src, "rb").read())
    data[2:4] = struct.pack("<H", 2)
    count = struct.unpack_from("<H", data, 4)[0]
    for i in range(count):
        x, y = hotspots[i % len(hotspots)]
        struct.pack_into("<HH", data, 6 + 16 * i + 4, x, y)
    open(dst, "wb").write(bytes(data))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__ if __doc__ else "usage: gen_sdl_image_testdata.py OUTDIR")
    main(sys.argv[1])
