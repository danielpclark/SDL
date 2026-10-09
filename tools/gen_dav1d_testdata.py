#!/usr/bin/env python3
# Generate the AV1 test streams of sdl3-image/src/testdata/dav1d/ for the
# translation of dav1d (sdl3-image/src/dav1d/).
#
# Usage: tools/gen_dav1d_testdata.py [OUTDIR]
#
# The streams are small IVF files made with ffmpeg's AV1 encoders (libaom,
# SVT-AV1 and rav1e) from ffmpeg's synthetic sources and a few raw
# patterns made here, chosen to cover the bit depths (8, 10, 12), the
# chroma layouts (4:2:0, 4:2:2, 4:4:4, 4:0:0), still pictures (the
# reduced still picture header AVIF uses), intra and inter frames (hidden
# alt-refs, compound prediction, OBMC, warped and global motion,
# references of another size), film grain, CDEF, loop restoration and
# super-resolution on and off, palette and intra block copy, lossless
# coding, segmentation and delta q/lf, quantizer matrices, 128x128
# superblocks, tiles and odd sizes. Most frames span several superblock
# rows and columns. Encoders aren't deterministic across versions, so the
# streams are checked in; the expected results in
# testdata/dav1d/reference.txt come from upstream dav1d's C (decoding the
# streams and the truncated and corrupted variants the test derives from
# them), not from this script.

import os
import subprocess
import sys
import tempfile


def ffmpeg(out, src, frames, codec_args, pix_fmt="yuv420p", raw=None):
    args = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y"]
    if raw:
        size, raw_fmt = raw
        args += ["-f", "rawvideo", "-s", size, "-pix_fmt", raw_fmt, "-i", src]
    else:
        args += ["-f", "lavfi", "-i", src]
    args += ["-frames:v", str(frames), "-pix_fmt", pix_fmt, *codec_args, "-f", "ivf", out]
    subprocess.run(args, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def aom(*params, crf=40, cpu=6, extra=()):
    a = ["-c:v", "libaom-av1", "-cpu-used", str(cpu), "-crf", str(crf),
         "-threads", "1", "-row-mt", "0", *extra]
    if params:
        a += ["-aom-params", ":".join(params)]
    return a


def svt(params, crf=45, preset=10):
    return ["-c:v", "libsvtav1", "-preset", str(preset), "-crf", str(crf),
            "-svtav1-params", params]


def tile_pattern(path, w, h, tile):
    # a 4:2:0 frame of one pseudo-random gray tile repeated (for intra
    # block copy: exact copies that neither palette nor intra prediction
    # code well)
    x = 12345
    t = []
    for _ in range(tile * tile):
        x = (x * 1103515245 + 12345) & 0x7fffffff
        t.append(64 + ((x >> 16) & 127))
    y = bytes(t[(j % tile) * tile + (i % tile)] for j in range(h) for i in range(w))
    with open(path, "wb") as f:
        f.write(y + bytes([128]) * (w * h // 2))


def main(out):
    os.makedirs(out, exist_ok=True)
    p = lambda name: os.path.join(out, name + ".ivf")
    moving = "testsrc2=size=128x96:rate=10"
    small = "testsrc2=size=96x80:rate=10"

    # 8-bit 4:2:0: a key frame, still pictures, inter frames with hidden
    # alt-refs
    ffmpeg(p("i420_8bit_key"), moving, 1, aom(crf=30))
    ffmpeg(p("i420_8bit_still"), moving, 1, aom(crf=35, extra=("-still-picture", "1")))
    ffmpeg(p("i420_8bit_inter"), moving, 8,
           aom(crf=50, cpu=4, extra=("-lag-in-frames", "8", "-auto-alt-ref", "1")))
    # the coding tools of the inter frames (rotating noisy content for
    # warped motion, inter-intra, compound masks and weights, and the
    # self-guided restoration filters)
    tools = ("enable-global-motion=1", "enable-warped-motion=1", "enable-obmc=1",
             "enable-dual-filter=1", "enable-interintra-comp=1",
             "enable-smooth-interintra=1", "enable-interintra-wedge=1",
             "enable-masked-comp=1", "enable-diff-wtd-comp=1",
             "enable-dist-wtd-comp=1", "enable-interinter-wedge=1")
    rotating = "testsrc2=size=128x96:rate=10,rotate=a=t*0.3:c=black,noise=alls=12:allf=t"
    ffmpeg(p("i420_8bit_tools"), rotating, 6,
           aom(*tools, crf=40, cpu=1, extra=("-lag-in-frames", "6")))
    ffmpeg(p("i420_10bit_tools"), rotating, 4,
           aom(*tools, crf=45, cpu=1, extra=("-lag-in-frames", "4")),
           pix_fmt="yuv420p10le")
    # segmentation, delta q and delta lf
    ffmpeg(p("i420_8bit_segments"), moving, 4,
           aom("deltaq-mode=1", "delta-lf-mode=1", crf=45, cpu=4,
               extra=("-aq-mode", "1")))
    # CDEF and loop restoration off; quantizer matrices at an odd size
    ffmpeg(p("i420_8bit_nofilters"), moving, 3,
           aom("enable-cdef=0", "enable-restoration=0", crf=45))
    ffmpeg(p("i420_8bit_qm"), "testsrc2=size=99x67:rate=10", 3,
           aom("enable-qm=1", "qm-min=4", "qm-max=10", crf=35))
    # super-resolution, and references of another size (SVT-AV1:
    # libaom's are not settable through ffmpeg)
    ffmpeg(p("svt_i420_8bit_superres"), moving, 3,
           svt("superres-mode=1:superres-denom=12:superres-kf-denom=13", crf=45))
    ffmpeg(p("svt_i420_8bit_resize"), moving, 4,
           svt("resize-mode=1:resize-denom=12:resize-kf-denom=9", crf=50))
    # film grain (libaom's test parameter sets)
    ffmpeg(p("i420_8bit_grain"), moving, 3, aom("film-grain-test=1", crf=50))
    ffmpeg(p("i420_8bit_grain2"), small, 2, aom("film-grain-test=2", crf=50))
    ffmpeg(p("i444_8bit_grain"), small, 2, aom("film-grain-test=7", crf=50),
           pix_fmt="yuv444p")
    ffmpeg(p("i420_10bit_grain"), small, 2, aom("film-grain-test=3", crf=50),
           pix_fmt="yuv420p10le")
    ffmpeg(p("i422_12bit_grain"), small, 2, aom("film-grain-test=10", crf=50),
           pix_fmt="yuv422p12le")
    # palette (screen content) and intra block copy
    ffmpeg(p("i444_8bit_screen"), "testsrc=size=128x96:rate=1", 1,
           aom("tune-content=screen", crf=30, cpu=4,
               extra=("-enable-palette", "1")), pix_fmt="yuv444p")
    ffmpeg(p("i420_8bit_screen"), "smptebars=size=128x96:rate=1", 1,
           aom("tune-content=screen", crf=30, cpu=4,
               extra=("-enable-palette", "1")))
    with tempfile.TemporaryDirectory() as tmp:
        raw = os.path.join(tmp, "tiles.yuv")
        tile_pattern(raw, 256, 192, 32)
        ffmpeg(p("i420_8bit_intrabc"), raw, 1,
               aom("tune-content=screen", crf=50, cpu=2,
                   extra=("-enable-intrabc", "1")),
               raw=("256x192", "yuv420p"))
    # lossless (Walsh-Hadamard transform)
    ffmpeg(p("i420_8bit_lossless"), "testsrc2=size=48x32:rate=1", 1,
           aom("lossless=1", crf=0))
    # tiles and 128x128 superblocks
    ffmpeg(p("i420_8bit_tiles"), "testsrc2=size=256x144:rate=10", 2,
           aom("tile-columns=1", "tile-rows=1", crf=50))
    ffmpeg(p("i420_8bit_sb128"), "testsrc2=size=192x160:rate=10", 2,
           aom("sb-size=128", crf=50))

    # other layouts and bit depths
    ffmpeg(p("i444_8bit"), small, 3, aom(crf=45), pix_fmt="yuv444p")
    ffmpeg(p("i422_8bit"), small, 2, aom(crf=45), pix_fmt="yuv422p")
    ffmpeg(p("i400_8bit"), small, 3, aom(crf=45), pix_fmt="gray")
    ffmpeg(p("i420_10bit"), moving, 4, aom(crf=45, cpu=4), pix_fmt="yuv420p10le")
    ffmpeg(p("i420_10bit_still"), small, 1, aom(crf=35, extra=("-still-picture", "1")),
           pix_fmt="yuv420p10le")
    ffmpeg(p("i422_10bit"), small, 2, aom(crf=45), pix_fmt="yuv422p10le")
    ffmpeg(p("i444_10bit"), small, 2, aom(crf=45), pix_fmt="yuv444p10le")
    ffmpeg(p("i400_10bit"), small, 2, aom(crf=45), pix_fmt="gray10le")
    ffmpeg(p("i420_12bit"), small, 3, aom(crf=45), pix_fmt="yuv420p12le")
    ffmpeg(p("i444_12bit"), small, 2, aom(crf=45), pix_fmt="yuv444p12le")
    ffmpeg(p("i400_12bit"), small, 1, aom(crf=40), pix_fmt="gray12le")

    # other encoders
    ffmpeg(p("rav1e_i420_8bit"), moving, 4,
           ["-c:v", "librav1e", "-speed", "10", "-qp", "150",
            "-rav1e-params", "threads=1"])
    ffmpeg(p("svt_i420_10bit"), moving, 3, svt("tune=0", crf=50),
           pix_fmt="yuv420p10le")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "sdl3-image/src/testdata/dav1d")
