#!/usr/bin/env python3
# Make the test fonts of sdl3-ttf/src/testdata/fonts/: subsets of DejaVu
# fonts (version 2.37, as Debian's fonts-dejavu-core ships them in
# /usr/share/fonts/truetype/dejavu/, or in the directory $DEJAVU_DIR) with
# fontTools' subsetter (`pip install fonttools`), and OpenType/CFF
# versions of the DejaVu Sans subset, hinted with the AFDKO's otfautohint
# (`pip install afdko`).
#
# Usage: tools/gen_sdl_ttf_testdata.py [--shaping | --formats] [OUTDIR]
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
# The fonts of the shaping tests (make_shaping_fonts; only those with
# --shaping) are subsets of Noto fonts ($NOTO_DIR) keeping the characters
# of their cases in sdl3-ttf/src/testdata/shaping_cases.txt, with all
# their OpenType layout features, unhinted versions of three of them (the
# auto-hinter's fonts; Noto Sans KR's with TrueType outlines), and
# DejaVuSans-NoLayout.ttf, DejaVu Sans's Arabic and Hebrew without its
# OpenType layout tables.
#
# The expected results in sdl3-ttf/src/testdata/reference.txt and
# shaping_reference.txt come from upstream SDL_ttf's C (with its bundled
# FreeType and HarfBuzz, without PlutoSVG), not from this script.

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


# ---------------------------------------------------------------------
# The fonts of FreeType's other drivers (make_format_fonts), made from the
# DejaVu subsets above (so under their license, LICENSE):
#
# * bitmap fonts of DejaVu Sans (and DejaVu Serif Bold), rasterized by
#   this script's own scanline rasterizer (`rasterize`: the glyphs'
#   outlines sampled at pixel centers, or 4x4 times per pixel for the
#   16-level gray BDF font): DejaVuSans-13.bdf and DejaVuSans-13-4bpp.bdf
#   (BDF, written by `write_bdf`), DejaVuSans-13.pcf and
#   DejaVuSans-13-lsb.pcf (PCF, converted from the BDF font with X.Org's
#   bdftopcf, with its default and with least significant bit and byte
#   first layouts), DejaVuSans-13.pcf.gz, DejaVuSans-13.pcf.Z and
#   DejaVuSans-13.pcf.bz2 (the PCF font compressed with gzip, compress's
#   LZW -- `lzw_compress` -- and bzip2, which SDL_ttf's FreeType does not
#   read), DejaVuSans-13-nosize.pcf.gz (the gzipped font without its size
#   in the gzip trailer, which FreeType then reads as a stream), DejaVuSans-13.fnt (a Windows 3.0 FNT font, `fnt_data`),
#   DejaVuSans.fon and DejaVuSans-PE.fon (two FNT 2.0 fonts, DejaVu Sans
#   13 px and DejaVu Serif Bold 16 px, in a 16-bit NE and a 32-bit PE
#   resource DLL, `write_ne_fon` and `write_pe_fon`).

BITMAP_CHARS = list(range(0x20, 0x7F)) + list(range(0xA0, 0x100))


class _FlattenPen:
    """A pen collecting a glyph's contours as polylines (curves cut into
    16 line segments), in font units."""

    def __init__(self, glyph_set):
        from fontTools.pens.basePen import BasePen

        pen = self

        class Pen(BasePen):
            def _moveTo(self, pt):
                pen.contours.append([pt])

            def _lineTo(self, pt):
                pen.contours[-1].append(pt)

            def _curveToOne(self, p1, p2, p3):
                p0 = self._getCurrentPoint()
                for i in range(1, 17):
                    t = i / 16
                    u = 1 - t
                    pen.contours[-1].append(
                        (u * u * u * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0]
                         + t * t * t * p3[0],
                         u * u * u * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1]
                         + t * t * t * p3[1]))

            def _qCurveToOne(self, p1, p2):
                p0 = self._getCurrentPoint()
                for i in range(1, 17):
                    t = i / 16
                    u = 1 - t
                    pen.contours[-1].append((u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0],
                                             u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1]))

            def _closePath(self):
                pass

            _endPath = _closePath

        self.contours = []
        self.pen = Pen(glyph_set)


def rasterize(glyph_set, name, scale, levels):
    """The glyph `name` scaled by `scale` (pixels per font unit) as
    (x, y, width, height, rows): its bitmap's bottom left corner relative
    to the origin (y up) and its rows from the top, each a list of
    coverage values from 0 to `levels` - 1 (2 or 16 levels; with 16, the
    coverage of 4x4 samples per pixel, at most 15). The bitmap is cut to
    the inked pixels; a blank glyph is (0, 0, 0, 0, [])."""
    fp = _FlattenPen(glyph_set)
    glyph_set[name].draw(fp.pen)
    edges = []
    for c in fp.contours:
        pts = [(x * scale, y * scale) for x, y in c]
        for i in range(len(pts)):
            (x0, y0), (x1, y1) = pts[i], pts[(i + 1) % len(pts)]
            if y0 != y1:
                edges.append((x0, y0, x1, y1))
    if not edges:
        return (0, 0, 0, 0, [])
    sub = 1 if levels == 2 else 4
    xmin = int(min(min(e[0], e[2]) for e in edges)) - 1
    xmax = int(max(max(e[0], e[2]) for e in edges)) + 2
    ymin = int(min(min(e[1], e[3]) for e in edges)) - 1
    ymax = int(max(max(e[1], e[3]) for e in edges)) + 2
    w, h = xmax - xmin, ymax - ymin
    cov = [[0] * w for _ in range(h)]
    for row in range(h * sub):
        ys = ymin + (row + 0.5) / sub
        xs = []
        for x0, y0, x1, y1 in edges:
            if min(y0, y1) <= ys < max(y0, y1):
                xs.append((x0 + (ys - y0) * (x1 - x0) / (y1 - y0), 1 if y1 > y0 else -1))
        xs.sort()
        wind = 0
        for i, (x, d) in enumerate(xs):
            wind += d
            if wind != 0 and i + 1 < len(xs):
                # the samples from x to the next crossing are inside
                a, b = x, xs[i + 1][0]
                for col in range(max(0, int((a - xmin) * sub - 1)),
                                 min(w * sub, int((b - xmin) * sub + 1))):
                    xc = xmin + (col + 0.5) / sub
                    if a <= xc < b:
                        cov[row // sub][col // sub] += 1
    if levels == 2:
        pix = [[1 if v else 0 for v in r] for r in cov]
    else:
        pix = [[min(v, 15) for v in r] for r in cov]
    rows = [r for r in range(h) if any(pix[r])]
    cols = [c for c in range(w) if any(pix[r][c] for r in range(h))]
    if not rows:
        return (0, 0, 0, 0, [])
    r0, r1, c0, c1 = rows[0], rows[-1], cols[0], cols[-1]
    bitmap = [pix[r][c0:c1 + 1] for r in range(r1, r0 - 1, -1)]
    return (xmin + c0, ymin + r0, c1 - c0 + 1, r1 - r0 + 1, bitmap)


def bitmap_glyphs(path, ppem, levels=2):
    """The glyphs of BITMAP_CHARS of the TrueType font `path` at `ppem`
    pixels per em: (code, name, advance, swidth, raster) tuples, and the
    font's ascent, descent and information."""
    font = TTFont(path)
    glyph_set = font.getGlyphSet()
    cmap = font.getBestCmap()
    upem = font["head"].unitsPerEm
    scale = ppem / upem
    glyphs = []
    for code in BITMAP_CHARS:
        name = cmap[code]
        adv = font["hmtx"][name][0]
        glyphs.append((code, name, round(adv * scale), round(adv * 1000 / upem),
                       rasterize(glyph_set, name, scale, levels)))
    hhea = font["hhea"]
    ascent = -(-hhea.ascent * ppem // upem)
    descent = -(hhea.descent * ppem // upem)
    names = font["name"]
    info = {
        "family": names.getDebugName(1),
        "copyright": " ".join(names.getDebugName(0).split()),
        "weight": font["OS/2"].usWeightClass,
    }
    return glyphs, ascent, descent, info


def write_bdf(path, glyphs, ascent, descent, info, ppem, bpp=1):
    """A BDF 2.1 font of `glyphs`; with `bpp` 4, an anti-aliased one (the
    SIZE's fourth value, as Mark Leisher's xmbdfed writes them)."""
    inked = [g for g in glyphs if g[4][2]]
    bx0 = min(g[4][0] for g in inked)
    by0 = min(g[4][1] for g in inked)
    bx1 = max(g[4][0] + g[4][2] for g in inked)
    by1 = max(g[4][1] + g[4][3] for g in inked)
    weight = "Bold" if info["weight"] >= 700 else "Medium"
    family = info["family"]
    avg = round(sum(g[2] for g in glyphs) * 10 / len(glyphs))
    lines = [
        "STARTFONT 2.1",
        "COMMENT Rasterized from DejaVu outlines by tools/gen_sdl_ttf_testdata.py",
        "FONT -DejaVu-%s-%s-R-Normal--%d-%d-75-75-P-%d-ISO10646-1"
        % (family, weight, ppem, ppem * 10, avg),
        "SIZE %d 75 75%s" % (ppem, "" if bpp == 1 else " %d" % bpp),
        "FONTBOUNDINGBOX %d %d %d %d" % (bx1 - bx0, by1 - by0, bx0, by0),
    ]
    props = [
        ("FOUNDRY", '"DejaVu"'),
        ("FAMILY_NAME", '"%s"' % family),
        ("WEIGHT_NAME", '"%s"' % weight),
        ("SLANT", '"R"'),
        ("SETWIDTH_NAME", '"Normal"'),
        ("ADD_STYLE_NAME", '""'),
        ("PIXEL_SIZE", ppem),
        ("POINT_SIZE", ppem * 10),
        ("RESOLUTION_X", 75),
        ("RESOLUTION_Y", 75),
        ("SPACING", '"P"'),
        ("AVERAGE_WIDTH", avg),
        ("CHARSET_REGISTRY", '"ISO10646"'),
        ("CHARSET_ENCODING", '"1"'),
        ("FONT_ASCENT", ascent),
        ("FONT_DESCENT", descent),
        ("DEFAULT_CHAR", 32),
        ("COPYRIGHT", '"%s"' % info["copyright"]),
        ("NOTICE", '"Bitstream Vera license; see LICENSE"'),
    ]
    lines.append("STARTPROPERTIES %d" % len(props))
    lines += ["%s %s" % p for p in props]
    lines.append("ENDPROPERTIES")
    lines.append("CHARS %d" % len(glyphs))
    for code, name, adv, swidth, (x, y, w, h, rows) in glyphs:
        lines += ["STARTCHAR %s" % name, "ENCODING %d" % code,
                  "SWIDTH %d 0" % swidth, "DWIDTH %d 0" % adv,
                  "BBX %d %d %d %d" % (w, h, x, y), "BITMAP"]
        for r in rows:
            bits = "".join(("{:0%db}" % bpp).format(v) for v in r)
            bits += "0" * (-len(bits) % 8)
            lines.append("".join("%02X" % int(bits[i:i + 8], 2) for i in range(0, len(bits), 8)))
        lines.append("ENDCHAR")
    lines.append("ENDFONT")
    with open(path, "w", newline="\n") as f:
        f.write("\n".join(lines) + "\n")


def fnt_data(glyphs, ascent, descent, info, ppem, version):
    """A Windows FNT font (`version` 0x200 or 0x300) of `glyphs`: their
    cells (the advance width, at least 1, by the font height; glyphs
    reaching out of their cells are cut), the characters from 0x20 to
    0xFF, those with no glyph (0x7F to 0x9F) the space's."""
    import struct

    height = ascent + descent
    by_code = {g[0]: g for g in glyphs}
    first, last = 0x20, 0xFF
    cells = []
    for code in range(first, last + 1):
        g = by_code.get(code, by_code[0x20])
        adv = max(g[2], 1)
        x, y, w, h, rows = g[4]
        cell = [[0] * adv for _ in range(height)]
        for r in range(h):
            py = ascent - (y + h - 1 - r) - 1
            for c in range(w):
                px = x + c
                if 0 <= py < height and 0 <= px < adv and rows[r][c]:
                    cell[py][px] = 1
        cells.append((adv, cell))
    header_size = 118 if version == 0x200 else 148
    entry = 4 if version == 0x200 else 6
    table = header_size + entry * (len(cells) + 1)
    bits = bytearray()
    offsets = []
    for adv, cell in cells:
        offsets.append(table + len(bits))
        for col in range((adv + 7) // 8):
            for r in range(height):
                b = 0
                for i in range(8):
                    px = col * 8 + i
                    if px < adv and cell[r][px]:
                        b |= 0x80 >> i
                bits.append(b)
    face_offset = table + len(bits)
    name = info["family"].encode("ascii") + b"\0"
    size = face_offset + len(name)
    widths = [c[0] for c in cells]
    copyright = b"(c) 2003 Bitstream, Inc. DejaVu changes: public domain"
    h = struct.pack("<HI60sHHHHHHHBBBHBHHBHHBBBBHIIIIB", version, size, copyright, 0,
                    round(ppem * 72 / 96), 96, 96, ascent, height - ppem, 0, 0, 0, 0,
                    info["weight"], 0, 0, height, 0x20, round(sum(widths) / len(widths)),
                    max(widths), first, last, 0, 0, (max(widths) + 7) // 8 * 2, 0,
                    face_offset, 0, table, 0)
    if version == 0x300:
        h += struct.pack("<IHHHI16s", 0x10, 0, 0, 0, 0, b"")
    assert len(h) == header_size
    t = b""
    for adv, off in zip(widths + [0], offsets + [face_offset]):
        t += struct.pack("<HH" if version == 0x200 else "<HI", adv, off)
    data = h + t + bytes(bits) + name
    assert len(data) == size
    return data


def write_ne_fon(path, fnts):
    """A 16-bit (NE) resource DLL of the FNT fonts `fnts` (RT_FONT
    resources, after an RT_FONTDIR one), as Windows' .fon files."""
    import struct

    shift = 4
    lfanew = 0x80
    mz = bytearray(b"MZ" + bytes(lfanew - 2))
    struct.pack_into("<I", mz, 0x3C, lfanew)
    ne_size = 64
    # the resource table: the alignment shift, the RT_FONTDIR and RT_FONT
    # types with their entries, the end of the types, then the names
    res_len = 2 + (8 + 12) + (8 + 12 * len(fnts)) + 2
    rname = b"\x07FONTDIR\x00"
    data_start = lfanew + ne_size + res_len + len(rname)
    data_start = (data_start + 15) // 16 * 16
    fontdir = struct.pack("<H", len(fnts)) + b"".join(
        struct.pack("<H", i + 1) + f[:113] for i, f in enumerate(fnts))
    blobs = [fontdir] + list(fnts)
    offs = []
    pos = data_start
    for b in blobs:
        offs.append(pos)
        pos = (pos + len(b) + 15) // 16 * 16
    res = struct.pack("<H", shift)
    res += struct.pack("<HHI", 0x8007, 1, 0)
    res += struct.pack("<HHHHHH", offs[0] >> shift, (len(fontdir) + 15) >> shift, 0x0C50,
                       0x8001, 0, 0)
    res += struct.pack("<HHI", 0x8008, len(fnts), 0)
    for i, f in enumerate(fnts):
        res += struct.pack("<HHHHHH", offs[i + 1] >> shift, (len(f) + 15) >> shift, 0x1C30,
                           0x8001 + i, 0, 0)
    res += struct.pack("<H", 0)
    assert len(res) == res_len
    ne = bytearray(ne_size)
    struct.pack_into("<2sBB", ne, 0, b"NE", 5, 10)
    struct.pack_into("<HH", ne, 36, ne_size, ne_size + res_len)
    struct.pack_into("<HHHH", ne, 40, ne_size + res_len + len(rname),
                     ne_size + res_len + len(rname), 0, 0)
    struct.pack_into("<B", ne, 54, 2)
    out = bytearray(mz + ne + res + rname)
    for o, b in zip(offs, blobs):
        out += bytes(o - len(out)) + b
    out += bytes(-len(out) % 16)
    with open(path, "wb") as f:
        f.write(out)


def write_pe_fon(path, fnts):
    """A 32-bit (PE, i386) resource DLL of the FNT fonts `fnts`, with an
    RT_FONTDIR and the RT_FONT resources in its .rsrc section."""
    import struct

    va = 0x1000
    raw = 0x200
    # the resource tree: the root (types), the names, the languages
    types = [(7, [b"FONTDIR"]), (8, list(fnts))]
    tree = bytearray()
    leaves = []

    def directory(count):
        return struct.pack("<IIHHHH", 0, 0, 4, 0, 0, count) + bytes(8 * count)

    root = len(tree)
    tree += directory(len(types))
    for ti, (tid, items) in enumerate(types):
        name_dir = len(tree)
        struct.pack_into("<II", tree, root + 16 + 8 * ti, tid, 0x80000000 | name_dir)
        tree += directory(len(items))
        for ii, item in enumerate(items):
            lang_dir = len(tree)
            struct.pack_into("<II", tree, name_dir + 16 + 8 * ii, ii + 1,
                             0x80000000 | lang_dir)
            tree += directory(1)
            leaves.append((lang_dir + 16, item))
    entries = []
    for at, item in leaves:
        e = len(tree)
        struct.pack_into("<II", tree, at, 0x409, e)
        tree += bytes(16)
        entries.append((e, item))
    for e, item in entries:
        tree += bytes(-len(tree) % 16)
        struct.pack_into("<IIII", tree, e, va + len(tree), len(item), 0, 0)
        tree += item
    size = len(tree)
    tree += bytes(-len(tree) % raw)
    mz = bytearray(b"MZ" + bytes(62))
    struct.pack_into("<I", mz, 0x3C, 0x40)
    coff = struct.pack("<4sHHIIIHH", b"PE\0\0", 0x14C, 1, 0, 0, 0, 0xE0, 0x2102)
    opt = bytearray(0xE0)
    struct.pack_into("<HBB", opt, 0, 0x10B, 1, 0)
    struct.pack_into("<I", opt, 28, 0x10000000)
    struct.pack_into("<II", opt, 32, 0x1000, raw)
    struct.pack_into("<HH", opt, 40, 4, 0)
    struct.pack_into("<HH", opt, 48, 4, 0)
    struct.pack_into("<II", opt, 56, va + len(tree), raw)
    struct.pack_into("<I", opt, 92, 16)
    struct.pack_into("<II", opt, 96 + 16, va, size)
    section = struct.pack("<8sIIIIIIHHI", b".rsrc", size, va, len(tree), raw, 0, 0, 0, 0,
                          0x40000040)
    head = mz + coff + opt + section
    out = head + bytes(raw - len(head)) + tree
    with open(path, "wb") as f:
        f.write(out)


def lzw_compress(data):
    """`data` compressed as compress(1) does (.Z, codes of up to 16 bits,
    block mode, no CLEAR codes): the codes of each width in blocks of 8,
    a block cut short by a wider code padded to its full size."""
    out = bytearray(b"\x1f\x9d\x90")
    table = {bytes([i]): i for i in range(256)}
    next_code = 257
    codes = []
    w = b""
    for c in data:
        wc = w + bytes([c])
        if wc in table:
            w = wc
        else:
            codes.append(table[w])
            if next_code < 65536:
                table[wc] = next_code
                next_code += 1
            w = bytes([c])
    if w:
        codes.append(table[w])
    nbits = 9
    block = []

    def flush(full):
        acc = 0
        for i, code in enumerate(block):
            acc |= code << (i * nbits)
        n = nbits if full else (len(block) * nbits + 7) // 8
        out.extend(acc.to_bytes(nbits, "little")[:n])
        block.clear()

    for k, code in enumerate(codes):
        # (the decoder widens its codes when the next table entry,
        # 256 + k, would not fit)
        width = 9
        while k > 0 and 256 + k >= (1 << width) and width < 16:
            width += 1
        if width != nbits:
            if block:
                flush(True)
            nbits = width
        block.append(code)
        if len(block) == 8:
            flush(True)
    if block:
        flush(False)
    return bytes(out)


def make_bitmap_fonts(out):
    import bz2
    import gzip
    import subprocess

    sans = os.path.join(out, "DejaVuSans.ttf")
    serif = os.path.join(out, "DejaVuSerif-Bold.ttf")
    glyphs, ascent, descent, info = bitmap_glyphs(sans, 13)
    bdf = os.path.join(out, "DejaVuSans-13.bdf")
    write_bdf(bdf, glyphs, ascent, descent, info, 13)
    gray = bitmap_glyphs(sans, 13, 16)
    write_bdf(os.path.join(out, "DejaVuSans-13-4bpp.bdf"), *gray, 13, bpp=4)
    pcf = os.path.join(out, "DejaVuSans-13.pcf")
    subprocess.run(["bdftopcf", "-o", pcf, bdf], check=True)
    subprocess.run(["bdftopcf", "-l", "-L", "-p2", "-u4", "-o",
                    os.path.join(out, "DejaVuSans-13-lsb.pcf"), bdf], check=True)
    with open(pcf, "rb") as f:
        pcf_data = f.read()
    with open(pcf + ".gz", "wb") as f:
        with gzip.GzipFile(filename="", mode="wb", fileobj=f, mtime=0) as z:
            z.write(pcf_data)
    # (with its size in the trailer zeroed: FreeType then reads it through
    # its gzip stream rather than decompressing it into memory at once)
    with open(pcf + ".gz", "rb") as f:
        gz = f.read()
    with open(os.path.join(out, "DejaVuSans-13-nosize.pcf.gz"), "wb") as f:
        f.write(gz[:-4] + bytes(4))
    with open(pcf + ".Z", "wb") as f:
        f.write(lzw_compress(pcf_data))
    with open(pcf + ".bz2", "wb") as f:
        f.write(bz2.compress(pcf_data))

    with open(os.path.join(out, "DejaVuSans-13.fnt"), "wb") as f:
        f.write(fnt_data(glyphs, ascent, descent, info, 13, 0x300))
    bold = bitmap_glyphs(serif, 16)
    fnts = [fnt_data(glyphs, ascent, descent, info, 13, 0x200), fnt_data(*bold, 16, 0x200)]
    write_ne_fon(os.path.join(out, "DejaVuSans.fon"), fnts)
    write_pe_fon(os.path.join(out, "DejaVuSans-PE.fon"), fnts)


def make_format_fonts(out):
    make_bitmap_fonts(out)


SHAPING_FONTS = [
    # (source, output, extra code points); the sources are Noto fonts as
    # notofonts.github.io publishes them (fonts/NAME/hinted/ttf/NAME.ttf),
    # Noto Sans KR from noto-cjk (Sans/SubsetOTF/KR), in $NOTO_DIR
    ("NotoSans-Regular.ttf", "NotoSans-Regular.ttf", list(range(0x20, 0x7F))),
    ("NotoSansArabic-Regular.ttf", "NotoSansArabic-Regular.ttf", [0x25CC]),
    ("NotoSansHebrew-Regular.ttf", "NotoSansHebrew-Regular.ttf", [0x25CC]),
    ("NotoSansDevanagari-Regular.ttf", "NotoSansDevanagari-Regular.ttf", [0x25CC]),
    ("NotoSansThai-Regular.ttf", "NotoSansThai-Regular.ttf", [0x25CC]),
    ("NotoSansKR-Regular.otf", "NotoSansKR-Regular.otf", [0x25CC]),
    ("NotoSansKhmer-Regular.ttf", "NotoSansKhmer-Regular.ttf", [0x25CC]),
    ("NotoSansMyanmar-Regular.ttf", "NotoSansMyanmar-Regular.ttf", [0x25CC]),
    ("NotoSansSinhala-Regular.ttf", "NotoSansSinhala-Regular.ttf", [0x25CC]),
]


UNHINTED_FONTS = [
    # (hinted subset, unhinted version, with TrueType outlines): fonts
    # without hinting instructions, which FreeType's auto-hinter hints
    # (with HarfBuzz finding the glyphs of each style)
    ("NotoSans-Regular.ttf", "NotoSans-Unhinted.ttf", False),
    ("NotoSansArabic-Regular.ttf", "NotoSansArabic-Unhinted.ttf", False),
    ("NotoSansKR-Regular.otf", "NotoSansKR-Unhinted.ttf", True),
]


def make_unhinted(src, dst, to_truetype):
    """A copy of the font `src` without hinting (all its glyphs and
    OpenType layout features), with TrueType (quadratic) outlines if
    `to_truetype`."""
    options = subset.Options()
    options.hinting = False
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.notdef_outline = True
    options.glyph_names = False
    font = subset.load_font(src, options)
    subsetter = subset.Subsetter(options)
    subsetter.populate(gids=list(range(font["maxp"].numGlyphs)))
    subsetter.subset(font)
    if to_truetype:
        from fontTools.pens.cu2quPen import Cu2QuPen
        from fontTools.pens.ttGlyphPen import TTGlyphPen
        from fontTools.ttLib import newTable

        glyph_order = font.getGlyphOrder()
        glyph_set = font.getGlyphSet()
        glyf = newTable("glyf")
        glyf.glyphOrder = glyph_order
        glyf.glyphs = {}
        for name in glyph_order:
            pen = TTGlyphPen(glyph_set)
            glyph_set[name].draw(Cu2QuPen(pen, 1.0, reverse_direction=True))
            glyf.glyphs[name] = pen.glyph()
        font["glyf"] = glyf
        font["loca"] = newTable("loca")
        font["head"].indexToLocFormat = 0
        font["head"].glyphDataFormat = 0
        maxp = newTable("maxp")
        maxp.tableVersion = 0x00010000
        for k in ["maxZones", "maxTwilightPoints", "maxStorage", "maxFunctionDefs",
                  "maxInstructionDefs", "maxStackElements", "maxSizeOfInstructions",
                  "maxComponentElements", "maxPoints", "maxContours", "maxCompositePoints",
                  "maxCompositeContours", "maxComponentDepth"]:
            setattr(maxp, k, 0)
        maxp.maxZones = 1
        maxp.numGlyphs = len(glyph_order)
        font["maxp"] = maxp
        del font["CFF "]
        for tag in ["VORG"]:
            if tag in font:
                del font[tag]
        font["post"].formatType = 3.0
        font.sfntVersion = "\x00\x01\x00\x00"
        font.recalcBBoxes = True
    font.save(dst)


def shaping_texts(path):
    """The characters of the cases of each font of shaping_cases.txt."""
    texts = {}
    font = None
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if line.startswith("font "):
                font = line.split()[1]
                texts.setdefault(font, set())
            elif line.startswith("case\t"):
                text = line.split("\t", 5)[5]
                texts[font].update(ord(c) for c in text.replace("\\x", ""))
    return texts


def make_shaping_fonts(out):
    """The fonts of the shaping tests: subsets of Noto fonts (SIL Open
    Font License, sdl3-ttf/src/testdata/fonts/OFL.txt) with all their
    OpenType layout features and their hinting, keeping the characters of
    their cases (and the dotted circle that shapers insert), and
    DejaVuSans-NoLayout.ttf, DejaVu Sans's Arabic and Hebrew (with their
    presentation forms) without its OpenType layout tables, for
    HarfBuzz's fallback shaping and mark positioning."""
    noto = os.environ.get("NOTO_DIR", ".")
    texts = shaping_texts(os.path.join(out, "..", "shaping_cases.txt"))
    for hinted, unhinted, _ in UNHINTED_FONTS:
        texts[hinted] |= texts.get(unhinted, set())
    for name, outname, extra in SHAPING_FONTS:
        options = subset.Options()
        options.hinting = True
        options.layout_features = ["*"]
        options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
        options.notdef_outline = True
        options.glyph_names = False
        font = subset.load_font(os.path.join(noto, name), options)
        subsetter = subset.Subsetter(options)
        subsetter.populate(unicodes=sorted(texts[outname] | set(extra) | {0x20}))
        subsetter.subset(font)
        subset.save_font(font, os.path.join(out, outname), options)

    for hinted, unhinted, to_truetype in UNHINTED_FONTS:
        make_unhinted(os.path.join(out, hinted), os.path.join(out, unhinted), to_truetype)

    src = os.environ.get("DEJAVU_DIR", "/usr/share/fonts/truetype/dejavu")
    options = subset.Options()
    options.hinting = True
    options.legacy_kern = True
    options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
    options.notdef_outline = True
    options.glyph_names = False
    options.drop_tables += ["GPOS", "GSUB", "GDEF", "FFTM", "MATH"]
    options.layout_features = []
    font = subset.load_font(os.path.join(src, "DejaVuSans.ttf"), options)
    subsetter = subset.Subsetter(options)
    unicodes = (list(range(0x20, 0x7F)) + list(range(0x0300, 0x0310)) + list(range(0x05B0, 0x05EB))
                + list(range(0x0621, 0x0656)) + [0x0670, 0x0671] + list(range(0xFB1D, 0xFB50))
                + list(range(0xFE70, 0xFEFD)))
    subsetter.populate(unicodes=unicodes)
    subsetter.subset(font)
    subset.save_font(font, os.path.join(out, "DejaVuSans-NoLayout.ttf"), options)


if __name__ == "__main__":
    args = sys.argv[1:]
    shaping_only = "--shaping" in args
    formats_only = "--formats" in args
    args = [a for a in args if a not in ("--shaping", "--formats")]
    outdir = args[0] if args else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "sdl3-ttf", "src", "testdata", "fonts")
    if formats_only:
        make_format_fonts(outdir)
    else:
        if not shaping_only:
            main(outdir)
            make_format_fonts(outdir)
        make_shaping_fonts(outdir)
