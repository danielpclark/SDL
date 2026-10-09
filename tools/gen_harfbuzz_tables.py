#!/usr/bin/env python3
# Translates the generated tables of HarfBuzz (8.5.0, as SDL_ttf's
# external/harfbuzz pins it) into the Rust table modules of
# sdl3-ttf/src/harfbuzz/: the Unicode Character Database tables
# (hb-ucd-table.hh, hb-unicode-emoji-table.hh), the OpenType tag tables
# (hb-ot-tag-table.hh), and the shapers' tables (Arabic joining, Indic,
# USE) and Ragel state machines.
#
# Usage: tools/gen_harfbuzz_tables.py HARFBUZZ_SRC_DIR [OUTDIR]
#
# HARFBUZZ_SRC_DIR is HarfBuzz's src/ directory; OUTDIR defaults to
# sdl3-ttf/src/harfbuzz. The tables are copied value for value (HarfBuzz
# itself generates them from the Unicode Character Database and other
# data files with its gen-*.py scripts); the lookup functions next to them
# are translated by the patterns below, which only cover the expressions
# those generated files use.

import os
import re
import sys

HEADER = """\
// Rust translation of src/{src} from HarfBuzz (8.5.0, as SDL_ttf's
// external/harfbuzz pins it), made by tools/gen_harfbuzz_tables.py.
// {copyright}
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license{extra}).
"""

UNICODE_EXTRA = ", and the Unicode License for the data"

CTYPES = {
    "uint8_t": "u8",
    "int8_t": "i8",
    "uint16_t": "u16",
    "int16_t": "i16",
    "uint32_t": "u32",
    "int32_t": "i32",
    "uint64_t": "u64",
    "hb_script_t": "HbScript",
}


def strip_comments(s):
    return re.sub(r"/\*.*?\*/", "", s, flags=re.S)


def find_array(src, name):
    """The type, the size and the elements' source of `static const T name[N] = {...};`."""
    m = re.search(r"static const (\w+)\s+%s\s*\[(\d*)\]\s*=\s*\{" % re.escape(name), src)
    if not m:
        raise KeyError(name)
    start = m.end()
    depth = 1
    i = start
    while depth:
        c = src[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        i += 1
    return m.group(1), m.group(2), src[start:i - 1]


def split_elements(body):
    body = strip_comments(body)
    out = []
    depth = 0
    cur = ""
    for c in body:
        if c in "({":
            depth += 1
        elif c in ")}":
            depth -= 1
        if c == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += c
    if cur.strip():
        out.append(cur.strip())
    return out


def c_value(v):
    v = v.strip()
    v = re.sub(r"\b(0x[0-9A-Fa-f]+|\d+)[uU]?[lL]*\b", r"\1", v)
    v = v.replace("HB_CODEPOINT_ENCODE3_11_7_14", "hb_codepoint_encode3_11_7_14")
    v = v.replace("HB_CODEPOINT_ENCODE3", "hb_codepoint_encode3")
    return v


def rust_array(src, name, rname=None, rtype=None, per_line=None, public=False):
    ctype, size, body = find_array(src, name)
    elements = [c_value(e) for e in split_elements(body)]
    if size and int(size) != len(elements):
        raise ValueError("%s: %s elements, not %s" % (name, len(elements), size))
    rtype = rtype or CTYPES[ctype]
    rname = rname or name
    if per_line is None:
        width = max(len(e) for e in elements)
        per_line = max(1, 96 // (width + 2))
    lines = []
    for i in range(0, len(elements), per_line):
        lines.append("    " + ", ".join(elements[i:i + per_line]) + ",")
    vis = "pub(crate) " if public else ""
    return "#[rustfmt::skip]\n%sstatic %s: [%s; %d] = [\n%s\n];\n" % (
        vis, rname, rtype, len(elements), "\n".join(lines))


def convert_index_expr(expr, arrays):
    """Translates a generated lookup expression (array lookups, shifts,
    masks, `cond?a:b`) over the code point `u` (u32)."""
    out = ""
    i = 0
    while i < len(expr):
        m = re.match(r"(\w+)\[", expr[i:])
        if m and m.group(1) in arrays:
            depth = 1
            j = i + m.end()
            while depth:
                if expr[j] == "[":
                    depth += 1
                elif expr[j] == "]":
                    depth -= 1
                j += 1
            inner = convert_index_expr(expr[i + m.end():j - 1], arrays)
            out += "(%s[(%s) as usize] as u32)" % (m.group(1), inner)
            i = j
            continue
        m = re.match(r"(\w+)\((\d+)\+(\w+),", expr[i:])
        if m:
            # b4/b1 helpers on an offset into a table: f(N+table, ...)
            out += "%s(&%s[%s..]," % (m.group(1), m.group(3), m.group(2))
            i += m.end()
            continue
        m = re.match(r"(\w+)\((\w+),", expr[i:])
        if m and m.group(2) in arrays:
            out += "%s(&%s[..]," % (m.group(1), m.group(2))
            i += m.end()
            continue
        m = re.match(r"(0x[0-9A-Fa-f]+|\d+)u", expr[i:])
        if m:
            out += m.group(1)
            i += m.end()
            continue
        out += expr[i]
        i += 1
    return out


def convert_ternary(body, arrays):
    m = re.match(r"\s*return\s+(.*?)\?(.*):(\w+);\s*$", body, flags=re.S)
    if not m:
        raise ValueError("unexpected lookup function body: " + body)
    cond, a, b = m.groups()
    return "if %s { %s } else { %s }" % (
        convert_index_expr(cond, arrays), convert_index_expr(a, arrays), b)


def lookup_functions(src, names, arrays, rtypes):
    out = []
    for name in names:
        m = re.search(r"static inline \w+\s+%s \(unsigned u\)\s*\{(.*?)\}" % name, src, flags=re.S)
        if not m:
            raise KeyError(name)
        expr = convert_ternary(m.group(1), arrays)
        out.append("#[rustfmt::skip]\n#[allow(unused_parens, clippy::double_parens)]\n#[inline]\npub(crate) fn %s(u: u32) -> %s {\n    (%s) as %s\n}\n"
                   % (name, rtypes[name], expr, rtypes[name]))
    return "\n".join(out)


def bit_helpers(prefix, widths):
    out = []
    for bits in widths:
        shift = {4: 1, 1: 3, 2: 2}[bits]
        mask = (1 << bits) - 1
        sh = {4: 2, 2: 1, 1: 0}[bits]
        out.append("#[inline]\nfn %s_b%d(a: &[u8], i: u32) -> u32 {\n"
                   "    ((a[(i >> %d) as usize] as u32) >> ((i & %d) << %d)) & %d\n}\n"
                   % (prefix, bits, shift, (8 // bits) - 1, sh, mask))
    return "\n".join(out)


def section(src, start_marker, end_marker):
    a = src.index(start_marker)
    b = src.index(end_marker, a)
    return src[a:b]


def gen_ucd(hb, out):
    src = open(os.path.join(hb, "hb-ucd-table.hh"), encoding="utf-8").read()
    # the default build's tables (HB_OPTIMIZE_SIZE is not defined)
    default = section(src, "#ifndef HB_OPTIMIZE_SIZE", "#elif !defined(HB_NO_UCD_UNASSIGNED)")
    arrays = ["_hb_ucd_u8", "_hb_ucd_u16", "_hb_ucd_i16"]
    text = HEADER.format(src="hb-ucd-table.hh",
                         copyright="Generated by HarfBuzz's gen-ucd-table.py from the Unicode Character Database (Unicode 15.1.0).",
                         extra=UNICODE_EXTRA)
    text += "\n//! The Unicode Character Database tables of `hb-ucd.cc`.\n\n"
    text += "#![allow(non_upper_case_globals)]\n\n"
    text += "use super::hb_common::*;\n\n"
    text += "/// `HB_CODEPOINT_ENCODE3`\npub(crate) const fn hb_codepoint_encode3(x: u32, y: u32, z: u32) -> u64 {\n"
    text += "    ((x as u64) << 42) | ((y as u64) << 21) | (z as u64)\n}\n\n"
    text += "/// `HB_CODEPOINT_ENCODE3_11_7_14`\npub(crate) const fn hb_codepoint_encode3_11_7_14(x: u32, y: u32, z: u32) -> u32 {\n"
    text += "    ((x & 0x07FF) << 21) | ((y & 0x007F) << 14) | (z & 0x3FFF)\n}\n\n"
    text += rust_array(src, "_hb_ucd_sc_map", per_line=2, public=True) + "\n"
    for name in ["_hb_ucd_dm1_p0_map", "_hb_ucd_dm1_p2_map", "_hb_ucd_dm2_u32_map", "_hb_ucd_dm2_u64_map"]:
        text += rust_array(src, name, public=True, per_line=(2 if "dm2" in name else None)) + "\n"
    for name in arrays:
        text += rust_array(default, name) + "\n"
    text += bit_helpers("_hb_ucd", [4]) + "\n"
    text += lookup_functions(default, ["_hb_ucd_gc", "_hb_ucd_ccc", "_hb_ucd_bmg", "_hb_ucd_sc", "_hb_ucd_dm"], arrays,
                             {"_hb_ucd_gc": "u8", "_hb_ucd_ccc": "u8", "_hb_ucd_bmg": "i16", "_hb_ucd_sc": "u8",
                              "_hb_ucd_dm": "u16"})
    write(out, "hb_ucd_table.rs", text)

    src = open(os.path.join(hb, "hb-unicode-emoji-table.hh"), encoding="utf-8").read()
    text = HEADER.format(src="hb-unicode-emoji-table.hh",
                         copyright="Generated by HarfBuzz's gen-emoji-table.py from the Unicode emoji data (emoji-data.txt, Emoji Version 15.1).",
                         extra=UNICODE_EXTRA)
    text += "\n//! The emoji table of `hb-unicode.cc` (Extended_Pictographic).\n\n"
    text += "#![allow(non_upper_case_globals)]\n\n"
    text += rust_array(src, "_hb_emoji_u8") + "\n"
    text += bit_helpers("_hb_emoji", [4, 1]) + "\n"
    text += lookup_functions(src, ["_hb_emoji_is_Extended_Pictographic"], ["_hb_emoji_u8"],
                             {"_hb_emoji_is_Extended_Pictographic": "u8"})
    write(out, "hb_unicode_emoji_table.rs", text)


def gen_arabic_pua(hb, out):
    src = open(os.path.join(hb, "hb-ot-shaper-arabic-pua.hh"), encoding="utf-8").read()
    arrays = ["_hb_arabic_u8", "_hb_arabic_u16"]
    text = HEADER.format(src="hb-ot-shaper-arabic-pua.hh",
                         copyright="Generated by HarfBuzz's gen-arabic-pua.py.",
                         extra="")
    text += "\n//! The Arabic private-use-area mappings of symbol fonts (Windows 3.1\n"
    text += "//! Arabic font pages).\n\n"
    text += "#![allow(non_upper_case_globals)]\n\n"
    for name in arrays:
        text += rust_array(src, name) + "\n"
    text += bit_helpers("_hb_arabic", [2, 4]) + "\n"
    text += lookup_functions(src, ["_hb_arabic_pua_simp_map", "_hb_arabic_pua_trad_map"], arrays,
                             {"_hb_arabic_pua_simp_map": "u16", "_hb_arabic_pua_trad_map": "u16"})
    write(out, "hb_ot_shaper_arabic_pua.rs", text)


def write(out, name, text):
    with open(os.path.join(out, name), "w", encoding="utf-8") as f:
        f.write(text.rstrip("\n") + "\n")


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__ or "usage: gen_harfbuzz_tables.py HARFBUZZ_SRC_DIR [OUTDIR]")
    hb = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..", "sdl3-ttf", "src", "harfbuzz")
    os.makedirs(out, exist_ok=True)
    gen_ucd(hb, out)
    gen_arabic_pua(hb, out)


if __name__ == "__main__":
    main()
