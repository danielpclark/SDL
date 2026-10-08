#!/usr/bin/env python3
# Make the test fonts of sdl3-ttf/src/testdata/fonts/: subsets of DejaVu
# fonts (version 2.37, as Debian's fonts-dejavu-core ships them in
# /usr/share/fonts/truetype/dejavu/, or in the directory $DEJAVU_DIR) with
# fontTools' subsetter (`pip install fonttools`).
#
# Usage: tools/gen_sdl_ttf_testdata.py [OUTDIR]
#
# The subsets keep Basic Latin and Latin-1 (and, in the monospaced font
# only, a few more characters that the fallback font test looks for), with
# the TrueType hinting instructions, the 'kern' table and the (English)
# names and copyright and license notices, so the hinting, kerning and
# metrics code paths all run. They keep their DejaVu names, which the
# Bitstream Vera license (sdl3-ttf/src/testdata/fonts/LICENSE) allows for
# modified fonts: only names with "Bitstream" or "Vera" in them are not.
#
# The expected results in sdl3-ttf/src/testdata/reference.txt come from
# upstream SDL_ttf's C (with its bundled FreeType, without HarfBuzz and
# PlutoSVG), not from this script.

import os
import sys

from fontTools import subset

FONTS = [
    # (source, output, extra code points)
    ("DejaVuSans.ttf", "DejaVuSans.ttf", []),
    ("DejaVuSerif-Bold.ttf", "DejaVuSerif-Bold.ttf", []),
    ("DejaVuSansMono.ttf", "DejaVuSansMono.ttf", [0x20AC, 0x2026, 0x2190, 0x263A]),
]


def main(out):
    src = os.environ.get("DEJAVU_DIR", "/usr/share/fonts/truetype/dejavu")
    os.makedirs(out, exist_ok=True)
    for name, outname, extra in FONTS:
        options = subset.Options()
        options.hinting = True
        options.legacy_kern = True
        # the copyright, names, version, license and its URL
        options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
        options.notdef_outline = True
        options.glyph_names = False
        options.drop_tables += ["GPOS", "GSUB", "GDEF", "FFTM", "MATH"]
        options.layout_features = []
        font = subset.load_font(os.path.join(src, name), options)
        unicodes = list(range(0x20, 0x7F)) + list(range(0xA0, 0x100)) + extra
        subsetter = subset.Subsetter(options)
        subsetter.populate(unicodes=unicodes)
        subsetter.subset(font)
        subset.save_font(font, os.path.join(out, outname), options)


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "sdl3-ttf", "src", "testdata", "fonts"))
