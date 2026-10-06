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


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__ if __doc__ else "usage: gen_sdl_image_testdata.py OUTDIR")
    main(sys.argv[1])
