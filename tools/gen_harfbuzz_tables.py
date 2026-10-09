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


def c_tag(expr):
    """`HB_TAG('a','b',' ',' ')` / `HB_TAG_NONE` as Rust."""
    expr = expr.strip()
    if expr == "HB_TAG_NONE":
        return "HB_TAG_NONE"
    m = re.fullmatch(r"HB_TAG\('(.)','(.)','(.)','(.)'\)", expr)
    if not m:
        raise ValueError(expr)
    return 't(b"%s")' % "".join(m.groups())


def rust_bytes(s):
    return 'b"%s"' % s


def gen_ot_tag(hb, out):
    src = open(os.path.join(hb, "hb-ot-tag-table.hh"), encoding="utf-8").read()
    text = HEADER.format(src="hb-ot-tag-table.hh",
                         copyright="Generated by HarfBuzz's gen-tag-table.py from the OpenType language system tags and the IANA language subtag registry.",
                         extra="")
    text += "\n//! The OpenType language system tags of the BCP 47 languages.\n//!\n"
    text += "//! `hb_ot_tags_from_complex_language` is generated C code; it is translated\n"
    text += "//! as the list of its rules, in order, which `hb_ot_tag.rs` runs.\n\n"
    text += "#![allow(non_upper_case_globals)]\n\n"
    text += "use super::hb_common::*;\n\n"
    text += "const fn t(s: &[u8; 4]) -> HbTag {\n    hb_tag(s[0], s[1], s[2], s[3])\n}\n\n"
    text += "/// `LangTag`\n#[derive(Debug, Clone, Copy)]\npub(crate) struct LangTag {\n"
    text += "    pub(crate) language: HbTag,\n    pub(crate) tag: HbTag,\n}\n\n"
    for name in ["ot_languages2", "ot_languages3"]:
        m = re.search(r"static const LangTag %s\[\] = \{(.*?)\n\};" % name, src, re.S)
        rows = []
        for line in m.group(1).split("\n"):
            line = line.strip()
            if not line or line.startswith("/*"):
                # (empty lines and the commented-out entries)
                continue
            mm = re.fullmatch(r"\{(HB_TAG\([^)]*\)),\s*(HB_TAG\([^)]*\)|HB_TAG_NONE)\s*\},\s*(/\*.*\*/)?", line)
            if not mm:
                raise ValueError(line)
            comment = (" // " + mm.group(3)[2:-2].strip()) if mm.group(3) else ""
            rows.append("    LangTag { language: %s, tag: %s },%s" % (c_tag(mm.group(1)), c_tag(mm.group(2)), comment))
        text += "#[rustfmt::skip]\npub(crate) static %s: [LangTag; %d] = [\n%s\n];\n\n" % (name, len(rows), "\n".join(rows))

    # hb_ot_tags_from_complex_language
    body = section(src, "hb_ot_tags_from_complex_language (const char", "hb_ot_ambiguous_tag_to_language")
    head, rest = body.split("\nout:\n", 1)
    text += "/// A condition of `hb_ot_tags_from_complex_language`'s rules (on\n"
    text += "/// `lang_str`, with `rest` its part after the first character).\n"
    text += "#[derive(Debug, Clone, Copy)]\npub(crate) enum ComplexCond {\n"
    text += "    /// `0 == strcmp (&lang_str[1], s)`\n    Strcmp(&'static [u8]),\n"
    text += "    /// `lang_matches (&lang_str[1], limit, s, strlen (s))`\n    LangMatches(&'static [u8]),\n"
    text += "    /// `0 == strncmp (&lang_str[1], a, strlen (a)) && subtag_matches (lang_str, limit, b, strlen (b))`\n"
    text += "    StrncmpSubtag(&'static [u8], &'static [u8]),\n}\n\n"

    def tags_of(block):
        return [c_tag(x) for x in re.findall(r"HB_TAG\('.','.','.','.'\)", strip_comments(block))]

    # the subtag rules on p (`subtag_matches (p, limit, s, n)`)
    prules = []
    for m in re.finditer(r'if \(subtag_matches \(p, limit, "([^"]*)", (\d+)\)\)\s*\{(.*?)return true;', head, re.S):
        assert len(m.group(1)) == int(m.group(2))
        prules.append("    (%s, &[%s])," % (rust_bytes(m.group(1)), ", ".join(tags_of(m.group(3)))))
    assert "limit - lang_str >= 7" in head and "limit - p < 5" in head
    text += "/// The rules on the subtags after the first `-` (`subtag_matches (p, limit,\n"
    text += "/// s, strlen (s))`), tried when `limit - lang_str >= 7` and the `-` is\n"
    text += "/// before `limit - 5`.\n"
    text += "#[rustfmt::skip]\npub(crate) static COMPLEX_SUBTAG_RULES: [(&[u8], &[HbTag]); %d] = [\n%s\n];\n\n" % (
        len(prules), "\n".join(prules))

    rules = []
    cur = None
    pos = 0
    pat = re.compile(r"case '(.)':|"
                     r'if \(0 == strcmp \(&lang_str\[1\], "([^"]*)"\)\)\s*\{(.*?)return true;|'
                     r'if \(lang_matches \(&lang_str\[1\], limit, "([^"]*)", (\d+)\)\)\s*\{(.*?)return true;|'
                     r'if \(0 == strncmp \(&lang_str\[1\], "([^"]*)", (\d+)\)\s*&& subtag_matches \(lang_str, limit, "([^"]*)", (\d+)\)\)\s*\{(.*?)return true;',
                     re.S)
    for m in pat.finditer(rest):
        if m.group(1):
            cur = m.group(1)
            continue
        if m.group(2) is not None:
            cond = "ComplexCond::Strcmp(%s)" % rust_bytes(m.group(2))
            block = m.group(3)
        elif m.group(4) is not None:
            assert len(m.group(4)) == int(m.group(5))
            cond = "ComplexCond::LangMatches(%s)" % rust_bytes(m.group(4))
            block = m.group(6)
        else:
            assert len(m.group(7)) == int(m.group(8)) and len(m.group(9)) == int(m.group(10))
            cond = "ComplexCond::StrncmpSubtag(%s, %s)" % (rust_bytes(m.group(7)), rust_bytes(m.group(9)))
            block = m.group(11)
        tags = tags_of(block)
        rules.append("    (b'%s', %s, &[%s])," % (cur, cond, ", ".join(tags)))
    assert len(rules) == rest.count("return true;"), (len(rules), rest.count("return true;"))
    text += "/// The rules of `switch (lang_str[0])`: the first character, the\n"
    text += "/// condition and the tags.\n"
    text += "#[rustfmt::skip]\npub(crate) static COMPLEX_RULES: [(u8, ComplexCond, &[HbTag]); %d] = [\n%s\n];\n\n" % (
        len(rules), "\n".join(rules))

    amb = section(src, "hb_ot_ambiguous_tag_to_language (hb_tag_t tag)", "#endif /* HB_OT_TAG_TABLE_HH */")
    pairs = re.findall(r"case (HB_TAG\([^)]*\)):.*?\n\s*return hb_language_from_string \(\"([^\"]*)\", -1\);", amb)
    assert len(pairs) == amb.count("case HB_TAG")
    rows = ['    (%s, "%s"),' % (c_tag(a), b) for a, b in pairs]
    text += "/// `hb_ot_ambiguous_tag_to_language`'s cases: the tag and its BCP 47\n/// language.\n"
    text += "#[rustfmt::skip]\npub(crate) static AMBIGUOUS_TAGS: [(HbTag, &str); %d] = [\n%s\n];\n" % (len(rows), "\n".join(rows))
    write(out, "hb_ot_tag_table.rs", text)


def gen_arabic(hb, out):
    src = open(os.path.join(hb, "hb-ot-shaper-arabic-table.hh"), encoding="utf-8").read()
    text = HEADER.format(src="hb-ot-shaper-arabic-table.hh",
                         copyright="Generated by HarfBuzz's gen-arabic-table.py from the Unicode Character Database (ArabicShaping.txt, UnicodeData.txt, Blocks.txt; Unicode 15.1.0).",
                         extra=UNICODE_EXTRA)
    text += "\n//! The Arabic joining types and the presentation forms of the Arabic\n//! fallback shaping.\n\n"
    text += "#![allow(non_upper_case_globals)]\n\n"
    text += "use super::hb_ot_shaper_arabic::*;\n\n"
    # joining_table
    body = section(src, "static const uint8_t joining_table[] =", "}; /* Table items")
    offsets = dict(re.findall(r"#define (joining_offset_0x[0-9a-f]+u) (\d+)", body))
    items = []
    for line in strip_comments(body.split("{", 1)[1]).split("\n"):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        items += [x for x in line.split(",") if x.strip()]
    names = {"A": "A", "DR": "DR", "C": "C", "D": "D", "L": "L", "R": "R", "T": "T", "U": "U", "X": "X"}
    items = [names[x.strip()] for x in items]
    text += "const A: u8 = JOINING_GROUP_ALAPH;\nconst DR: u8 = JOINING_GROUP_DALATH_RISH;\n"
    text += "const C: u8 = JOINING_TYPE_C;\nconst D: u8 = JOINING_TYPE_D;\nconst L: u8 = JOINING_TYPE_L;\n"
    text += "const R: u8 = JOINING_TYPE_R;\nconst T: u8 = JOINING_TYPE_T;\nconst U: u8 = JOINING_TYPE_U;\n"
    text += "const X: u8 = JOINING_TYPE_X;\n\n"
    rows = ["    " + ", ".join(items[i:i + 32]) + "," for i in range(0, len(items), 32)]
    text += "#[rustfmt::skip]\nstatic joining_table: [u8; %d] = [\n%s\n];\n\n" % (len(items), "\n".join(rows))
    for k, v in sorted(offsets.items(), key=lambda kv: int(kv[1])):
        text += "const %s: usize = %s;\n" % (k, v)
    # joining_type
    fn = section(src, "joining_type (hb_codepoint_t u)", "#undef A")
    text += "\n/// `joining_type`\npub(crate) fn joining_type(u: u32) -> u8 {\n    match u >> 12 {\n"
    for case in re.finditer(r"case (0x[0-9A-F]+)u:(.*?)break;", fn, re.S):
        text += "        %s => {\n" % case.group(1)
        for m in re.finditer(r"if \(hb_in_range<hb_codepoint_t> \(u, (0x[0-9A-F]+)u, (0x[0-9A-F]+)u\)\) return joining_table\[u - (0x[0-9A-F]+)u \+ (joining_offset_0x[0-9a-f]+u)\];", case.group(2)):
            assert m.group(1) == m.group(3)
            text += "            if (%s..=%s).contains(&u) {\n                return joining_table[(u - %s) as usize + %s];\n            }\n" % (
                m.group(1), m.group(2), m.group(1), m.group(4))
        text += "        }\n"
    text += "        _ => {}\n    }\n    X\n}\n\n"
    # shaping_table
    m = re.search(r"static const uint16_t shaping_table\[\]\[4\] =\s*\{(.*?)\n\};", src, re.S)
    rows = []
    for line in m.group(1).split("\n"):
        mm = re.match(r"\s*\{(0x[0-9A-F]+)u, (0x[0-9A-F]+)u, (0x[0-9A-F]+)u, (0x[0-9A-F]+)u\}, /\* (.*) \*/", line)
        if mm:
            rows.append("    [%s, %s, %s, %s], // %s" % (mm.group(1), mm.group(2), mm.group(3), mm.group(4), mm.group(5).strip()))
    first = re.search(r"#define SHAPING_TABLE_FIRST\s+(0x[0-9A-F]+)u", src).group(1)
    last = re.search(r"#define SHAPING_TABLE_LAST\s+(0x[0-9A-F]+)u", src).group(1)
    text += "pub(crate) const SHAPING_TABLE_FIRST: u32 = %s;\npub(crate) const SHAPING_TABLE_LAST: u32 = %s;\n\n" % (first, last)
    text += "#[rustfmt::skip]\npub(crate) static shaping_table: [[u16; 4]; %d] = [\n%s\n];\n\n" % (len(rows), "\n".join(rows))
    # ligature tables
    for name, ncomp in [("ligature_table", 1), ("ligature_mark_table", 1), ("ligature_3_table", 2)]:
        m = re.search(r"\} ligatures\[(\d+)\];\n\} %s\[\] =\s*\{(.*?)\n\};" % name, src, re.S)
        nlig = int(m.group(1))
        sets = []
        for sm in re.finditer(r"\{ (0x[0-9A-F]+)u, \{\n(.*?)\n  \}\},", m.group(2), re.S):
            ligs = []
            for lm in re.finditer(r"\{ \{([^}]*)\}, (0x[0-9A-F]+)u\s*\}, /\* (.*) \*/", sm.group(2)):
                comps = [c.strip().rstrip("u") for c in lm.group(1).split(",")]
                assert len(comps) == ncomp
                ligs.append("            LigaturePair { components: [%s], ligature: %s }, // %s" % (
                    ", ".join(comps), lm.group(2), lm.group(3).strip()))
            while len(ligs) < nlig:
                ligs.append("            LigaturePair { components: [%s], ligature: 0 }," % ", ".join(["0"] * ncomp))
            sets.append("    LigatureSet {\n        first: %s,\n        ligatures: [\n%s\n        ],\n    }," % (sm.group(1), "\n".join(ligs)))
        text += "#[rustfmt::skip]\npub(crate) static %s: [LigatureSet<%d, %d>; %d] = [\n%s\n];\n\n" % (
            name, ncomp, nlig, len(sets), "\n".join(sets))
    write(out, "hb_ot_shaper_arabic_table.rs", text)


def machine_categories(src, prefix):
    """The `#define <prefix>_ex_<Cat> <n>u` categories of a Ragel machine."""
    return {m.group(1): int(m.group(2)) for m in re.finditer(r"#define %s_ex_(\w+) (\d+)u" % prefix, src)}


def gen_machine(hb, out, name, prefix, copyright):
    """The tables of a Ragel state machine (hb-ot-shaper-NAME-machine.hh)."""
    src = open(os.path.join(hb, "hb-ot-shaper-%s-machine.hh" % name), encoding="utf-8").read()
    text = HEADER.format(src="hb-ot-shaper-%s-machine.hh" % name,
                         copyright=copyright + " Generated by Ragel from hb-ot-shaper-%s-machine.rl." % name,
                         extra="")
    text += "\n//! The state machine tables of the %s syllable machine (made by Ragel);\n" % name
    text += "//! `hb_ragel.rs` runs them, with the machine's actions in the shaper.\n\n"
    text += "#![allow(non_upper_case_globals, dead_code)]\n\n"
    text += "use super::hb_ragel::RagelTables;\n\n"
    cats = machine_categories(src, prefix)
    for cat, n in sorted(cats.items()):
        text += "pub(crate) const %s_ex_%s: u8 = %d;\n" % (prefix, cat, n)
    text += "\n"
    names = []
    for m in re.finditer(r"static const (unsigned char|char|short|unsigned short|int) _%s_(\w+)\[\] = \{(.*?)\};" % prefix, src, re.S):
        vals = [int(v) for v in re.findall(r"-?\d+", m.group(3))]
        assert all(0 <= v < 65536 for v in vals), m.group(2)
        rows = ["    " + ", ".join(str(v) for v in vals[i:i + 16]) + "," for i in range(0, len(vals), 16)]
        text += "#[rustfmt::skip]\nstatic _%s_%s: [u16; %d] = [\n%s\n];\n\n" % (prefix, m.group(2), len(vals), "\n".join(rows))
        names.append(m.group(2))
    for k in ["start", "first_final", "error", "en_main"]:
        v = re.search(r"static const int %s_%s = (-?\d+);" % (prefix, k), src).group(1)
        text += "pub(crate) const %s_%s: i32 = %s;\n" % (prefix, k, v)
    # the state actions of the scanner (ts = p, ts = 0)
    from_case = re.search(r"switch \( _%s_from_state_actions\[cs\] \) \{\s*case (\d+):" % prefix, src).group(1)
    to_case = re.search(r"switch \( _%s_to_state_actions\[cs\] \) \{\s*case (\d+):" % prefix, src).group(1)
    for k in ["trans_keys", "key_spans", "index_offsets", "indicies", "trans_targs", "trans_actions",
              "to_state_actions", "from_state_actions", "eof_trans"]:
        assert k in names, (name, k)
    text += "\n/// The machine's tables.\npub(crate) static %s_TABLES: RagelTables = RagelTables {\n" % name.upper()
    for k in ["trans_keys", "key_spans", "index_offsets", "indicies", "trans_targs", "trans_actions",
              "to_state_actions", "from_state_actions", "eof_trans"]:
        text += "    %s: &_%s_%s,\n" % (k, prefix, k)
    text += "    start: %s_start,\n" % prefix
    text += "    from_state_action_ts: %s,\n    to_state_action_ts: %s,\n};\n" % (from_case, to_case)
    write(out, "hb_ot_shaper_%s_machine.rs" % name, text)
    return cats


def gen_indic_table(hb, out, indic_cats, khmer_cats, myanmar_cats):
    src = open(os.path.join(hb, "hb-ot-shaper-indic-table.cc"), encoding="utf-8").read()
    text = HEADER.format(src="hb-ot-shaper-indic-table.cc",
                         copyright="Generated by HarfBuzz's gen-indic-table.py from the Unicode Character Database (IndicSyllabicCategory.txt, IndicPositionalCategory.txt, Blocks.txt; Unicode 15.1.0).",
                         extra=UNICODE_EXTRA)
    text += "\n//! The Indic, Khmer and Myanmar categories and positions of the characters.\n\n"
    # OT_* -> numbers
    ot = {}
    cat_sets = {"I_Cat": indic_cats, "K_Cat": khmer_cats, "M_Cat": myanmar_cats}
    for m in re.finditer(r"#define (OT_\w+) ([IKM]_Cat)\((\w+)\)", src):
        ot[m.group(1)] = cat_sets[m.group(2)][m.group(3)]
    for m in re.finditer(r"static_assert \((OT_\w+) == ([IKM]_Cat)\((\w+)\), \"\"\);", src):
        assert ot[m.group(1)] == cat_sets[m.group(2)][m.group(3)]
    pos_src = open(os.path.join(hb, "hb-ot-shaper-indic.hh"), encoding="utf-8").read()
    pos = {m.group(1): int(m.group(2)) for m in re.finditer(r"(POS_\w+) = (\d+)", pos_src)}
    s_short = {m.group(1): ot[m.group(2)] for m in re.finditer(r"#define _OT_(\w+)\s+(OT_\w+)", src)}
    m_short = {m.group(1): pos[m.group(2)] for m in re.finditer(r"#define _POS_(\w+)\s+(POS_\w+)", src)}

    def val(s, mm):
        return s_short[s] | (m_short[mm] << 8)

    body = section(src, "static const uint16_t indic_table[] = {", "}; /* Table items")
    offsets = dict(re.findall(r"#define (indic_offset_0x[0-9a-f]+u) (\d+)", body))
    rows = []
    count = 0
    for line in body.split("\n"):
        items = re.findall(r"_\((\w+),(\w+)\)", line)
        if not items:
            continue
        label = re.search(r"/\* ([0-9A-F]+) \*/", line).group(1)
        rows.append("    /* %s */ %s," % (label, ", ".join("0x%04X" % val(a, b) for a, b in items)))
        count += len(items)
    text += "#[rustfmt::skip]\nstatic indic_table: [u16; %d] = [\n%s\n];\n\n" % (count, "\n".join(rows))
    for k, v in sorted(offsets.items(), key=lambda kv: int(kv[1])):
        text += "const %s: usize = %s;\n" % (k, v)
    fn = section(src, "hb_indic_get_categories (hb_codepoint_t u)", "#undef _\n")
    text += "\n/// `hb_indic_get_categories`: the category (low byte) and position (high\n/// byte) of `u`.\n"
    text += "pub(crate) fn hb_indic_get_categories(u: u32) -> u16 {\n    match u >> 12 {\n"
    for case in re.finditer(r"case (0x[0-9A-F]+)u:(.*?)break;", fn, re.S):
        text += "        %s => {\n" % case.group(1)
        for stmt in case.group(2).strip().split("\n"):
            stmt = stmt.strip()
            m1 = re.fullmatch(r"if \(unlikely \(u == (0x[0-9A-F]+)u\)\) return _\((\w+),(\w+)\);", stmt)
            m2 = re.fullmatch(r"if \(hb_in_range<hb_codepoint_t> \(u, (0x[0-9A-F]+)u, (0x[0-9A-F]+)u\)\) return indic_table\[u - (0x[0-9A-F]+)u \+ (indic_offset_0x[0-9a-f]+u)\];", stmt)
            if m1:
                text += "            if u == %s {\n                return 0x%04X;\n            }\n" % (m1.group(1), val(m1.group(2), m1.group(3)))
            elif m2:
                assert m2.group(1) == m2.group(3)
                text += "            if (%s..=%s).contains(&u) {\n                return indic_table[(u - %s) as usize + %s];\n            }\n" % (
                    m2.group(1), m2.group(2), m2.group(1), m2.group(4))
            elif stmt:
                raise ValueError(stmt)
        text += "        }\n"
    dflt = re.search(r"return _\((\w+),(\w+)\);\s*\}\s*$", fn.strip() + "\n}")
    text += "        _ => {}\n    }\n    0x%04X\n}\n" % val("X", "X")
    write(out, "hb_ot_shaper_indic_table.rs", text)


def gen_vowel_constraints(hb, out):
    src = open(os.path.join(hb, "hb-ot-shaper-vowel-constraints.cc"), encoding="utf-8").read()
    text = HEADER.format(src="hb-ot-shaper-vowel-constraints.cc",
                         copyright="Generated by HarfBuzz's gen-vowel-constraints.py from the Unicode data (Scripts.txt) and ms-use/IndicShapingInvalidCluster.txt.",
                         extra=UNICODE_EXTRA)
    text += "\n//! The vowel sequences that get a dotted circle inserted, by script.\n//!\n"
    text += "//! `_hb_preprocess_text_vowel_constraints` is generated C code; its cases are\n"
    text += "//! translated as the rules below, which `hb_ot_shaper_vowel_constraints.rs` runs.\n\n"
    text += "use super::hb_common::*;\n\n"
    text += "/// A rule: the following characters that match (`Second`), or\n"
    text += "/// the two that must follow, after which the dotted circle goes (`Third`).\n"
    text += "#[derive(Debug, Clone, Copy)]\npub(crate) enum VowelRule {\n    Second(&'static [u32]),\n    Third(u32, u32),\n}\n\n"
    body = section(src, "switch ((unsigned) buffer->props.script)", "    default:\n      break;")
    scripts = []
    for sm in re.finditer(r"case (HB_SCRIPT_\w+):\n(.*?)\n      break;\n", body, re.S):
        block = sm.group(2)
        rules = []
        # switch form
        for cm in re.finditer(r"\n\t  ((?:case 0x[0-9A-F]+u: ?)+)\n(.*?)\n\t    break;", block, re.S):
            b = cm.group(2)
            m1 = re.search(r"matched = (0x[0-9A-F]+)u == buffer->cur \(1\)\.codepoint;", b)
            m3 = re.search(r"if \((0x[0-9A-F]+)u == buffer->cur \(1\)\.codepoint &&\s*buffer->idx \+ 2 < count &&\s*(0x[0-9A-F]+)u == buffer->cur \(2\)\.codepoint\)", b)
            for first in re.findall(r"case (0x[0-9A-F]+)u:", cm.group(1)):
                if m3:
                    rules.append("(%s, VowelRule::Third(%s, %s))" % (first, m3.group(1), m3.group(2)))
                elif m1:
                    rules.append("(%s, VowelRule::Second(&[%s]))" % (first, m1.group(1)))
                else:
                    seconds = re.findall(r"case (0x[0-9A-F]+)u:", b)
                    assert seconds and "matched = true;" in b, b
                    rules.append("(%s, VowelRule::Second(&[%s]))" % (first, ", ".join(seconds)))
        # if form (Tamil)
        for im in re.finditer(r"if \((0x[0-9A-F]+)u == buffer->cur \(\)\.codepoint &&\s*(0x[0-9A-F]+)u == buffer->cur \(1\)\.codepoint\)", block):
            rules.append("(%s, VowelRule::Second(&[%s]))" % (im.group(1), im.group(2)))
        assert rules, sm.group(1)
        scripts.append("    (\n        %s,\n        &[\n%s\n        ],\n    )," % (
            sm.group(1), "\n".join("            %s," % r for r in rules)))
    text += "/// The rules by script, as `(first character, rule)`.\n#[rustfmt::skip]\n"
    text += "pub(crate) static VOWEL_CONSTRAINTS: [(HbScript, &[(u32, VowelRule)]); %d] = [\n%s\n];\n" % (
        len(scripts), "\n".join(scripts))
    write(out, "hb_ot_shaper_vowel_constraints_table.rs", text)


def gen_use_table(hb, out, use_cats):
    src = open(os.path.join(hb, "hb-ot-shaper-use-table.hh"), encoding="utf-8").read()
    text = HEADER.format(src="hb-ot-shaper-use-table.hh",
                         copyright="Generated by HarfBuzz's gen-use-table.py from the Unicode Character Database (IndicSyllabicCategory.txt, IndicPositionalCategory.txt, ArabicShaping.txt, DerivedCoreProperties.txt, UnicodeData.txt, Blocks.txt, Scripts.txt; Unicode 15.1.0) and Microsoft's USE overrides (IndicSyllabicCategory-Additional.txt, IndicPositionalCategory-Additional.txt).",
                         extra=UNICODE_EXTRA)
    text += "\n//! The Universal Shaping Engine categories of the characters.\n\n"
    text += "#![allow(non_upper_case_globals)]\n\n"
    # the default build's tables (HB_OPTIMIZE_SIZE is not defined)
    default = section(src, "#ifndef HB_OPTIMIZE_SIZE", "#else")
    names = {m.group(1): m.group(2) for m in re.finditer(r"#define (\w+)\tUSE\((\w+)\)", src)}

    def sub_names(s):
        return re.sub(r"\b([A-Za-z]\w*)\b", lambda m: str(use_cats[names[m.group(1)]]) if m.group(1) in names else m.group(1), s)

    arrays = ["hb_use_u8", "hb_use_u16"]
    for name in arrays:
        ctype, size, body = find_array(default, name)
        fake = "static const %s %s[%s] = {%s};" % (ctype, name, size, sub_names(strip_comments(body)))
        text += rust_array(fake, name) + "\n"
    text += bit_helpers("hb_use", [4]) + "\n"
    fn = lookup_functions(default, ["hb_use_get_category"], arrays, {"hb_use_get_category": "u8"})
    text += fn.replace("else { O }", "else { %d }" % use_cats["O"])
    write(out, "hb_ot_shaper_use_table.rs", text)


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
    gen_ot_tag(hb, out)
    gen_arabic(hb, out)
    indic = gen_machine(hb, out, "indic", "indic_syllable_machine", "Copyright © 2011,2012  Google, Inc.")
    khmer = gen_machine(hb, out, "khmer", "khmer_syllable_machine", "Copyright © 2011,2012  Google, Inc.")
    myanmar = gen_machine(hb, out, "myanmar", "myanmar_syllable_machine", "Copyright © 2011,2012  Google, Inc.")
    gen_indic_table(hb, out, indic, khmer, myanmar)
    gen_vowel_constraints(hb, out)
    use = gen_machine(hb, out, "use", "use_syllable_machine",
                      "Copyright © 2015  Mozilla Foundation.\n// Copyright © 2015  Google, Inc.")
    gen_use_table(hb, out, use)


if __name__ == "__main__":
    main()
