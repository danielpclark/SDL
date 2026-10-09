#!/usr/bin/env python3
# Make the test fonts of sdl3-ttf/src/testdata/fonts/: subsets of DejaVu
# fonts (version 2.37, as Debian's fonts-dejavu-core ships them in
# /usr/share/fonts/truetype/dejavu/, or in the directory $DEJAVU_DIR) with
# fontTools' subsetter (`pip install fonttools`), and OpenType/CFF
# versions of the DejaVu Sans subset, hinted with the AFDKO's otfautohint
# (`pip install afdko`).
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
# The CFF versions of DejaVu Sans run FreeType's CFF driver and its Adobe
# CFF engine: DejaVuSans-CFF.otf has the subset's outlines (its quadratic
# curves made cubic) as CFF charstrings, with blue zones and standard stem
# widths measured from its glyphs and otfautohint's stem hints, hint
# replacement and flex; DejaVuSans.cff is its bare `CFF ' table (a
# PostScript font that is not an SFNT, whose Unicode charmap FreeType makes
# from its glyph names); and DejaVuSans-CFF2.otf is a CFF2 variable font
# with a width axis, whose masters are the CFF font and a copy 12% wider,
# and three named instances.
#
# The expected results in sdl3-ttf/src/testdata/reference.txt come from
# upstream SDL_ttf's C (with its bundled FreeType, without HarfBuzz and
# PlutoSVG), not from this script.

import os
import sys
import tempfile

from fontTools import subset
from fontTools.designspaceLib import (AxisDescriptor, DesignSpaceDocument,
                                      InstanceDescriptor, SourceDescriptor)
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.t2CharStringPen import T2CharStringPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools import varLib

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
    make_cff_fonts(out)


def bounds(glyph_set, name):
    pen = BoundsPen(glyph_set)
    glyph_set[name].draw(pen)
    return pen.bounds


def to_cff(path, xscale=1.0, descender_zone=True):
    """An OpenType/CFF version of the TrueType font `path`, its outlines
    and advance widths scaled horizontally by `xscale`."""
    font = TTFont(path)
    glyph_set = font.getGlyphSet()
    hmtx = font["hmtx"]
    char_strings = {}
    for name in font.getGlyphOrder():
        width = round(hmtx[name][0] * xscale)
        pen = T2CharStringPen(width, glyph_set)
        glyph_set[name].draw(TransformPen(pen, (xscale, 0, 0, 1, 0, 0)))
        char_strings[name] = pen.getCharString()
        hmtx[name] = (width, hmtx[name][1])

    # the blue zones and standard stems, from the glyphs
    b = {g: bounds(glyph_set, g) for g in ["o", "x", "H", "O", "l", "p", "hyphen"]}
    overshoot = b["O"][3] - b["H"][3]
    private = {
        "BlueValues": [b["o"][1], 0, b["x"][3], b["o"][3], b["H"][3], b["O"][3],
                       b["l"][3], b["l"][3] + overshoot],
        "StdHW": b["hyphen"][3] - b["hyphen"][1],
        "StdVW": round((b["l"][2] - b["l"][0]) * xscale),
    }
    if descender_zone:
        private["OtherBlues"] = [b["p"][1], b["p"][1]]
    private["StemSnapH"] = [private["StdHW"]]
    private["StemSnapV"] = [private["StdVW"]]

    name = font["name"]
    info = {
        "version": name.getDebugName(5),
        "Notice": name.getDebugName(0),
        "FullName": name.getDebugName(4),
        "FamilyName": name.getDebugName(1),
        "Weight": name.getDebugName(2),
    }
    ps_name = name.getDebugName(6)

    for tag in ["glyf", "loca", "fpgm", "prep", "cvt ", "gasp", "hdmx", "LTSH", "VDMX"]:
        if tag in font:
            del font[tag]
    font["maxp"].tableVersion = 0x00005000
    font["post"].formatType = 3.0
    builder = FontBuilder(font=font)
    builder.setupCFF(ps_name, info, char_strings, private)
    return font


def autohint(path, tmp):
    from afdko.otfautohint.__main__ import main as otfautohint

    if not otfautohint([path, "-o", path]):
        return
    # otfautohint 5.0.1 fails on a few glyphs of the variable font (a check
    # of its ghost hints): leave those unhinted
    one = os.path.join(tmp, "one.otf")
    failing = [name for name in TTFont(path).getGlyphOrder()
               if otfautohint([path, "-o", one, "-g", name])]
    if otfautohint([path, "-o", path, "-x", ",".join(failing)]):
        raise RuntimeError("otfautohint failed on " + path)


def keep_timestamp(path, src):
    """Gives the font `path` the modification time of the font `src`, so
    the output does not depend on when it was made."""
    font = TTFont(path, recalcTimestamp=False)
    font["head"].modified = TTFont(src)["head"].modified
    font.save(path)


def make_cff_fonts(out):
    src = os.path.join(out, "DejaVuSans.ttf")
    with tempfile.TemporaryDirectory() as tmp:
        # DejaVuSans-CFF.otf and DejaVuSans.cff
        cff_path = os.path.join(out, "DejaVuSans-CFF.otf")
        to_cff(src).save(cff_path)
        autohint(cff_path, tmp)
        keep_timestamp(cff_path, src)
        with open(os.path.join(out, "DejaVuSans.cff"), "wb") as f:
            f.write(TTFont(cff_path).reader["CFF "])

        # DejaVuSans-CFF2.otf
        doc = DesignSpaceDocument()
        axis = AxisDescriptor()
        axis.tag, axis.name = "wdth", "Width"
        axis.minimum, axis.default, axis.maximum = 100, 100, 112
        doc.addAxis(axis)
        for i, (xscale, width) in enumerate([(1.0, 100), (1.12, 112)]):
            master = os.path.join(tmp, "master%d.otf" % i)
            # (without the descender zone, whose ghost hints otfautohint
            # 5.0.1 fails on in variable fonts)
            to_cff(src, xscale, descender_zone=False).save(master)
            source = SourceDescriptor()
            source.path, source.location = master, {"Width": width}
            doc.addSource(source)
        for style, width in [("Regular", 100), ("SemiExpanded", 106), ("Expanded", 112)]:
            instance = InstanceDescriptor()
            instance.familyName, instance.styleName = "DejaVu Sans", style
            instance.location = {"Width": width}
            doc.addInstance(instance)
        # (DejaVu's OS/2 table is version 1, without the MVAR metrics)
        vf, _, _ = varLib.build(doc, exclude=["MVAR"])
        cff2_path = os.path.join(out, "DejaVuSans-CFF2.otf")
        vf.save(cff2_path)
        autohint(cff2_path, tmp)
        keep_timestamp(cff2_path, src)


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "sdl3-ttf", "src", "testdata", "fonts"))
