#!/usr/bin/env python3
# Generate the synthetic test images of sdl3-image/src/testdata/images/ (the ones
# that aren't upstream SDL_image's own: not sample.*, rgbrgb.*, palette.*, svg.svg or
# svg-class.svg).
#
# Usage: tools/gen_sdl_image_testdata.py OUTDIR
#
# The images are small (23x13, odd sizes for the row padding paths) and made
# with ImageMagick (`convert`), except the variants ImageMagick doesn't write
# (16-bit and colormapped TGAs, multi-image cursors, broken GIFs), which are
# built byte by byte here, as are the TIFFs ImageMagick can't make (the
# codecs it doesn't write, YCbCr, broken directories). The WebP images are made with libwebp's cwebp,
# img2webp and webpmux (on PATH, or in the directory $WEBP_TOOLS; libwebp
# 1.3.2, as SDL_image's external/libwebp) and libwebp's encoding API
# through ctypes (for the token partitions cwebp doesn't set), and the raw
# alpha planes with
# their filters byte by byte. The expected results in
# sdl3-image/src/testdata/reference.txt come from upstream SDL_image's C,
# not from this script.

import ctypes
import ctypes.util
import math
import os
import shutil
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

    # LBM: a PBM whose body is too short for its padded rows, and complete
    # pictures: compressed PBM, interleaved planes with a stencil, EHB with
    # a transparent color, HAM6, 24 planes
    write_lbm(p("lbm_pbm.lbm"))
    write_lbm_pbm(p("lbm_pbm8.lbm"))
    write_ilbm(p("lbm_ilbm.lbm"), 4, compress=True, mask=1, ncolors=16)
    write_ilbm(p("lbm_ehb.lbm"), 6, compress=False, mask=2, ncolors=32, camg=0x80)
    write_ilbm(p("lbm_ham.lbm"), 6, compress=True, mask=0, ncolors=16, camg=0x800)
    write_ilbm(p("lbm_24.lbm"), 24, compress=False, mask=0, ncolors=0)

    # XV thumbnails
    write_xv(p("thumb.xv"))
    write_xv_big(p("xv_crlf.xv"))

    # XCF: RGBA layers (RLE, offsets, an invisible layer, a channel), gray
    # (uncompressed, version 11 with 64-bit offsets), indexed with alpha
    write_xcf(p("xcf_rgba.xcf"), "rgba")
    write_xcf(p("xcf_gray.xcf"), "gray")
    write_xcf(p("xcf_indexed.xcf"), "indexed")
    write_xcf(p("xcf_rgb.xcf"), "rgb")

    # XPM: many colors (32-bit), transparency, and named, short and long
    # hex colors with symbolic names and two characters per pixel
    run(tmp, p("xpm_rgb.xpm"))
    run(tmpa, "-channel", "A", "-threshold", "50%", "+channel", "-colors", "12",
        p("xpm_trans.xpm"))
    write_xpm_named(p("xpm_named.xpm"))

    # Animations: a GIF with every disposal method, offsets, transparency, a
    # loop count and a comment; ANI cursors with a sequence, rates and an
    # info list
    run("-delay", "5", "-dispose", "None", tmp,
        "-dispose", "Background", "-delay", "1",
        "(", "-size", "10x5", "xc:blue", "-set", "page", "+3+2", ")",
        "-dispose", "Previous", "-delay", "200",
        "(", tmpa, "-resize", "8x6!", "-set", "page", "+12+5", ")",
        "-dispose", "None", "-delay", "7",
        "(", "-size", "4x4", "xc:yellow", "-set", "page", "+1+8", ")",
        "-loop", "3", "-set", "comment", "gif comment", p("gif_dispose.gif"))
    write_ani(p("ani_seq.ani"), [p("cur_multi.cur"), p("ico_pal.ico"), p("ico_24.ico")],
              sequence=[0, 2, 1, 2, 0], rates=[3, 6, 9, 12, 15], title=b"test cursor",
              author=b"gen script")
    write_ani(p("ani_plain.ani"), [p("ico_multi.ico"), p("cur_multi.cur")])

    # SVG: path commands (arcs too), shapes and units, strokes (joins, caps,
    # dashes), gradients, transforms, styles and colors
    for name, text in SVGS.items():
        open(p(name), "w").write(text)

    write_tiffs(p, tmp, tmpa)

    write_webps(p, tmp, tmpa)

    for f in (tmp, tmpa, p("_16.png"), p("_32.png"), p("_24.png")):
        os.remove(f)


SVGS = {
    "svg_paths.svg": """<?xml version="1.0" encoding="UTF-8"?>
<!-- path commands, absolute and relative, and the basic shapes -->
<svg xmlns="http://www.w3.org/2000/svg" width="64" height="48" viewBox="0 0 128 96">
  <path d="M10,10 L40,10 l0,20 H10 z m40,0 h20 v20 h-20 V10 Z" fill="#c33" fill-rule="evenodd"/>
  <path d="M80 10 C 90 0, 110 0, 120 10 S 110 40, 80 30 Q 70 20 80 10 T 90 5" fill="rgb(20, 160, 90)"/>
  <path d="M10 50 a 15 10 30 1 0 30 10 A 10 10 0 0 1 60 50 a5 5 0 0,1 10 0 z" fill="navy" stroke="yellow"/>
  <path d="M 70 50 q 10 -10 20 0 t 20 0 l 5 5 Z c" fill="#0a8"/>
  <path d="m90,80 -10,10 5-15 +1.5e1.5 z" fill="rgb(50%, 20.5%, 100%)"/>
  <polygon points="5,90 15,70 25,90" fill="magenta"/>
  <polyline points="30 92 40 72 50 92 60 72" fill="none" stroke="black" stroke-width="2"/>
  <ellipse cx="110" cy="80" rx="15" ry="8" fill="#808"/>
  <circle cx="100" cy="60" r="6" fill="cyan" opacity="0.5"/>
  <rect x="62" y="62" width="20" height="14" rx="4" fill="gray" stroke="white" stroke-width="1.5"/>
  <line x1="0" y1="0" x2="128" y2="96" stroke="#f80" stroke-width="0.8"/>
  <rect x="1mm" y="0.1in" width="2em" height="5pt" fill="black" display="none"/>
</svg>
""",
    "svg_strokes.svg": """<svg xmlns="http://www.w3.org/2000/svg" width="60" height="50">
  <g fill="none" stroke="#2050c0" stroke-width="5">
    <polyline points="5,5 20,15 5,25" stroke-linejoin="miter"/>
    <polyline points="25,5 40,15 25,25" stroke-linejoin="round" stroke-linecap="round"/>
    <polyline points="45,5 58,15 45,25" stroke-linejoin="bevel" stroke-linecap="square"/>
    <polyline points="5,30 30,32 5,34" stroke-miterlimit="1.5" stroke-width="3"/>
  </g>
  <path d="M5,45 h50" stroke="red" stroke-width="3" stroke-dasharray="6,3,1" stroke-dashoffset="2"/>
  <circle cx="45" cy="38" r="7" fill="none" stroke="green" stroke-width="2" stroke-dasharray="4 2" stroke-opacity="0.7"/>
  <rect x="32" y="28" width="6" height="6" style="fill: blue; stroke: black; stroke-width: 1; fill-opacity: .5"/>
</svg>
""",
    "svg_gradients.svg": """<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="80" height="40">
  <defs>
    <linearGradient id="lg">
      <stop offset="0%" stop-color="#ff0000"/>
      <stop offset="0.5" stop-color="#00ff00" stop-opacity="0.5"/>
      <stop offset="100%" stop-color="#0000ff"/>
    </linearGradient>
    <linearGradient id="lg2" xlink:href="#lg" x1="0" y1="0" x2="0" y2="1" gradientTransform="rotate(20)"/>
    <radialGradient id="rg" cx="0.5" cy="0.5" r="0.5" fx="0.3" fy="0.3">
      <stop offset="0" stop-color="white"/>
      <stop offset="1" stop-color="black"/>
    </radialGradient>
    <radialGradient id="ug" gradientUnits="userSpaceOnUse" cx="60" cy="20" r="15" spreadMethod="reflect">
      <stop offset="0.2" stop-color="yellow"/>
      <stop offset="0.9" stop-color="#804"/>
    </radialGradient>
  </defs>
  <rect x="2" y="2" width="36" height="16" fill="url(#lg)"/>
  <rect x="2" y="22" width="36" height="16" fill="url(#lg2)" stroke="url(#rg)" stroke-width="2"/>
  <circle cx="60" cy="20" r="18" fill="url(#ug)"/>
  <circle cx="60" cy="20" r="5" fill="url(#missing)" stroke="url(#rg)"/>
</svg>
""",
    "svg_transforms.svg": """<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="-10 -10 100 60" width="90" height="45" preserveAspectRatio="xMidYMax meet">
  <style>
    .warm, .hot { fill: #e60; }
    .cool { fill: #06e; stroke: #003; stroke-width: 0.5 }
    p { fill: red }
  </style>
  <g transform="translate(10,5) rotate(15)">
    <rect class="warm" width="20" height="10"/>
    <rect class="cool" y="15" width="20" height="10" transform="skewX(20)"/>
  </g>
  <g transform="matrix(0.8 0.2 -0.2 0.8 50 0)">
    <rect class="cool hot" width="15" height="15"/>
    <circle cx="25" cy="25" r="5" transform="scale(1.5) rotate(45 25 25)" fill="#5a5"/>
    <ellipse cx="5" cy="30" rx="6" ry="3" transform="skewY(-10)" fill="#555" opacity="0.6"/>
  </g>
  <g fill="#909" display="none"><rect width="80" height="40"/></g>
  <text x="0" y="0">ignored</text>
</svg>
""",
    # Odd input nanosvg must survive the same way: deep nesting, many
    # attributes, huge and infinite numbers, degenerate dashes, gradient
    # reference loops, malformed colors and transforms, markup it skips
    "svg_odd.svg": """<?xml version='1.0'?><!DOCTYPE svg><!-- odd -->
<svg width="40" height="30" viewBox="0,0,40%,30">
<![CDATA[ <rect width="40" height="30"/> ]]>
""" + "<g fill='#123'>" * 130 + """<rect x="2" y="2" width="6" height="6"/>""" + "</g>" * 130 + """
<rect x="10" y="2" width="6" height="6" """ + " ".join('a%d="%d"' % (i, i) for i in range(140)) + """ fill="red"/>
<path d="M 1e30 1e30 L -1e30 5 L 3 1e999 z" fill="blue" stroke="black" stroke-width="1e999"/>
<path d="M 20 2 L 30 2 L 30 8 Z" fill="none" stroke="green" stroke-width="2" stroke-dasharray="0,0"/>
<path d="M 20 10 L 38 10" stroke="#0f0" stroke-width="2" stroke-dasharray="3" stroke-dashoffset="-7"/>
<path d="M 20 14 L 38 14" stroke="#00f" stroke-width="1e7" stroke-dasharray="1e999"/>
<defs>
  <linearGradient id="a" xlink:href="#b"/><linearGradient id="b" xlink:href="#a"/>
  <linearGradient id="c" x1="1" x2="0" spreadMethod="repeat">
    <stop offset="2" stop-color="red"/><stop offset="-1" stop-color="blue"/>
    <stop offset="50%" stop-color="rgb(300, -5, 20)" stop-opacity="7"/>
  </linearGradient>
</defs>
<stop offset="0.5" stop-color="white"/>
<rect x="2" y="12" width="8" height="8" fill="url(#a)" stroke="url(#c)"/>
<rect x="12" y="12" width="8" height="8" fill="url(#c)" rx="2"/>
<circle cx="5" cy="25" r="3" fill="#12" stroke="#1234567"/>
<circle cx="12" cy="25" r="3" fill="Red" stroke="  white" stroke-width="1em" font-size="0.5pc"/>
<ellipse cx="20" cy="25" rx="3cm" ry="2" fill="rgb(10%, 20%,30% )" transform="matrix(1 2 3)"/>
<rect x='25' y='20' width='5' height='5' style=' ;fill:#ff0;;stroke: ; stroke-width:x;' class='none nothing'/>
<polygon points="30 28 35 20 39 29 31" fill="#0ff" transform="translate(1) scale(0.9) skewY(5)"/>
<path d="M 1 2"/><path d="z m 1 1 l 1 1"/><path d="M5 5 A 0 0 0 0 0 10 10 A 3 3 0 1 1 5 5 Z" fill="none" stroke="red"/>
<svg x="5" y="5"><rect width="100" height="100" fill="#f0f" opacity=".2"/></svg>
</svg>
""",
    "svg_huge.svg": '<svg width="1e999" height="10"><rect width="1" height="1"/></svg>\n',
    "svg_empty.svg": "<svg>\n</svg>\n",
}


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


def byterun1(row):
    """IFF ByteRun1 (PackBits) compression of one plane's row."""
    out = b""
    i = 0
    while i < len(row):
        n = 1
        while i + n < len(row) and row[i + n] == row[i] and n < 128:
            n += 1
        if n > 1:
            out += bytes([257 - n, row[i]])
            i += n
            continue
        j = i
        while j < len(row) and j - i < 128 and not (j + 1 < len(row) and row[j + 1] == row[j]):
            j += 1
        j = max(j, i + 1)
        out += bytes([j - i - 1]) + row[i:j]
        i = j
    return out


def iff(form, chunks):
    data = b""
    for name, body in chunks:
        data += name + struct.pack(">I", len(body)) + body + (b"\0" if len(body) % 2 else b"")
    return b"FORM" + struct.pack(">I", 4 + len(data)) + form + data


def write_lbm_pbm(path):
    """A compressed PBM picture, 20 pixels wide (padded to 32)."""
    w, h = 20, 7
    bmhd = struct.pack(">HHhhBBBBHBBhh", w, h, 0, 0, 8, 2, 1, 0, 3, 1, 1, w, h)
    cmap = b"".join(bytes([i, 255 - i, (i * 7) & 255]) for i in range(256))
    body = b""
    for y in range(h):
        row = bytes(((x // 3) * 11 + y * 5) & 255 if x < w else 0 for x in range(32))
        body += byterun1(row)
    open(path, "wb").write(iff(b"PBM ", [(b"BMHD", bmhd), (b"CMAP", cmap), (b"BODY", body)]))


def write_ilbm(path, nplanes, compress, mask, ncolors, camg=None):
    """An interleaved-planes ILBM picture, 21 pixels wide (padded to 32),
    with an optional stencil plane (mask 1) or transparent color (mask 2)."""
    w, h = 21, 6
    bpl = ((w + 15) // 16) * 2
    bmhd = struct.pack(">HHhhBBBBHBBhh", w, h, 0, 0, nplanes, mask, 1 if compress else 0, 0,
                       5, 1, 1, w, h)
    cmap = b"".join(bytes([i * 8 & 255, 255 - i * 6 & 255, i * 37 & 255]) for i in range(ncolors))
    chunks = [(b"BMHD", bmhd)]
    if ncolors:
        chunks.append((b"CMAP", cmap))
    if camg is not None:
        chunks.append((b"CAMG", struct.pack(">I", camg)))
    body = b""
    for y in range(h):
        if nplanes == 24:
            pixels = [((x * 12) << 16) | ((y * 40) << 8) | (x * y * 3) for x in range(w)]
        else:
            pixels = [(x * 3 + y * 5) % (1 << nplanes) for x in range(w)]
        planes = list(range(nplanes)) + ([nplanes] if mask == 1 else [])
        for plane in planes:
            if plane == nplanes:
                bits = [(x + y) % 3 != 0 for x in range(w)]
            else:
                bits = [(v >> plane) & 1 for v in pixels]
            bits += [0] * (bpl * 8 - w)
            row = bytes(sum(b << (7 - k) for k, b in enumerate(bits[i:i + 8]))
                        for i in range(0, len(bits), 8))
            body += byterun1(row) if compress else row
    chunks.append((b"BODY", body))
    open(path, "wb").write(iff(b"ILBM", chunks))


def write_xv_big(path):
    """An XV thumbnail with CRLF line ends and several comments."""
    w, h = 23, 13
    header = (b"P7 332\r\n#XVVERSION:Version 3.10a\r\n#IMGINFO:a gradient\r\n"
              b"#END_OF_COMMENTS\r\n%d %d 255\r\n" % (w, h))
    pixels = bytes(((x * 11) & 0xe0) | ((y * 19 >> 3) & 0x1c) | (x & 3) for y in range(h) for x in range(w))
    open(path, "wb").write(header + pixels)


def xcf_rle(data):
    """GIMP's tile RLE of one channel."""
    out = b""
    i = 0
    while i < len(data):
        n = 1
        while i + n < len(data) and data[i + n] == data[i]:
            n += 1
        if n >= 3:
            if n < 128:
                out += bytes([n - 1, data[i]])
            else:
                out += bytes([127]) + struct.pack(">H", n) + bytes([data[i]])
            i += n
            continue
        j = i
        while j < len(data) and not (j + 2 < len(data) and data[j] == data[j + 1] == data[j + 2]):
            j += 1
        n = j - i
        if n < 128:
            out += bytes([256 - n]) + data[i:j]
        else:
            out += bytes([128]) + struct.pack(">H", n) + data[i:j]
        i = j
    return out


def write_xcf(path, kind):
    """A GIMP image, built chunk by chunk: the header and its properties,
    the layer and channel offsets, then each layer's hierarchy, level and
    64x64 tiles."""
    version, image_type, bpp, compress = {
        "rgba": (0, 0, 4, 1),
        "gray": (11, 1, 1, 0),
        "indexed": (3, 2, 2, 1),
        "rgb": (5, 0, 3, 0),
    }[kind]
    w, h = (70, 67) if kind == "rgba" else (23, 13)
    sign = b"gimp xcf file\0" if version == 0 else b"gimp xcf v%03d\0" % version
    osize = ">Q" if version >= 11 else ">I"
    prop = lambda pid, body: struct.pack(">II", pid, len(body)) + body
    props = prop(17, bytes([compress]))
    ncolors = 6
    if kind == "indexed":
        props += prop(1, struct.pack(">I", ncolors) +
                      b"".join(bytes([i * 40, 255 - i * 40, i * 13]) for i in range(ncolors)))
    props += prop(19, struct.pack(">ff", 72.0, 72.0)) + prop(0, b"")
    header = sign + struct.pack(">III", w, h, image_type)
    if version >= 4:
        header += struct.pack(">I", 150)
    header += props

    def pixel(layer, x, y):
        if kind == "rgba":
            a = 255 if layer == 0 else (x * 9 + y * 5) & 255
            return bytes([(x * 3 + layer * 90) & 255, (y * 3) & 255, (x * y) & 255, a])
        if kind == "gray":
            return bytes([(x * 11 + y * 3) & 255])
        if kind == "indexed":
            return bytes([(x + y) % (ncolors + 2), 255 if (x + y) % 4 else 100])
        return bytes([x * 10 & 255, y * 19 & 255, (x + y) * 5 & 255])

    # layers: (width, height, offset, visible)
    if kind == "rgba":
        layers = [(40, 30, (35, 40), True), (70, 67, (0, 0), True)]
    elif kind == "rgb":
        layers = [(10, 10, (2, 2), False), (23, 13, (0, 0), True)]
    else:
        layers = [(w, h, (0, 0), True)]

    blobs = []  # (placeholder-relative data) built after the offsets are known
    nlayers = len(layers)
    nchannels = 1 if kind == "rgba" else 0
    pos = len(header) + struct.calcsize(osize) * (nlayers + 1 + nchannels + 1)
    layer_offsets = []
    channel_offsets = []
    out = b""

    def string(s):
        return struct.pack(">I", len(s) + 1) + s + b"\0"

    for i, (lw, lh, (ox, oy), visible) in enumerate(layers):
        layer_offsets.append(pos + len(out))
        lprops = (prop(8, struct.pack(">I", 1 if visible else 0)) +
                  prop(15, struct.pack(">ii", ox, oy)) + prop(6, struct.pack(">I", 255)) + prop(0, b""))
        ltype = {"rgba": 1, "gray": 2, "indexed": 5, "rgb": 0}[kind]
        layer = struct.pack(">III", lw, lh, ltype) + string(b"Layer %d" % i) + lprops
        hier_at = pos + len(out) + len(layer) + 2 * struct.calcsize(osize)
        layer += struct.pack(osize, hier_at) + struct.pack(osize, 0)
        # hierarchy: one level, then a dummy second level GIMP writes
        tiles = []
        for ty in range(0, lh, 64):
            for tx in range(0, lw, 64):
                tw, th = min(64, lw - tx), min(64, lh - ty)
                raw = b"".join(pixel(i, tx + x, ty + y) for y in range(th) for x in range(tw))
                if compress:
                    raw = b"".join(xcf_rle(raw[c::bpp]) for c in range(bpp))
                tiles.append(raw)
        hier = struct.pack(">III", lw, lh, bpp)
        level_at = hier_at + len(hier) + 3 * struct.calcsize(osize)
        level = struct.pack(">II", lw, lh)
        tiles_at = level_at + len(level) + (len(tiles) + 1) * struct.calcsize(osize)
        dummy_at = tiles_at + sum(len(t) for t in tiles)
        hier += struct.pack(osize, level_at) + struct.pack(osize, dummy_at) + struct.pack(osize, 0)
        t_at = tiles_at
        for t in tiles:
            level += struct.pack(osize, t_at)
            t_at += len(t)
        level += struct.pack(osize, 0)
        dummy = struct.pack(">II", lw // 2, lh // 2) + struct.pack(osize, 0)
        out += layer + hier + level + b"".join(tiles) + dummy
    for i in range(nchannels):
        channel_offsets.append(pos + len(out))
        cprops = (prop(6, struct.pack(">I", 128)) + prop(8, struct.pack(">I", 1)) +
                  prop(16, bytes([20, 200, 90])) + prop(0, b""))
        out += struct.pack(">II", w, h) + string(b"Channel") + cprops + struct.pack(osize, 0)
    offsets = b"".join(struct.pack(osize, o) for o in layer_offsets) + struct.pack(osize, 0)
    offsets += b"".join(struct.pack(osize, o) for o in channel_offsets) + struct.pack(osize, 0)
    if kind == "rgb":
        # SDL_image reads the last uncompressed tile as if it were RLE data of
        # the largest size (6 bytes a pixel): without data after it, as in
        # the gray image, the read is short and the load fails
        out += b"\0" * 2048
    open(path, "wb").write(header + offsets + out)


def write_xpm_named(path):
    """An XPM with two characters per pixel: named colors (any case, some
    only in the extended table), #rgb and #rrrrggggbbbb colors, symbolic
    names, a transparent color, and a pixel key with no color."""
    colors = [
        ("..", "c None"),
        ("aa", "s background c Red"),
        ("bB", "c AliceBlue"),
        ("cc", "m white c #0F8"),
        ("d ", "c #123456789abc"),
        ("ee", "c navy"),
        ("ff", "g4 black c MediumSeaGreen s sea"),
        ("gg", "c #808080"),
    ]
    w, h = 9, 5
    keys = [k for k, _ in colors] + ["zz"]
    lines = ['"%d %d %d 2",' % (w, h, len(colors))]
    lines += ['"%s %s",' % (k, c) for k, c in colors]
    for y in range(h):
        lines.append('"' + "".join(keys[(x + y * 2) % len(keys)] for x in range(w)) + '",')
    lines[-1] = lines[-1].rstrip(",")
    text = "/* XPM */\nstatic char *named[] = {\n/* columns rows colors chars-per-pixel */\n"
    text += "\n".join(lines) + "\n};\n"
    open(path, "w").write(text)


def write_ani(path, frames, sequence=None, rates=None, title=None, author=None):
    """A Windows animated cursor (RIFF ACON) of icon or cursor files, with an
    optional sequence, rates and INFO list; odd chunks are padded."""
    def chunk(fourcc, data):
        return fourcc + struct.pack("<I", len(data)) + data + (b"\0" if len(data) % 2 else b"")

    steps = len(sequence) if sequence else len(frames)
    flags = 1 | (2 if sequence else 0)
    body = chunk(b"anih", struct.pack("<9I", 36, len(frames), steps, 0, 0, 0, 0, 10, flags))
    if title or author:
        info = b"INFO"
        if title:
            info += chunk(b"INAM", title + b"\0")
        if author:
            info += chunk(b"IART", author + b"\0")
        body += chunk(b"LIST", info)
    if rates:
        body += chunk(b"rate", struct.pack("<%dI" % len(rates), *rates))
    if sequence:
        body += chunk(b"seq ", struct.pack("<%dI" % len(sequence), *sequence))
    fram = b"fram" + b"".join(chunk(b"icon", open(f, "rb").read()) for f in frames)
    body += chunk(b"LIST", fram)
    open(path, "wb").write(b"RIFF" + struct.pack("<I", 4 + len(body)) + b"ACON" + body)


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


def webp_tool(name):
    """The path of one of libwebp's tools."""
    tools = os.environ.get("WEBP_TOOLS")
    path = os.path.join(tools, name) if tools else shutil.which(name)
    if not path or not os.path.exists(path):
        sys.exit("%s not found (set WEBP_TOOLS to libwebp's tools directory)" % name)
    return path


class WebPPicture(ctypes.Structure):
    """libwebp's struct WebPPicture (src/webp/encode.h, ABI 0x020f)."""
    _fields_ = [("use_argb", ctypes.c_int), ("colorspace", ctypes.c_int),
                ("width", ctypes.c_int), ("height", ctypes.c_int),
                ("y", ctypes.c_void_p), ("u", ctypes.c_void_p), ("v", ctypes.c_void_p),
                ("y_stride", ctypes.c_int), ("uv_stride", ctypes.c_int),
                ("a", ctypes.c_void_p), ("a_stride", ctypes.c_int),
                ("pad1", ctypes.c_uint32 * 2), ("argb", ctypes.c_void_p),
                ("argb_stride", ctypes.c_int), ("pad2", ctypes.c_uint32 * 3),
                ("writer", ctypes.c_void_p), ("custom_ptr", ctypes.c_void_p),
                ("extra_info_type", ctypes.c_int), ("extra_info", ctypes.c_void_p),
                ("stats", ctypes.c_void_p), ("error_code", ctypes.c_int),
                ("progress_hook", ctypes.c_void_p), ("user_data", ctypes.c_void_p),
                ("pad3", ctypes.c_uint32 * 3), ("pad4", ctypes.c_void_p),
                ("pad5", ctypes.c_void_p), ("pad6", ctypes.c_uint32 * 8),
                ("memory_", ctypes.c_void_p), ("memory_argb_", ctypes.c_void_p),
                ("pad7", ctypes.c_void_p * 2)]


class WebPMemoryWriter(ctypes.Structure):
    """libwebp's struct WebPMemoryWriter."""
    _fields_ = [("mem", ctypes.POINTER(ctypes.c_uint8)), ("size", ctypes.c_size_t),
                ("max_size", ctypes.c_size_t), ("pad", ctypes.c_uint32 * 1)]


# The fields of libwebp's struct WebPConfig (all 32-bit), in order
WEBP_CONFIG_FIELDS = ["lossless", "quality", "method", "image_hint", "target_size",
                      "target_PSNR", "segments", "sns_strength", "filter_strength",
                      "filter_sharpness", "filter_type", "autofilter", "alpha_compression",
                      "alpha_filtering", "alpha_quality", "pass", "show_compressed",
                      "preprocessing", "partitions"]


def write_webp_with_config(path, rgb, w, h, quality, **options):
    """A lossy WebP of RGB samples, encoded with libwebp's advanced API and
    the given WebPConfig fields."""
    lib = ctypes.CDLL(ctypes.util.find_library("webp"))
    config = (ctypes.c_uint32 * 64)()
    if not lib.WebPConfigInitInternal(config, 0, ctypes.c_float(quality), 0x020f):
        sys.exit("WebPConfigInit failed")
    for name, value in options.items():
        config[WEBP_CONFIG_FIELDS.index(name)] = value
    if not lib.WebPValidateConfig(config):
        sys.exit("invalid WebPConfig")
    pic = WebPPicture()
    if not lib.WebPPictureInitInternal(ctypes.byref(pic), 0x020f):
        sys.exit("WebPPictureInit failed")
    pic.width, pic.height = w, h
    if not lib.WebPPictureImportRGB(ctypes.byref(pic), rgb, 3 * w):
        sys.exit("WebPPictureImportRGB failed")
    writer = WebPMemoryWriter()
    lib.WebPMemoryWriterInit(ctypes.byref(writer))
    pic.writer = ctypes.cast(lib.WebPMemoryWrite, ctypes.c_void_p).value
    pic.custom_ptr = ctypes.addressof(writer)
    if not lib.WebPEncode(config, ctypes.byref(pic)):
        sys.exit("WebPEncode failed: %d" % pic.error_code)
    open(path, "wb").write(ctypes.string_at(writer.mem, writer.size))
    lib.WebPPictureFree(ctypes.byref(pic))
    lib.WebPMemoryWriterClear(ctypes.byref(writer))


def riff_chunks(data):
    """The (fourcc, payload) chunks of a RIFF WEBP file."""
    assert data[:4] == b"RIFF" and data[8:12] == b"WEBP"
    chunks = []
    pos = 12
    while pos + 8 <= len(data):
        fourcc = data[pos:pos + 4]
        size = struct.unpack_from("<I", data, pos + 4)[0]
        chunks.append((fourcc, data[pos + 8:pos + 8 + size]))
        pos += 8 + size + (size & 1)
    return chunks


def riff_chunk(fourcc, payload):
    return fourcc + struct.pack("<I", len(payload)) + payload + (b"\0" if len(payload) % 2 else b"")


def filter_alpha(alpha, w, h, method):
    """The ALPH chunk's spatial filters (1 horizontal, 2 vertical, 3
    gradient), as libwebp's unfilters invert them: the first row is
    filtered horizontally from 0, the first sample of the others from the
    sample above."""
    out = bytearray(len(alpha))
    for y in range(h):
        row = alpha[y * w:(y + 1) * w]
        prev = alpha[(y - 1) * w:y * w] if y > 0 else None
        for x in range(w):
            if prev is None:
                pred = row[x - 1] if x > 0 else 0
            elif method == 1:
                pred = row[x - 1] if x > 0 else prev[0]
            elif method == 2:
                pred = prev[x]
            elif x == 0:
                pred = prev[0]
            else:
                pred = min(255, max(0, row[x - 1] + prev[x] - prev[x - 1]))
            out[y * w + x] = (row[x] - pred) & 0xff
    return bytes(out)


def write_webp_raw_alpha(path, vp8, alpha, w, h, method):
    """A lossy image with an uncompressed ALPH chunk, filtered with
    'method' (0 to 3)."""
    data = filter_alpha(alpha, w, h, method) if method else alpha
    vp8x = struct.pack("<I", 0x10) + struct.pack("<I", w - 1)[:3] + struct.pack("<I", h - 1)[:3]
    body = b"WEBP" + riff_chunk(b"VP8X", vp8x) + riff_chunk(b"ALPH", bytes([method << 2]) + data)
    body += riff_chunk(b"VP8 ", vp8)
    open(path, "wb").write(b"RIFF" + struct.pack("<I", len(body)) + body)


WEBP_XMP = """<?xpacket begin='' id='W5M0MpCehiHzreSzNTczkc9d'?>
<x:xmpmeta xmlns:x='adobe:ns:meta/'>
 <rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'>
  <rdf:Description rdf:about='' xmlns:dc='http://purl.org/dc/elements/1.1/'
    xmlns:xmp='http://ns.adobe.com/xap/1.0/'>
   <dc:title><rdf:Alt><rdf:li xml:lang='fr'>titre</rdf:li>
    <rdf:li xml:lang='en-US'>A &amp; B</rdf:li></rdf:Alt></dc:title>
   <dc:creator><rdf:Seq><rdf:li> gen script </rdf:li></rdf:Seq></dc:creator>
   <dc:rights><rdf:Alt><rdf:li xml:lang='de'>frei</rdf:li></rdf:Alt></dc:rights>
   <xmp:CreateDate>2024-05-06T07:08:09</xmp:CreateDate>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end='w'?>"""


def write_webps(p, tmp, tmpa):
    """The WebP images: lossy (simple and complex in-loop filters,
    segments, partitions), lossy with alpha (lossless-compressed, quantized
    and raw with each filter), lossless (with and without alpha, paletted)
    and animations (lossy and lossless frames, offsets, disposal, blending,
    a background color, a loop count and XMP metadata)."""
    cwebp, img2webp, webpmux = (webp_tool(t) for t in ("cwebp", "img2webp", "webpmux"))

    def enc(src, dst, *args):
        subprocess.run([cwebp, "-quiet", *args, src, "-o", dst], check=True)

    big = p("_big.png")
    biga = p("_biga.png")
    run("-seed", "7", "-size", "37x29", "plasma:red-blue", "-depth", "8", big)
    run(big, "-alpha", "set", "-channel", "A", "-fx", "0.5+0.5*sin(i/3)*cos(j/4)",
        "+channel", "-depth", "8", biga)

    enc(tmp, p("webp_lossy.webp"), "-q", "75")
    # (cwebp has no option for the token partitions: libwebp's own API sets
    # them, and they need a method below 3, without the token buffer)
    rgb = subprocess.run(["convert", big, "-depth", "8", "rgb:-"],
                         check=True, capture_output=True).stdout
    write_webp_with_config(p("webp_lossy_strong.webp"), rgb, 37, 29, quality=60.0,
                           filter_type=1, filter_strength=60, filter_sharpness=3,
                           segments=4, partitions=3, sns_strength=80, method=2)
    enc(big, p("webp_lossy_simple.webp"), "-q", "40", "-nostrong", "-f", "40", "-segments", "2")
    enc(big, p("webp_lossy_nofilter.webp"), "-q", "90", "-f", "0", "-segments", "1")
    enc(tmpa, p("webp_lossy_alpha.webp"), "-q", "75")
    enc(biga, p("webp_alpha_best.webp"), "-q", "50", "-alpha_filter", "best")
    enc(biga, p("webp_alpha_q.webp"), "-q", "50", "-alpha_q", "20")
    enc(tmp, p("webp_lossless.webp"), "-lossless", "-z", "9")
    enc(big, p("webp_lossless_big.webp"), "-lossless", "-m", "6", "-q", "100")
    enc(tmpa, p("webp_lossless_alpha.webp"), "-lossless", "-exact")
    pal = p("_pal.png")
    run(tmp, "-colors", "4", "PNG8:" + pal)
    enc(pal, p("webp_palette.webp"), "-lossless")
    os.remove(pal)

    # Uncompressed alpha planes, unfiltered and with each filter
    vp8 = dict(riff_chunks(open(p("webp_lossy.webp"), "rb").read()))[b"VP8 "]
    alpha = subprocess.run(["convert", tmpa, "-alpha", "extract", "-depth", "8", "gray:-"],
                           check=True, capture_output=True).stdout
    for method, name in ((0, "raw"), (1, "h"), (2, "v"), (3, "g")):
        write_webp_raw_alpha(p("webp_alpha_%s.webp" % name), vp8, alpha, 23, 13, method)

    # Animations: img2webp's (sub-frames, blending and disposal of its
    # choice), and webpmux's with set offsets, disposal and blending
    f2 = p("_f2.png")
    run(tmp, "-fill", "red", "-draw", "rectangle 5,3 12,8", f2)
    f3 = p("_f3.png")
    run(tmp, "-negate", f3)
    subprocess.run([img2webp, "-loop", "3", "-mixed", "-d", "100", tmp, "-d", "50", f2,
                    "-lossy", "-q", "70", "-d", "70", f3, "-o", p("webp_anim.webp")],
                   check=True, capture_output=True)
    frames = []
    for i, (src, args) in enumerate(((tmpa, ["-lossless"]),
                                     (biga, ["-q", "60", "-resize", "10", "6"]),
                                     (big, ["-lossless", "-resize", "8", "8"]),
                                     (tmp, ["-q", "80"]))):
        frames.append(p("_frame%d.webp" % i))
        enc(src, frames[-1], *args)
    xmp = p("_meta.xmp")
    open(xmp, "w").write(WEBP_XMP)
    anim = p("_anim.webp")
    subprocess.run([webpmux, "-frame", frames[0], "+40+0+0+0+b",
                    "-frame", frames[1], "+30+4+2+1+b",
                    "-frame", frames[2], "+20+12+4+0-b",
                    "-frame", frames[3], "+10+0+0+1-b",
                    "-loop", "2", "-bgcolor", "255,10,200,30", "-o", anim],
                   check=True, capture_output=True)
    subprocess.run([webpmux, "-set", "xmp", xmp, anim, "-o", p("webp_anim_alpha.webp")],
                   check=True, capture_output=True)
    opaque = [p("_opaque%d.webp" % i) for i in range(3)]
    enc(tmp, opaque[0], "-q", "70")
    enc(big, opaque[1], "-q", "70", "-resize", "9", "7")
    enc(f3, opaque[2], "-lossless", "-resize", "6", "6")
    subprocess.run([webpmux, "-frame", opaque[0], "+60+0+0+1+b",
                    "-frame", opaque[1], "+30+2+2+1-b",
                    "-frame", opaque[2], "+5+16+6+0+b",
                    "-bgcolor", "255,10,200,30", "-o", p("webp_anim_bgcolor.webp")],
                   check=True, capture_output=True)
    for f in [big, biga, f2, f3, xmp, anim] + frames + opaque:
        os.remove(f)


def write_tiffs(p, tmp, tmpa):
    """The TIFF images: ImageMagick's (through the system's libtiff) for
    the common layouts, and the rest built here byte by byte."""
    t = lambda name: "TIFF:" + p(name)
    tiff_none = ["-compress", "None"]

    # Strips and tiles, both byte orders, the codecs SDL_image's libtiff
    # has (none, PackBits, LZW with and without the predictor, CCITT)
    run(tmp, *tiff_none, "-endian", "LSB", t("tif_rgb.tif"))
    run(tmp, "-compress", "LZW", "-define", "tiff:predictor=1", "-endian", "MSB",
        t("tif_rgb_lzw_be.tif"))
    run(tmp, "-compress", "LZW", "-define", "tiff:predictor=2",
        "-define", "tiff:rows-per-strip=4", t("tif_rgb_lzw_pred.tif"))
    run(tmp, "-compress", "RLE", "-define", "tiff:rows-per-strip=5", t("tif_rgb_packbits.tif"))
    run(tmp, "-depth", "16", "-compress", "LZW", "-define", "tiff:predictor=2",
        "-endian", "MSB", t("tif_rgb16_pred_be.tif"))
    run(tmp, "-depth", "16", *tiff_none, "-endian", "LSB", t("tif_rgb16.tif"))
    run(tmp, "-compress", "LZW", "-define", "tiff:tile-geometry=16x16", t("tif_tiled_rgb.tif"))
    run(tmp, "-depth", "16", "-compress", "LZW", "-endian", "MSB",
        "-define", "tiff:tile-geometry=16x16", t("tif_tiled_rgb16_be.tif"))
    # (libtiff takes uncompressed tiles of a multiple of 1024 bytes only,
    # the size it rounds its buffer up to)
    run(tmp, *tiff_none, "-define", "tiff:tile-geometry=16x16", t("tif_tiled_badcounts.tif"))
    run(tmp, "-compress", "LZW", "TIFF64:" + p("tif_bigtiff.tif"))

    # Alpha: associated, unassociated, unspecified (taken as associated),
    # 8 and 16 bits
    run(tmpa, *tiff_none, "-define", "tiff:alpha=associated", t("tif_rgba_assoc.tif"))
    run(tmpa, "-compress", "LZW", "-define", "tiff:alpha=unassociated", t("tif_rgba_unassoc.tif"))
    run(tmpa, *tiff_none, "-define", "tiff:alpha=unspecified", t("tif_rgba_unspec.tif"))
    run(tmpa, "-depth", "16", *tiff_none, "-define", "tiff:alpha=associated",
        t("tif_rgba16_assoc.tif"))
    run(tmpa, "-depth", "16", "-compress", "LZW", "-define", "tiff:alpha=unassociated",
        "-endian", "MSB", t("tif_rgba16_unassoc_be.tif"))

    # Separate planes, in strips and tiles
    run(tmp, *tiff_none, "-interlace", "Plane", t("tif_planar_rgb.tif"))
    run(tmpa, "-compress", "LZW", "-interlace", "Plane", "-define", "tiff:alpha=associated",
        t("tif_planar_rgba.tif"))
    run(tmpa, *tiff_none, "-interlace", "Plane", "-define", "tiff:alpha=unassociated",
        "-define", "tiff:rows-per-strip=3", t("tif_planar_rgba_unassoc.tif"))
    run(tmp, "-depth", "16", *tiff_none, "-interlace", "Plane", "-endian", "MSB",
        t("tif_planar_rgb16_be.tif"))
    run(tmpa, "-depth", "16", *tiff_none, "-interlace", "Plane",
        "-define", "tiff:alpha=unassociated", t("tif_planar_rgba16_unassoc.tif"))
    run(tmp, "-compress", "LZW", "-interlace", "Plane", "-define", "tiff:tile-geometry=16x16",
        t("tif_planar_tiled_rgb.tif"))

    # Palettes, gray at every depth, bilevel (both polarities, CCITT)
    run(tmp, "-colors", "50", "-type", "Palette", "-compress", "RLE", t("tif_pal8.tif"))
    run(tmp, "-colors", "12", "-type", "Palette", "-depth", "4", *tiff_none, t("tif_pal4.tif"))
    run(tmp, "-colors", "4", "-type", "Palette", "-depth", "2", *tiff_none, t("tif_pal2.tif"))
    run(tmp, "-colors", "50", "-type", "Palette", "-compress", "LZW",
        "-define", "tiff:tile-geometry=16x16", t("tif_tiled_pal8.tif"))
    run(tmp, "-colorspace", "Gray", *tiff_none, t("tif_gray8.tif"))
    run(tmp, "-colorspace", "Gray", "-depth", "16", "-compress", "LZW", "-endian", "MSB",
        t("tif_gray16_be.tif"))
    run(tmp, "-colorspace", "Gray", "-depth", "4", *tiff_none, t("tif_gray4.tif"))
    run(tmp, "-colorspace", "Gray", "-depth", "2", *tiff_none, t("tif_gray2.tif"))
    run(tmp, "-colorspace", "Gray", "-define", "quantum:polarity=min-is-white", *tiff_none,
        t("tif_gray8_miniswhite.tif"))
    run(tmp, "-colorspace", "Gray", "-define", "tiff:tile-geometry=16x16", "-compress", "LZW",
        "-define", "tiff:predictor=2", t("tif_tiled_gray8.tif"))
    run(tmp, "-monochrome", "-depth", "1", *tiff_none, t("tif_bilevel.tif"))
    run(tmp, "-monochrome", "-depth", "1", "-define", "quantum:polarity=min-is-white",
        *tiff_none, t("tif_bilevel_miniswhite.tif"))
    run(tmp, "-monochrome", "-compress", "Fax", t("tif_g3.tif"))
    run(tmp, "-monochrome", "-compress", "Group4", t("tif_g4.tif"))
    run(tmp, "-monochrome", "-compress", "Group4", "-define", "tiff:fill-order=lsb",
        "-endian", "MSB", t("tif_g4_lsb.tif"))
    run(tmp, "-monochrome", "-compress", "Group4", "-define", "tiff:tile-geometry=16x16",
        t("tif_tiled_g4.tif"))

    # CMYK, CIE L*a*b*
    run(tmp, "-colorspace", "CMYK", *tiff_none, t("tif_cmyk.tif"))
    run(tmp, "-colorspace", "Lab", *tiff_none, t("tif_lab8.tif"))
    run(tmp, "-colorspace", "Lab", "-depth", "16", *tiff_none, t("tif_lab16.tif"))

    # Orientations: the tag as written, the pixels as they are
    for o, name in enumerate(["TopRight", "BottomRight", "BottomLeft", "LeftTop", "RightTop",
                              "RightBottom", "LeftBottom"], 2):
        run(tmp, *tiff_none, "-orient", name, t("tif_orient%d.tif" % o))
    run(tmp, "-define", "tiff:tile-geometry=16x16", "-compress", "LZW", "-orient",
        "BottomRight", t("tif_tiled_orient3.tif"))
    run(tmp, "-define", "tiff:tile-geometry=16x16", "-compress", "LZW", "-orient",
        "LeftBottom", t("tif_tiled_orient8.tif"))

    # What SDL_image's libtiff can't decode: codecs it's built without,
    # floating point and 32-bit samples
    run(tmp, "-compress", "JPEG", t("tif_jpeg.tif"))
    run(tmp, "-compress", "Zip", t("tif_zip.tif"))
    run(tmp, "-define", "quantum:format=floating-point", "-depth", "32", *tiff_none,
        t("tif_float.tif"))
    run(tmp, "-depth", "32", *tiff_none, t("tif_rgb32.tif"))

    # Built here: the codecs ImageMagick doesn't write (ThunderScan, NeXT,
    # CCITT RLE, SGI LogL and LogLuv), YCbCr with every subsampling, and
    # broken directories
    write_tif_thunder(p("tif_thunder.tif"))
    write_tif_next(p("tif_next.tif"))
    write_tif_ccitt_rle(p("tif_ccitt_rle.tif"), word_aligned=False)
    # (libtiff loses the word alignment after the first row of this one)
    write_tif_ccitt_rle(p("tif_ccitt_rlew.tif"), word_aligned=True, be=True)
    write_tif_logluv(p("tif_logluv.tif"))
    write_tif_logluv(p("tif_logluv_tiled.tif"), tiled=True, be=True)
    write_tif_logluv24(p("tif_logluv24.tif"))
    write_tif_logl(p("tif_logl.tif"))
    for hs, vs in [(1, 1), (2, 1), (2, 2), (4, 1), (4, 2), (4, 4), (1, 2)]:
        write_tif_ycbcr(p("tif_ycbcr%d%d.tif" % (hs, vs)), hs, vs)
    write_tif_ycbcr(p("tif_ycbcr44_even.tif"), 4, 4, w=24, h=16, be=True)
    write_tif_ycbcr(p("tif_ycbcr42_even.tif"), 4, 2, w=24, h=14, refbw=True)
    write_tif_ycbcr(p("tif_ycbcr11_planar.tif"), 1, 1, planar=True, refbw=True)
    write_tif_ycbcr(p("tif_ycbcr_badsub.tif"), 3, 1)
    write_tif_planar_cmyk(p("tif_planar_cmyk.tif"))
    write_tif_graya(p("tif_graya.tif"), planar=False)
    write_tif_graya(p("tif_planar_graya.tif"), planar=True)
    write_tif_tiled_gray(p("tif_tiled_gray_none.tif"))
    write_tif_broken(p)


def tiff_bytes(w, h, tags, chunks, be=False, tiled=False):
    """A classic TIFF file: the chunks (strips or tiles) after the header,
    then the directory of the tags ({tag: (type, values)}, None to leave
    one out) and the width, height, offsets and byte counts."""
    e = ">" if be else "<"
    out = bytearray(b"MM\0\x2a" if be else b"II\x2a\0") + b"\0\0\0\0"
    offsets = []
    for c in chunks:
        offsets.append(len(out))
        out += c
        if len(out) % 2:
            out += b"\0"
    tags = dict(tags)
    tags.setdefault(256, (4, [w]))
    tags.setdefault(257, (4, [h]))
    tags.setdefault(324 if tiled else 273, (4, offsets))
    tags.setdefault(325 if tiled else 279, (4, [len(c) for c in chunks]))
    tags = {k: v for k, v in tags.items() if v is not None}
    ifd = len(out)
    struct.pack_into(e + "I", out, 4, ifd)
    extra_at = ifd + 2 + 12 * len(tags) + 4
    entries = bytearray(struct.pack(e + "H", len(tags)))
    extra = bytearray()
    fmt = {1: "B", 3: "H", 4: "I", 5: "II", 11: "f"}
    for tag in sorted(tags):
        typ, vals = tags[tag]
        if typ == 2:
            data = bytes(vals) + b"\0"
            count = len(data)
        elif typ == 5:
            data = b"".join(struct.pack(e + "II", *v) for v in vals)
            count = len(vals)
        else:
            data = b"".join(struct.pack(e + fmt[typ], v) for v in vals)
            count = len(vals)
        if len(data) <= 4:
            field = data + bytes(4 - len(data))
        else:
            field = struct.pack(e + "I", extra_at + len(extra))
            extra += data
            if len(extra) % 2:
                extra += b"\0"
        entries += struct.pack(e + "HHI", tag, typ, count) + field
    out += entries + b"\0\0\0\0" + extra
    return bytes(out)


def write_tif_thunder(path, w=23, h=13):
    """ThunderScan 4-bit: runs, 2-bit and 3-bit deltas and raw pixels."""
    two = {0: 0, 1: 1, -1: 3}
    three = {0: 0, 1: 1, 2: 2, 3: 3, -3: 5, -2: 6, -1: 7}
    data = bytearray()
    for y in range(h):
        row = [((x // 3) + y) % 16 if x < 12 else (x * 5 + y * 3) % 16 for x in range(w)]
        if y % 4 == 3:
            row = [(x + y) % 16 for x in range(w)]
        last, x = 0, 0
        while x < w:
            k = 0
            while x + k < w and row[x + k] == last and k < 63:
                k += 1
            if k >= 2:
                data.append(0x00 | k)
                x += k
                continue
            d = [row[x + i] - (row[x + i - 1] if i else last) for i in range(min(3, w - x))]
            if len(d) == 3 and all(v in two for v in d) and 0 <= last + d[0] <= 15:
                data.append(0x40 | two[d[0]] << 4 | two[d[1]] << 2 | two[d[2]])
                last = row[x + 2]
                x += 3
            elif len(d) >= 2 and d[0] in three and d[1] in three:
                data.append(0x80 | three[d[0]] << 3 | three[d[1]])
                last = row[x + 1]
                x += 2
            elif len(d) == 1 and d[0] in three:
                data.append(0x80 | three[d[0]] << 3 | 4)
                last = row[x]
                x += 1
            else:
                data.append(0xc0 | row[x])
                last = row[x]
                x += 1
    tags = {258: (3, [4]), 259: (3, [32809]), 262: (3, [1]), 277: (3, [1]), 278: (4, [h])}
    open(path, "wb").write(tiff_bytes(w, h, tags, [bytes(data)]))


def write_tif_next(path, w=23, h=13):
    """NeXT 2-bit: literal rows, literal spans and runs."""
    scanline = (w * 2 + 7) // 8
    data = bytearray()
    for y in range(h):
        if y % 3 == 0:
            data.append(0x00)
            data += bytes((y * 37 + i * 91) & 0xff for i in range(scanline))
        elif y % 3 == 1:
            data += bytes([0x40, 0, 1, 0, 3]) + bytes((y * 13 + i * 7) & 0xff for i in range(3))
        else:
            x, g = 0, y % 4
            while x < w:
                n = min(1 + (x + y) % 7, w - x)
                data.append(g << 6 | n)
                g = (g + 1) % 4
                x += n
    tags = {258: (3, [2]), 259: (3, [32766]), 262: (3, [1]), 277: (3, [1]), 278: (4, [h])}
    open(path, "wb").write(tiff_bytes(w, h, tags, [bytes(data)]))


# The terminating codes of T.4's modified Huffman code (runs of 0 to 23)
WHITE_CODES = ["00110101", "000111", "0111", "1000", "1011", "1100", "1110", "1111",
               "10011", "10100", "00111", "01000", "001000", "000011", "110100", "110101",
               "101010", "101011", "0100111", "0001100", "0001000", "0010111", "0000011",
               "0000100"]
BLACK_CODES = ["0000110111", "010", "11", "10", "011", "0011", "0010", "00011", "000101",
               "000100", "0000100", "0000101", "0000111", "00000100", "00000111",
               "000011000", "0000010111", "0000011000", "0000001000", "00001100111",
               "00001101000", "00001101100", "00000110111", "00000101000"]


def write_tif_ccitt_rle(path, word_aligned, be=False, w=23, h=13):
    """CCITT modified Huffman (RLE), each row byte (or word, RLEW)
    aligned."""
    data = bytearray()
    for y in range(h):
        runs, x, color = [], 0, 0
        if y % 4 == 2:
            runs.append(0)  # a row starting black
            color = 1
        while x < w:
            n = min((y + 2 * len(runs)) % 6 + 1 + color * 2, w - x)
            runs.append(n)
            x += n
            color ^= 1
        bits = "".join((BLACK_CODES if i % 2 else WHITE_CODES)[n] for i, n in enumerate(runs))
        align = 16 if word_aligned else 8
        bits += "0" * (-len(bits) % align)
        data += bytes(int(bits[i:i + 8], 2) for i in range(0, len(bits), 8))
    tags = {258: (3, [1]), 259: (3, [32771 if word_aligned else 2]), 262: (3, [0]),
            277: (3, [1]), 278: (4, [h])}
    open(path, "wb").write(tiff_bytes(w, h, tags, [bytes(data)], be=be))


def sgilog_rle(plane):
    """A byte plane of a SGILog row: runs (128 + n - 2, byte) of 4 or more
    equal bytes, literals (n, bytes) between them."""
    out, i = bytearray(), 0
    while i < len(plane):
        k = 1
        while i + k < len(plane) and plane[i + k] == plane[i] and k < 129:
            k += 1
        if k >= 4:
            out += bytes([128 + k - 2, plane[i]])
            i += k
            continue
        j = i
        while j < len(plane) and j - i < 127:
            k = 1
            while j + k < len(plane) and plane[j + k] == plane[j] and k < 4:
                k += 1
            if k >= 4:
                break
            j += 1
        out += bytes([j - i]) + bytes(plane[i:j])
        i = j
    return bytes(out)


def logluv_pixel(x, y):
    """A 32-bit LogLuv pixel: luminance from about 0.05 to 1.5, chroma
    around neutral (u', v' about 0.21, 0.47)."""
    le = int(256 * (math.log2(0.05 + (x + y) / 24.0) + 64))
    ue = 70 + (x * 3) % 30
    ve = 180 + (y * 5) % 30
    return le << 16 | ue << 8 | ve


def write_tif_logluv(path, tiled=False, be=False, w=23, h=13):
    """SGILog 32-bit LogLuv, its byte planes run length coded."""
    def rows_of(x0, y0, cw, ch):
        data = bytearray()
        for y in range(y0, y0 + ch):
            px = [logluv_pixel(x, y) if x < w and y < h else 0 for x in range(x0, x0 + cw)]
            for shift in (24, 16, 8, 0):
                data += sgilog_rle([(v >> shift) & 0xff for v in px])
        return bytes(data)
    tags = {258: (3, [8, 8, 8]), 259: (3, [34676]), 262: (3, [32845]), 277: (3, [3])}
    if tiled:
        tags[322] = (4, [16])
        tags[323] = (4, [16])
        chunks = [rows_of(x, y, 16, 16) for y in range(0, h, 16) for x in range(0, w, 16)]
    else:
        tags[278] = (4, [5])
        chunks = [rows_of(0, y, w, min(5, h - y)) for y in range(0, h, 5)]
    open(path, "wb").write(tiff_bytes(w, h, tags, chunks, be=be, tiled=tiled))


def write_tif_logluv24(path, w=23, h=13):
    """SGILog24: 10-bit luminance and a 14-bit chroma index (the last
    column's out of range)."""
    data = bytearray()
    for y in range(h):
        for x in range(w):
            le = int(64 * (math.log2(0.02 + (x + 2 * y) / 30.0) + 12))
            ce = 16300 if x == w - 1 else (8000 + x * 397 + y * 811) % 16289
            v = le << 14 | ce
            data += bytes([v >> 16 & 0xff, v >> 8 & 0xff, v & 0xff])
    tags = {258: (3, [8, 8, 8]), 259: (3, [34677]), 262: (3, [32845]), 277: (3, [3]),
            278: (4, [h])}
    open(path, "wb").write(tiff_bytes(w, h, tags, [bytes(data)]))


def write_tif_logl(path, w=23, h=13):
    """SGILog LogL: 16-bit luminance (negative in the last row), two
    byte planes a row."""
    data = bytearray()
    for y in range(h):
        px = [int(256 * (math.log2(0.01 + (x * 3 + y) / 40.0) + 64)) for x in range(w)]
        if y == h - 1:
            px = [v | 0x8000 for v in px]
        for shift in (8, 0):
            data += sgilog_rle([(v >> shift) & 0xff for v in px])
    tags = {258: (3, [8]), 259: (3, [34676]), 262: (3, [32844]), 277: (3, [1]), 278: (4, [h])}
    open(path, "wb").write(tiff_bytes(w, h, tags, [bytes(data)]))


def write_tif_ycbcr(path, hs, vs, w=23, h=13, be=False, planar=False, refbw=False):
    """YCbCr with hs x vs subsampling: blocks of luma samples followed by
    Cb and Cr (or three planes, unsubsampled)."""
    luma = lambda x, y: (x * 11 + y * 17) % 220 + 16
    cb = lambda bx, by: 128 + ((bx * 23 + by * 5) % 90) - 45
    cr = lambda bx, by: 128 + ((bx * 7 + by * 29) % 90) - 45
    tags = {258: (3, [8, 8, 8]), 259: (3, [1]), 262: (3, [6]), 277: (3, [3]),
            278: (4, [h]), 530: (3, [hs, vs])}
    if refbw:
        tags[532] = (5, [(16, 1), (235, 1), (128, 1), (240, 1), (128, 1), (240, 1)])
        tags[529] = (5, [(2125, 10000), (7154, 10000), (721, 10000)])
    if planar:
        tags[284] = (3, [2])
        chunks = [bytes(f(x, y) for y in range(h) for x in range(w))
                  for f in (luma, lambda x, y: cb(x, y), lambda x, y: cr(x, y))]
    else:
        data = bytearray()
        bw = max(hs, 1)
        for by in range((h + vs - 1) // vs):
            for bx in range((w + bw - 1) // bw):
                for j in range(vs):
                    for i in range(hs):
                        data.append(luma(bx * hs + i, by * vs + j))
                data += bytes([cb(bx, by), cr(bx, by)])
        chunks = [bytes(data)]
    open(path, "wb").write(tiff_bytes(w, h, tags, chunks, be=be))


def write_tif_planar_cmyk(path, w=23, h=13):
    """CMYK in four planes."""
    planes = [bytes((x * (5 + 3 * c) + y * (11 - 2 * c)) & 0xff for y in range(h) for x in range(w))
              for c in range(4)]
    tags = {258: (3, [8] * 4), 259: (3, [1]), 262: (3, [5]), 277: (3, [4]), 278: (4, [h]),
            284: (3, [2])}
    open(path, "wb").write(tiff_bytes(w, h, tags, planes))


def write_tif_graya(path, planar, w=23, h=13):
    """Gray with associated alpha, contiguous or in two planes."""
    gray = [(x * 11 + y * 7) & 0xff for y in range(h) for x in range(w)]
    alpha = [(x * 255 // (w - 1)) if y % 2 else 255 - y * 9 for y in range(h) for x in range(w)]
    tags = {258: (3, [8, 8]), 259: (3, [1]), 262: (3, [1]), 277: (3, [2]), 278: (4, [h]),
            338: (3, [1])}
    if planar:
        tags[284] = (3, [2])
        chunks = [bytes(gray), bytes(alpha)]
    else:
        chunks = [bytes(v for pair in zip(gray, alpha) for v in pair)]
    open(path, "wb").write(tiff_bytes(w, h, tags, chunks))


def write_tif_tiled_gray(path, w=40, h=13, tw=32, th=32):
    """Uncompressed 8-bit gray in 32x32 tiles (padded past the image; libtiff
    takes uncompressed tiles of a multiple of 1024 bytes only)."""
    tiles = []
    for ty in range(0, h, th):
        for tx in range(0, w, tw):
            tiles.append(bytes(((x * 9 + y * 5) & 0xff) if x < w and y < h else 0x77
                               for y in range(ty, ty + th) for x in range(tx, tx + tw)))
    tags = {258: (3, [8]), 259: (3, [1]), 262: (3, [1]), 277: (3, [1]), 322: (4, [tw]),
            323: (4, [th])}
    open(path, "wb").write(tiff_bytes(w, h, tags, tiles, be=True, tiled=True))


def write_tif_broken(p):
    """Directories libtiff (or the RGBA reader) rejects, or reads in
    spite of what's wrong with them."""
    w, h = 23, 13
    gray = bytes((x * 11 + y * 7) & 0xff for y in range(h) for x in range(w))
    base = {258: (3, [8]), 259: (3, [1]), 262: (3, [1]), 277: (3, [1]), 278: (4, [h])}

    def write(name, tags, chunks=None, **kw):
        open(p(name), "wb").write(tiff_bytes(w, h, tags, chunks or [gray], **kw))

    # No ImageLength
    write("tif_no_length.tif", base | {257: None})
    # A strip past the end of the file, and a short one
    write("tif_strip_past_end.tif", base | {273: (4, [100000]), 279: (4, [w * h])})
    write("tif_strip_short.tif", base | {279: (4, [w * 5 + 3])})
    # No strip byte counts (estimated), and no offsets
    write("tif_no_bytecounts.tif", base | {279: None})
    # Unsupported bits per sample, photometric interpretations, extra
    # samples, and an RGB with two channels
    write("tif_bps3.tif", base | {258: (3, [3])})
    write("tif_photometric_icclab.tif", base | {262: (3, [9])})
    write("tif_logl_nocomp.tif", base | {262: (3, [32844])})
    write("tif_rgb_2spp.tif", base | {258: (3, [8, 8]), 262: (3, [2]), 277: (3, [2])},
          [gray + gray])
    write("tif_no_photometric.tif", {k: v for k, v in base.items() if k != 262})
    # Zero rows per strip, zero width
    write("tif_rps0.tif", base | {278: (4, [0])})
    write("tif_width0.tif", base | {256: (4, [0])})
    # A palette without a colormap
    write("tif_pal_nocmap.tif", base | {262: (3, [3])})
    # A colormap of 8-bit values (the "old style" libtiff warns about)
    write("tif_pal_old.tif", base | {262: (3, [3]), 320: (3, [i for i in range(256)] * 3)})
    # A huge tile size
    write("tif_tile_huge.tif", base | {322: (4, [0x40000000]), 323: (4, [16])},
          [gray], tiled=True)
    # MDI's byte order mark (which IMG_isTIF() doesn't take)
    data = bytearray(tiff_bytes(w, h, base, [gray]))
    data[0:2] = b"EP"
    open(p("tif_mdi.tif"), "wb").write(bytes(data))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__ if __doc__ else "usage: gen_sdl_image_testdata.py OUTDIR")
    main(sys.argv[1])
