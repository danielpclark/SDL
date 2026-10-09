#!/usr/bin/env python3
# Generate the JPEG XL test images of sdl3-image/src/testdata/images/ (the
# jxl_*.jxl files) for the translation of libjxl's decoder
# (sdl3-image/src/jxl/).
#
# Usage: CJXL=/path/to/cjxl [CJXL_DEV=/path/to/cjxl] \
#        tools/gen_jxl_testdata.py [OUTDIR]
#
# cjxl is libjxl's encoder, built from the libjxl SDL_image's
# external/libjxl pins (libjxl 0.7.3). The inputs are small synthetic PNG,
# APNG, PFM and PAM pictures made here, and JPEGs made from them with
# ImageMagick (`convert`). The outputs cover lossless (modular, at several
# efforts, with palettes, squeeze and modular group sizes) and lossy
# (VarDCT at several distances and efforts, lossy modular), gray, RGB,
# alpha (straight and premultiplied), 1-, 12- and 16-bit and float
# samples, patches and dots, photon noise, resampling, the loop filters,
# all the orientations (from PNG eXIf chunks, which also make containers
# with an Exif box), ICC profiles and enumerated color encodings (linear,
# PQ, HLG, DCI-P3, BT.709 and gamma transfer functions), DC frames,
# recompressed JPEGs (YCbCr with chroma subsampling), images of several
# groups, and animations (with offset and blended frames, of which
# SDL_image loads the last).
#
# cjxl 0.7 writes no splines (FindSplines() is a stub) and no progressive
# AC passes (the encoder API leaves them as a TODO): jxl_splines.jxl and
# the jxl_progressive_*.jxl files with AC passes are made with $CJXL_DEV,
# a cjxl built from the same libjxl with two changes, and are left as they
# are when it isn't set. In lib/jxl/enc_splines.cc, FindSplines() returns
# one spline instead of none: control points (2, 3), (30, 20), (10, 35),
# (45, 38); color DCTs X {0.2, 0.1}, Y {0.5, 0, 0.3}, B {-0.02, 0.4, 0.2};
# sigma DCT {0.9, 0, 0, 0, 0, 0, 0, 0.6}; all other coefficients 0;
# quantization adjustment 0, YtoX and YtoB 0 (it is called at efforts 7
# and above only, so the progressive files are made at effort 6). In
# lib/jxl/encode.cc, the frame encoder's progressive splitter is set up as
# EncodeFile() does it: for qprogressive_ac (cjxl -p) the passes of
# progressive_passes_dc_quant_ac_full_ac, for progressive_ac those of
# progressive_passes_dc_lf_salient_ac with a saliency threshold of 0.
#
# The files are checked in, since encoders aren't deterministic across
# versions; the expected results in testdata/reference.txt come from
# upstream SDL_image's C with libjxl (built for Highway's scalar target),
# not from this script.

import os
import struct
import subprocess
import sys
import tempfile
import zlib


def pattern(w, h, seed, maxval, noise=24):
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


def glyphs(w, h):
    # dark glyphs repeated on a light background (patches and dots)
    shapes = [
        [".XXX.", "X...X", "XXXXX", "X...X", "X...X"],
        ["XXXX.", "X...X", "XXXX.", "X...X", "XXXX."],
        [".XXXX", "X....", "X....", "X....", ".XXXX"],
    ]
    planes = [[235] * (w * h) for _ in range(3)]
    k = 0
    for gy in range(2, h - 6, 8):
        for gx in range(2, w - 6, 7):
            s = shapes[k % 3]
            k += 1
            for j in range(5):
                for i in range(5):
                    if s[j][i] == "X":
                        for c in range(3):
                            planes[c][(gy + j) * w + gx + i] = 20 + 30 * c
    # a few isolated dots
    for n in range(6):
        x, y = 3 + n * 13 % (w - 6), h - 3 - n % 2
        for c in range(3):
            planes[c][y * w + x] = 40
    return planes


def chunk(t, d):
    c = struct.pack(">I", len(d)) + t + d
    return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)


def png_raw(w, h, planes, depth):
    raw = b""
    for j in range(h):
        row = []
        for i in range(w):
            row += [p[j * w + i] for p in planes]
        raw += b"\0" + (struct.pack(">%dH" % len(row), *row) if depth == 16 else bytes(row))
    return zlib.compress(raw, 9)


def png(path, w, h, planes, depth=8, extra_chunks=()):
    # planes: gray, gray+alpha, RGB or RGBA
    color_type = {1: 0, 2: 4, 3: 2, 4: 6}[len(planes)]
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, color_type, 0, 0, 0)))
        for c in extra_chunks:
            f.write(c)
        f.write(chunk(b"IDAT", png_raw(w, h, planes, depth)))
        f.write(chunk(b"IEND", b""))


def apng(path, w, h, frames, loops=0):
    # frames: (planes RGBA, fw, fh, x, y, delay_num, delay_den, dispose, blend)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)))
        f.write(chunk(b"acTL", struct.pack(">II", len(frames), loops)))
        seq = 0
        for n, (planes, fw, fh, x, y, dn, dd, dispose, blend) in enumerate(frames):
            f.write(chunk(b"fcTL", struct.pack(">IIIIIHHBB", seq, fw, fh, x, y, dn, dd, dispose,
                                               blend)))
            seq += 1
            data = png_raw(fw, fh, planes, 8)
            if n == 0:
                f.write(chunk(b"IDAT", data))
            else:
                f.write(chunk(b"fdAT", struct.pack(">I", seq) + data))
                seq += 1
        f.write(chunk(b"IEND", b""))


def exif_orientation(orientation):
    # a big-endian TIFF header with one IFD entry, the orientation
    return chunk(b"eXIf", b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01" +
                 struct.pack(">H", orientation) + b"\0\0\0\0\0\0")


def icc_profile():
    # an RGB display profile (v2) with BT.709-like primaries and a sampled
    # tone curve (not a pure gamma, so the encoder keeps the ICC profile)
    def s15(v):
        return struct.pack(">i", int(round(v * 65536)))

    def xyz(x, y, z):
        return b"XYZ \0\0\0\0" + s15(x) + s15(y) + s15(z)

    curve = [int(65535 * (i / 15.0) ** 2.1 * (0.9 + 0.1 * (i % 2))) for i in range(16)]
    curve[-1] = 65535
    curv = b"curv\0\0\0\0" + struct.pack(">I", len(curve)) + struct.pack(">%dH" % len(curve), *curve)
    curv += b"\0" * (-len(curv) % 4)
    desc_text = b"sdl3-image test\0"
    desc = (b"desc\0\0\0\0" + struct.pack(">I", len(desc_text)) + desc_text +
            b"\0" * 8 + b"\0" * 3 + b"\0" * 67)
    desc += b"\0" * (-len(desc) % 4)
    cprt = b"text\0\0\0\0" + b"No copyright\0"
    cprt += b"\0" * (-len(cprt) % 4)
    tags = [
        (b"desc", desc), (b"cprt", cprt), (b"wtpt", xyz(0.9505, 1.0, 1.089)),
        (b"rXYZ", xyz(0.4361, 0.2225, 0.0139)), (b"gXYZ", xyz(0.3851, 0.7169, 0.0971)),
        (b"bXYZ", xyz(0.1431, 0.0606, 0.7141)), (b"rTRC", curv), (b"gTRC", curv),
        (b"bTRC", curv),
    ]
    offset = 128 + 4 + 12 * len(tags)
    table = struct.pack(">I", len(tags))
    data = b""
    shared = {}
    for sig, body in tags:
        if body in shared:
            table += sig + struct.pack(">II", shared[body], len(body))
            continue
        shared[body] = offset + len(data)
        table += sig + struct.pack(">II", offset + len(data), len(body))
        data += body
    size = 128 + len(table) + len(data)
    header = (struct.pack(">I", size) + b"lcms" + b"\x02\x10\0\0" + b"mntrRGB XYZ " +
              struct.pack(">HHHHHH", 2020, 1, 1, 0, 0, 0) + b"acsp" + b"APPL" +
              struct.pack(">I", 0) + b"\0" * 8 + b"\0" * 8 + struct.pack(">I", 0) +
              s15(0.9642) + s15(1.0) + s15(0.8249) + b"lcms")
    header += b"\0" * (128 - len(header))
    return header + table + data


def iccp(profile):
    return chunk(b"iCCP", b"test\0\0" + zlib.compress(profile, 9))


def pfm(path, w, h, planes):
    # float RGB, bottom row first; values from the 8-bit planes, some above 1
    with open(path, "wb") as f:
        f.write(b"PF\n%d %d\n-1.0\n" % (w, h))
        for j in reversed(range(h)):
            for i in range(w):
                f.write(struct.pack("<3f", *[p[j * w + i] / 200.0 for p in planes]))


def pam_bw(path, w, h):
    # a 1-bit gray picture (BLACKANDWHITE)
    with open(path, "wb") as f:
        f.write(b"P7\nWIDTH %d\nHEIGHT %d\nDEPTH 1\nMAXVAL 1\nTUPLTYPE BLACKANDWHITE\nENDHDR\n"
                % (w, h))
        f.write(bytes(((i * 3 + j * 5) // 7) % 2 for j in range(h) for i in range(w)))


def main(out):
    cjxl = os.environ.get("CJXL", "cjxl")
    cjxl_dev = os.environ.get("CJXL_DEV")
    os.makedirs(out, exist_ok=True)
    tmp = tempfile.mkdtemp()
    t = lambda name: os.path.join(tmp, name)

    def enc(name, src, *args, tool=None):
        cmd = [tool or cjxl, src, os.path.join(out, name), "--num_threads=0", "--quiet", *args]
        subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)

    # inputs: RGB, RGBA, gray, gray+alpha, 16 bits, odd sizes
    rgb = pattern(40, 30, 1, 255)
    png(t("rgb.png"), 40, 30, rgb)
    png(t("rgba.png"), 37, 21, pattern(37, 21, 2, 255) + [alpha_pattern(37, 21, 255)])
    png(t("gray.png"), 33, 19, pattern(33, 19, 3, 255)[:1])
    png(t("graya.png"), 26, 18, pattern(26, 18, 4, 255)[:1] + [alpha_pattern(26, 18, 255)])
    png(t("rgb16.png"), 29, 17, pattern(29, 17, 5, 65535), depth=16)
    png(t("rgba16.png"), 24, 16, pattern(24, 16, 6, 65535) + [alpha_pattern(24, 16, 65535)],
        depth=16)
    png(t("rgb12.png"), 21, 13, pattern(21, 13, 7, 4095), depth=16)
    few = [[(v // 64) * 85 for v in p] for p in pattern(31, 23, 8, 255, noise=0)]
    png(t("few.png"), 31, 23, few)
    png(t("tiny.png"), 1, 1, [[200], [30], [90]])
    png(t("tall.png"), 3, 70, pattern(3, 70, 9, 255))
    png(t("glyphs.png"), 96, 64, glyphs(96, 64))
    png(t("wide.png"), 300, 40, pattern(300, 40, 10, 255))
    png(t("big.png"), 272, 264, pattern(272, 264, 11, 255, noise=8))

    # lossless (modular): efforts, palette, squeeze, group size, gray, alpha
    enc("jxl_lossless.jxl", t("rgb.png"), "-d", "0")
    enc("jxl_lossless_e1.jxl", t("rgb.png"), "-d", "0", "-e", "1")
    enc("jxl_lossless_e3.jxl", t("rgb.png"), "-d", "0", "-e", "3")
    enc("jxl_lossless_e9.jxl", t("few.png"), "-d", "0", "-e", "9")
    enc("jxl_lossless_palette.jxl", t("few.png"), "-d", "0", "--modular_palette_colors=64")
    enc("jxl_lossless_squeeze.jxl", t("rgb.png"), "-d", "0", "-R", "1")
    enc("jxl_lossless_groups.jxl", t("wide.png"), "-d", "0", "-g", "0", "-e", "2")
    enc("jxl_lossless_gray.jxl", t("gray.png"), "-d", "0")
    enc("jxl_lossless_alpha.jxl", t("rgba.png"), "-d", "0")
    enc("jxl_lossless_graya.jxl", t("graya.png"), "-d", "0")
    enc("jxl_lossless_16bit.jxl", t("rgb16.png"), "-d", "0")
    enc("jxl_lossless_16bit_alpha.jxl", t("rgba16.png"), "-d", "0")
    enc("jxl_lossless_12bit.jxl", t("rgb12.png"), "-d", "0", "--override_bitdepth=12")
    pam_bw(t("bw.pam"), 19, 11)
    enc("jxl_lossless_1bit.jxl", t("bw.pam"), "-d", "0")
    enc("jxl_lossless_tiny.jxl", t("tiny.png"), "-d", "0")
    enc("jxl_lossless_tall.jxl", t("tall.png"), "-d", "0")

    # lossy (VarDCT): distances, efforts, gray, alpha, 16 bits, filters
    enc("jxl_lossy.jxl", t("rgb.png"), "-d", "1")
    enc("jxl_lossy_d0.3_e9.jxl", t("rgb.png"), "-d", "0.3", "-e", "9")
    enc("jxl_lossy_d3_e3.jxl", t("rgb.png"), "-d", "3", "-e", "3")
    enc("jxl_lossy_d2_e5.jxl", t("rgb.png"), "-d", "2", "-e", "5")
    enc("jxl_lossy_d8.jxl", t("rgb.png"), "-d", "8")
    enc("jxl_lossy_d15_e8.jxl", t("big.png"), "-d", "15", "-e", "8")
    enc("jxl_lossy_resampling.jxl", t("rgb.png"), "-d", "1", "--resampling=4")
    enc("jxl_lossy_epf3.jxl", t("rgb.png"), "-d", "3", "--epf=3")
    enc("jxl_lossy_epf1.jxl", t("rgb.png"), "-d", "2", "--epf=1", "--gaborish=0")
    enc("jxl_lossy_nofilters.jxl", t("rgb.png"), "-d", "2", "--epf=0", "--gaborish=0")
    enc("jxl_lossy_gray.jxl", t("gray.png"), "-d", "1.5")
    enc("jxl_lossy_alpha.jxl", t("rgba.png"), "-d", "1.5")
    enc("jxl_lossy_alpha_premultiplied.jxl", t("rgba.png"), "-d", "1", "--premultiply=1")
    enc("jxl_lossy_alpha_resampled.jxl", t("rgba.png"), "-d", "4", "--ec_resampling=4")
    enc("jxl_lossy_graya.jxl", t("graya.png"), "-d", "1")
    enc("jxl_lossy_16bit.jxl", t("rgb16.png"), "-d", "1")
    enc("jxl_lossy_odd.jxl", t("tall.png"), "-d", "1")
    enc("jxl_lossy_tiny.jxl", t("tiny.png"), "-d", "1")
    enc("jxl_lossy_groups.jxl", t("big.png"), "-d", "2", "-e", "5")
    enc("jxl_lossy_modular.jxl", t("rgb.png"), "-m", "1", "-d", "2")
    enc("jxl_lossy_modular_alpha.jxl", t("rgba.png"), "-m", "1", "-d", "1")
    enc("jxl_lossy_faster_decoding.jxl", t("rgb.png"), "-d", "1", "--faster_decoding=4")

    # float samples
    pfm(t("float.pfm"), 22, 14, pattern(22, 14, 12, 255))
    enc("jxl_float_lossless.jxl", t("float.pfm"), "-d", "0")
    enc("jxl_float_lossy.jxl", t("float.pfm"), "-d", "1")

    # patches and dots, photon noise, splines
    enc("jxl_patches.jxl", t("glyphs.png"), "-d", "1", "--patches=1", "-e", "7")
    enc("jxl_patches_lossless.jxl", t("glyphs.png"), "-d", "0", "--patches=1", "-e", "7")
    enc("jxl_dots.jxl", t("glyphs.png"), "-d", "2", "--dots=1", "--patches=0", "-e", "7")
    enc("jxl_noise.jxl", t("rgb.png"), "-d", "1", "--photon_noise=ISO6400")
    enc("jxl_noise_alpha.jxl", t("rgba.png"), "-d", "2", "--photon_noise=ISO1600")
    if cjxl_dev:
        png(t("splines.png"), 48, 40, pattern(48, 40, 13, 255, noise=4))
        enc("jxl_splines.jxl", t("splines.png"), "-d", "1", "-e", "7", tool=cjxl_dev)

    # orientations (Exif in a container), lossless and lossy
    for o in range(2, 9):
        png(t("orient%d.png" % o), 13, 9, pattern(13, 9, 20 + o, 255), extra_chunks=[
            exif_orientation(o)])
        enc("jxl_orientation_%d.jxl" % o, t("orient%d.png" % o), "-d", "0")
    enc("jxl_orientation_6_lossy.jxl", t("orient6.png"), "-d", "1")
    png(t("orient5a.png"), 13, 9, pattern(13, 9, 30, 255) + [alpha_pattern(13, 9, 255)],
        extra_chunks=[exif_orientation(5)])
    enc("jxl_orientation_5_alpha.jxl", t("orient5a.png"), "-d", "1")

    # color encodings: ICC profiles, and enumerated ones
    png(t("icc.png"), 20, 15, pattern(20, 15, 14, 255), extra_chunks=[iccp(icc_profile())])
    enc("jxl_icc_lossless.jxl", t("icc.png"), "-d", "0")
    enc("jxl_icc_lossy.jxl", t("icc.png"), "-d", "1")
    for name, cs in [("linear", "RGB_D65_SRG_Rel_Lin"), ("pq", "RGB_D65_202_Rel_PeQ"),
                     ("hlg", "RGB_D65_202_Rel_HLG"), ("p3", "RGB_D65_DCI_Rel_SRG"),
                     ("709", "RGB_D65_SRG_Rel_709"), ("gamma", "RGB_D65_SRG_Rel_g0.45455"),
                     ("dci", "RGB_DCI_DCI_Rel_DCI")]:
        enc("jxl_enum_%s.jxl" % name, t("rgb.png"), "-d", "1", "-x", "color_space=" + cs)
    enc("jxl_enum_pq_lossless.jxl", t("rgb16.png"), "-d", "0", "-x",
        "color_space=RGB_D65_202_Rel_PeQ")
    enc("jxl_enum_gray_linear.jxl", t("gray.png"), "-d", "1", "-x",
        "color_space=Gra_D65_Rel_Lin")

    # progressive: AC passes, quantized AC passes (with alpha, and of
    # several groups), DC frames; containers, group order
    if cjxl_dev:
        enc("jxl_progressive_ac.jxl", t("rgb.png"), "-d", "1", "-e", "6", "--progressive_ac",
            tool=cjxl_dev)
        enc("jxl_progressive_qac.jxl", t("rgb.png"), "-d", "1", "-e", "6", "-p", tool=cjxl_dev)
        enc("jxl_progressive_qac_alpha.jxl", t("rgba.png"), "-d", "2", "-e", "6", "-p",
            tool=cjxl_dev)
        enc("jxl_progressive_ac_groups.jxl", t("big.png"), "-d", "3", "-e", "5",
            "--progressive_ac", tool=cjxl_dev)
    enc("jxl_progressive_dc.jxl", t("big.png"), "-d", "2", "--progressive_dc=1")
    enc("jxl_progressive_dc2.jxl", t("big.png"), "-d", "3", "--progressive_dc=2", "-e", "3")
    enc("jxl_container.jxl", t("rgb.png"), "-d", "1", "--container=1")
    enc("jxl_group_order.jxl", t("big.png"), "-d", "4", "--group_order=1", "-e", "3")

    # recompressed JPEGs: 4:2:0, 4:4:4, gray, progressive
    subprocess.run(["convert", t("rgb.png"), "-quality", "80", "-sampling-factor", "2x2",
                    t("j420.jpg")], check=True)
    subprocess.run(["convert", t("rgb.png"), "-quality", "92", "-sampling-factor", "1x1",
                    t("j444.jpg")], check=True)
    subprocess.run(["convert", t("rgb.png"), "-quality", "85", "-sampling-factor", "2x1",
                    t("j422.jpg")], check=True)
    subprocess.run(["convert", t("gray.png"), "-quality", "85", t("jgray.jpg")], check=True)
    subprocess.run(["convert", t("tall.png"), "-quality", "70", "-interlace", "JPEG",
                    t("jprog.jpg")], check=True)
    enc("jxl_jpeg_420.jxl", t("j420.jpg"), "-j", "1")
    enc("jxl_jpeg_444.jxl", t("j444.jpg"), "-j", "1")
    enc("jxl_jpeg_422.jxl", t("j422.jpg"), "-j", "1")
    enc("jxl_jpeg_gray.jxl", t("jgray.jpg"), "-j", "1")
    enc("jxl_jpeg_progressive.jxl", t("jprog.jpg"), "-j", "1")

    # animations: full frames, then an offset frame blended over the
    # previous one and one replacing a region; lossless and lossy
    w, h = 24, 16
    full = lambda seed: pattern(w, h, seed, 255) + [[255] * (w * h)]
    part = pattern(10, 6, 41, 255) + [alpha_pattern(10, 6, 255)]
    part2 = pattern(8, 8, 42, 255) + [[255] * 64]
    frames = [
        (full(40), w, h, 0, 0, 1, 10, 0, 0),
        (part, 10, 6, 5, 3, 3, 20, 0, 1),
        (part2, 8, 8, 12, 6, 7, 100, 1, 0),
        (full(43), w, h, 0, 0, 2, 10, 0, 0),
    ]
    apng(t("anim.png"), w, h, frames, loops=3)
    enc("jxl_anim.jxl", t("anim.png"), "-d", "0")
    enc("jxl_anim_lossy.jxl", t("anim.png"), "-d", "1")
    apng(t("anim2.png"), w, h, frames[:3])
    enc("jxl_anim_blend.jxl", t("anim2.png"), "-d", "1.5", "-e", "3")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else
         os.path.join(os.path.dirname(__file__), "..", "sdl3-image", "src", "testdata", "images"))
