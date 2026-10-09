// Rust translation of src/hb-ot-tag.cc (and the public part of
// src/hb-ot-tag.h) from HarfBuzz (8.5.0, as SDL_ttf's external/harfbuzz
// pins it).
// Copyright © 2009  Red Hat, Inc.
// Copyright © 2011  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod, Roozbeh Pournader

//! OpenType script and language system tags.
//!
//! Translation notes: the language strings are handled as byte strings
//! with C's terminating NUL implied (reads past the end give `0`).

use super::hb_common::*;
use super::hb_ot_layout::{HB_OT_TAG_DEFAULT_LANGUAGE, HB_OT_TAG_DEFAULT_SCRIPT};
use super::hb_ot_tag_table::*;

/// `HB_OT_TAG_MATH_SCRIPT`
pub const HB_OT_TAG_MATH_SCRIPT: HbTag = hb_tag(b'm', b'a', b't', b'h');

/// `HB_OT_MAX_TAGS_PER_SCRIPT`
pub const HB_OT_MAX_TAGS_PER_SCRIPT: usize = 3;
/// `HB_OT_MAX_TAGS_PER_LANGUAGE`
pub const HB_OT_MAX_TAGS_PER_LANGUAGE: usize = 3;

/// The byte of a NUL-terminated string at `i` (`0` past the end).
#[inline]
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `strlen (s + i)`
fn strlen_from(s: &[u8], i: usize) -> usize {
    let mut n = 0;
    while at(s, i + n) != 0 {
        n += 1;
    }
    n
}

/// `strchr (s + i, c)`: the index of the first `c` from `i`, if before the
/// terminating NUL.
fn strchr_from(s: &[u8], i: usize, c: u8) -> Option<usize> {
    let mut j = i;
    loop {
        let b = at(s, j);
        if b == c {
            return Some(j);
        }
        if b == 0 {
            return None;
        }
        j += 1;
    }
}

/// `strstr (s + i, needle)`
fn strstr_from(s: &[u8], i: usize, needle: &[u8]) -> Option<usize> {
    let n = strlen_from(s, i);
    if needle.is_empty() {
        return Some(i);
    }
    if needle.len() > n {
        return None;
    }
    (i..=i + n - needle.len()).find(|&j| &s[j..j + needle.len()] == needle)
}

/// `strncmp (s + i, t, n) == 0` (with `t` at least `n` long)
fn strncmp_eq(s: &[u8], i: usize, t: &[u8], n: usize) -> bool {
    for k in 0..n {
        let a = at(s, i + k);
        let b = at(t, k);
        if a != b {
            return false;
        }
        if a == 0 {
            return true;
        }
    }
    true
}

#[inline]
fn is_alpha(c: u8) -> bool {
    c.is_ascii_alphabetic()
}
#[inline]
fn is_alnum(c: u8) -> bool {
    c.is_ascii_alphanumeric()
}

/* hb_script_t */

/// `hb_ot_old_tag_from_script`
fn hb_ot_old_tag_from_script(script: HbScript) -> HbTag {
    /* This seems to be accurate as of end of 2012. */

    match script {
        HB_SCRIPT_INVALID => return HB_OT_TAG_DEFAULT_SCRIPT,
        HB_SCRIPT_MATH => return HB_OT_TAG_MATH_SCRIPT,

        /* KATAKANA and HIRAGANA both map to 'kana' */
        HB_SCRIPT_HIRAGANA => return hb_tag(b'k', b'a', b'n', b'a'),

        /* Spaces at the end are preserved, unlike ISO 15924 */
        HB_SCRIPT_LAO => return hb_tag(b'l', b'a', b'o', b' '),
        HB_SCRIPT_YI => return hb_tag(b'y', b'i', b' ', b' '),
        /* Unicode-5.0 additions */
        HB_SCRIPT_NKO => return hb_tag(b'n', b'k', b'o', b' '),
        /* Unicode-5.1 additions */
        HB_SCRIPT_VAI => return hb_tag(b'v', b'a', b'i', b' '),
        _ => {}
    }

    /* Else, just change first char to lowercase and return */
    script | 0x20000000
}

/// `hb_ot_old_tag_to_script`
fn hb_ot_old_tag_to_script(mut tag: HbTag) -> HbScript {
    if tag == HB_OT_TAG_DEFAULT_SCRIPT {
        return HB_SCRIPT_INVALID;
    }
    if tag == HB_OT_TAG_MATH_SCRIPT {
        return HB_SCRIPT_MATH;
    }

    /* This side of the conversion is fully algorithmic. */

    /* Any spaces at the end of the tag are replaced by repeating the last
     * letter.  Eg 'nko ' -> 'Nkoo' */
    if (tag & 0x0000FF00) == 0x00002000 {
        tag |= (tag >> 8) & 0x0000FF00; /* Copy second letter to third */
    }
    if (tag & 0x000000FF) == 0x00000020 {
        tag |= (tag >> 8) & 0x000000FF; /* Copy third letter to fourth */
    }

    /* Change first char to uppercase and return */
    tag & !0x20000000
}

/// `hb_ot_new_tag_from_script`
fn hb_ot_new_tag_from_script(script: HbScript) -> HbTag {
    match script {
        HB_SCRIPT_BENGALI => hb_tag(b'b', b'n', b'g', b'2'),
        HB_SCRIPT_DEVANAGARI => hb_tag(b'd', b'e', b'v', b'2'),
        HB_SCRIPT_GUJARATI => hb_tag(b'g', b'j', b'r', b'2'),
        HB_SCRIPT_GURMUKHI => hb_tag(b'g', b'u', b'r', b'2'),
        HB_SCRIPT_KANNADA => hb_tag(b'k', b'n', b'd', b'2'),
        HB_SCRIPT_MALAYALAM => hb_tag(b'm', b'l', b'm', b'2'),
        HB_SCRIPT_ORIYA => hb_tag(b'o', b'r', b'y', b'2'),
        HB_SCRIPT_TAMIL => hb_tag(b't', b'm', b'l', b'2'),
        HB_SCRIPT_TELUGU => hb_tag(b't', b'e', b'l', b'2'),
        HB_SCRIPT_MYANMAR => hb_tag(b'm', b'y', b'm', b'2'),
        _ => HB_OT_TAG_DEFAULT_SCRIPT,
    }
}

/// `hb_ot_new_tag_to_script`
fn hb_ot_new_tag_to_script(tag: HbTag) -> HbScript {
    const BNG2: HbTag = hb_tag(b'b', b'n', b'g', b'2');
    const DEV2: HbTag = hb_tag(b'd', b'e', b'v', b'2');
    const GJR2: HbTag = hb_tag(b'g', b'j', b'r', b'2');
    const GUR2: HbTag = hb_tag(b'g', b'u', b'r', b'2');
    const KND2: HbTag = hb_tag(b'k', b'n', b'd', b'2');
    const MLM2: HbTag = hb_tag(b'm', b'l', b'm', b'2');
    const ORY2: HbTag = hb_tag(b'o', b'r', b'y', b'2');
    const TML2: HbTag = hb_tag(b't', b'm', b'l', b'2');
    const TEL2: HbTag = hb_tag(b't', b'e', b'l', b'2');
    const MYM2: HbTag = hb_tag(b'm', b'y', b'm', b'2');
    match tag {
        BNG2 => HB_SCRIPT_BENGALI,
        DEV2 => HB_SCRIPT_DEVANAGARI,
        GJR2 => HB_SCRIPT_GUJARATI,
        GUR2 => HB_SCRIPT_GURMUKHI,
        KND2 => HB_SCRIPT_KANNADA,
        MLM2 => HB_SCRIPT_MALAYALAM,
        ORY2 => HB_SCRIPT_ORIYA,
        TML2 => HB_SCRIPT_TAMIL,
        TEL2 => HB_SCRIPT_TELUGU,
        MYM2 => HB_SCRIPT_MYANMAR,
        _ => HB_SCRIPT_UNKNOWN,
    }
}

/*
 * Complete list at:
 * https://docs.microsoft.com/en-us/typography/opentype/spec/scripttags
 *
 * Most of the script tags are the same as the ISO 15924 tag but lowercased.
 * So we just do that, and handle the exceptional cases in a switch.
 */

/// `hb_ot_all_tags_from_script`
fn hb_ot_all_tags_from_script(script: HbScript, count: &mut usize, tags: &mut [HbTag]) {
    let mut i = 0;

    let new_tag = hb_ot_new_tag_from_script(script);
    if new_tag != HB_OT_TAG_DEFAULT_SCRIPT {
        /* HB_SCRIPT_MYANMAR maps to 'mym2', but there is no 'mym3'. */
        if new_tag != hb_tag(b'm', b'y', b'm', b'2') {
            tags[i] = new_tag | b'3' as u32;
            i += 1;
        }
        if *count > i {
            tags[i] = new_tag;
            i += 1;
        }
    }

    if *count > i {
        let old_tag = hb_ot_old_tag_from_script(script);
        if old_tag != HB_OT_TAG_DEFAULT_SCRIPT {
            tags[i] = old_tag;
            i += 1;
        }
    }

    *count = i;
}

/// `hb_ot_tag_to_script`: converts a script tag to an `HbScript`.
pub fn hb_ot_tag_to_script(tag: HbTag) -> HbScript {
    let digit = (tag & 0x000000FF) as u8;
    if digit == b'2' || digit == b'3' {
        return hb_ot_new_tag_to_script(tag & 0xFFFFFF32);
    }

    hb_ot_old_tag_to_script(tag)
}

/* hb_language_t */

/// `subtag_matches`
fn subtag_matches(s: &[u8], mut lang_str: usize, limit: usize, subtag: &[u8]) -> bool {
    let subtag_len = subtag.len();
    if (limit.wrapping_sub(lang_str) as u32) < subtag_len as u32 {
        return false;
    }

    loop {
        let Some(p) = strstr_from(s, lang_str, subtag) else {
            return false;
        };
        if p >= limit {
            return false;
        }
        if !is_alnum(at(s, p + subtag_len)) {
            return true;
        }
        lang_str = p + subtag_len;
    }
}

/// `lang_matches`
fn lang_matches(s: &[u8], lang_str: usize, limit: usize, spec: &[u8]) -> bool {
    let spec_len = spec.len();
    /* Same as hb_language_matches(); duplicated. */

    if (limit.wrapping_sub(lang_str) as u32) < spec_len as u32 {
        return false;
    }

    strncmp_eq(s, lang_str, spec, spec_len)
        && (at(s, lang_str + spec_len) == 0 || at(s, lang_str + spec_len) == b'-')
}

/// The binary search of `LangTag`s (`hb_sorted_array (...).bfind`).
fn lang_tags_bfind(ot_languages: &[LangTag], lang_tag: HbTag) -> Option<usize> {
    let mut min: i32 = 0;
    let mut max: i32 = ot_languages.len() as i32 - 1;
    while min <= max {
        let mid = ((min as u32 + max as u32) / 2) as i32;
        let language = ot_languages[mid as usize].language;
        let c = if lang_tag < language {
            -1
        } else if lang_tag > language {
            1
        } else {
            0
        };
        if c < 0 {
            max = mid - 1;
        } else if c > 0 {
            min = mid + 1;
        } else {
            return Some(mid as usize);
        }
    }
    None
}

/// `hb_ot_tags_from_complex_language` (generated in C; here, the rules of
/// `hb_ot_tag_table.rs`)
fn hb_ot_tags_from_complex_language(
    s: &[u8],
    limit: usize,
    count: &mut usize,
    tags: &mut [HbTag],
) -> bool {
    let lang_str = 0;
    let found = |tags: &mut [HbTag], count: &mut usize, possible_tags: &[HbTag]| {
        if possible_tags.len() == 1 {
            tags[0] = possible_tags[0];
            *count = 1;
        } else {
            let mut i = 0;
            while i < possible_tags.len() && i < *count {
                tags[i] = possible_tags[i];
                i += 1;
            }
            *count = i;
        }
    };

    'out: {
        if limit - lang_str >= 7 {
            let p = strchr_from(s, lang_str, b'-');
            let p = match p {
                Some(p) if p < limit && limit - p >= 5 => p,
                _ => break 'out,
            };
            for (subtag, possible_tags) in COMPLEX_SUBTAG_RULES.iter() {
                if subtag_matches(s, p, limit, subtag) {
                    found(tags, count, possible_tags);
                    return true;
                }
            }
        }
    }
    /* out: */
    let first = at(s, lang_str);
    for (c, cond, possible_tags) in COMPLEX_RULES.iter() {
        if *c != first {
            continue;
        }
        let m = match *cond {
            ComplexCond::Strcmp(t) => {
                strlen_from(s, lang_str + 1) == t.len() && strncmp_eq(s, lang_str + 1, t, t.len())
            }
            ComplexCond::LangMatches(t) => lang_matches(s, lang_str + 1, limit, t),
            ComplexCond::StrncmpSubtag(a, b) => {
                strncmp_eq(s, lang_str + 1, a, a.len()) && subtag_matches(s, lang_str, limit, b)
            }
        };
        if m {
            found(tags, count, possible_tags);
            return true;
        }
    }
    false
}

/// `hb_ot_tags_from_language`
fn hb_ot_tags_from_language(
    s: &[u8],
    mut lang_str: usize,
    limit: usize,
    count: &mut usize,
    tags: &mut [HbTag],
) {
    /* Check for matches of multiple subtags. */
    if hb_ot_tags_from_complex_language(s, limit, count, tags) {
        return;
    }

    /* Find a language matching in the first component. */
    let mut dash_s = strchr_from(s, lang_str, b'-');
    {
        if let Some(d) = dash_s {
            if limit - lang_str >= 6 {
                let extlang_end = strchr_from(s, d + 1, b'-');
                /* If there is an extended language tag, use it. */
                let len = match extlang_end {
                    Some(e) => e - d - 1,
                    None => strlen_from(s, d + 1),
                };
                if len == 3 && is_alpha(at(s, d + 1)) {
                    lang_str = d + 1;
                }
            }
        }
        let mut ot_languages: &[LangTag] = &[];
        let dash = strchr_from(s, lang_str, b'-');
        let first_len = match dash {
            Some(d) => d - lang_str,
            None => limit.wrapping_sub(lang_str),
        };
        if first_len == 2 {
            ot_languages = &ot_languages2;
        } else if first_len == 3 {
            ot_languages = &ot_languages3;
        }

        /* (hb_tag_from_string reads at most 4 characters, up to the NUL) */
        let start = lang_str.min(s.len());
        let lang_tag = hb_tag_from_string(&s[start..(start + first_len.min(4)).min(s.len())]);

        /* (C first tries the index of the last search: a cache) */
        if let Some(mut tag_idx) = lang_tags_bfind(ot_languages, lang_tag) {
            while tag_idx != 0
                && ot_languages[tag_idx].language == ot_languages[tag_idx - 1].language
            {
                tag_idx -= 1;
            }
            let mut i = 0;
            while i < *count
                && tag_idx + i < ot_languages.len()
                && ot_languages[tag_idx + i].tag != HB_TAG_NONE
                && ot_languages[tag_idx + i].language == ot_languages[tag_idx].language
            {
                tags[i] = ot_languages[tag_idx + i].tag;
                i += 1;
            }

            *count = i;
            return;
        }
    }

    let sp = *dash_s.get_or_insert_with(|| lang_str + strlen_from(s, lang_str));
    if sp.wrapping_sub(lang_str) == 3 {
        /* Assume it's ISO-639-3 and upper-case and use it. */
        tags[0] = hb_tag_from_string(&s[lang_str..sp]) & !0x20202000;
        *count = 1;
        return;
    }

    *count = 0;
}

/// `parse_private_use_subtag`
fn parse_private_use_subtag(
    s: &[u8],
    private_use_subtag: Option<usize>,
    count: Option<&mut usize>,
    tags: Option<&mut [HbTag]>,
    prefix: &[u8],
    normalize: fn(u8) -> u8,
) -> bool {
    let (Some(private_use_subtag), Some(count), Some(tags)) = (private_use_subtag, count, tags)
    else {
        return false;
    };
    if *count == 0 {
        return false;
    }

    let Some(mut p) = strstr_from(s, private_use_subtag, prefix) else {
        return false;
    };

    let mut tag = [0u8; 4];
    let mut i;
    p += prefix.len();
    if at(s, p) == b'-' {
        p += 1;
        i = 0;
        while i < 8 && at(s, p + i).is_ascii_hexdigit() {
            let c = (at(s, p + i) as char).to_digit(16).unwrap_or(0) as u8;
            if i % 2 == 0 {
                tag[i / 2] = c << 4;
            } else {
                tag[i / 2] = tag[i / 2].wrapping_add(c);
            }
            i += 1;
        }
        if i != 8 {
            return false;
        }
    } else {
        i = 0;
        while i < 4 && is_alnum(at(s, p + i)) {
            tag[i] = normalize(at(s, p + i));
            i += 1;
        }
        if i == 0 {
            return false;
        }

        while i < 4 {
            tag[i] = b' ';
            i += 1;
        }
    }
    tags[0] = hb_tag(tag[0], tag[1], tag[2], tag[3]);
    if (tags[0] & 0xDFDFDFDF) == HB_OT_TAG_DEFAULT_SCRIPT {
        tags[0] ^= !0xDFDFDFDF;
    }
    *count = 1;
    true
}

/// `hb_ot_tags_from_script_and_language`: converts an `HbScript` and an
/// `HbLanguage` to script and language tags (`script_tags` and
/// `language_tags` hold at most their slices' lengths; the counts of the
/// tags found are returned).
pub fn hb_ot_tags_from_script_and_language(
    script: HbScript,
    language: HbLanguage,
    script_tags: Option<&mut [HbTag]>,
    language_tags: Option<&mut [HbTag]>,
) -> (usize, usize) {
    let mut script_count = script_tags.as_ref().map_or(0, |t| t.len());
    let mut language_count = language_tags.as_ref().map_or(0, |t| t.len());
    let has_script = script_tags.is_some();
    let has_language = language_tags.is_some();
    let mut script_tags = script_tags;
    let mut language_tags = language_tags;

    let mut needs_script = true;

    match hb_language_to_string(language) {
        None => {
            if has_language && language_count != 0 {
                language_count = 0;
            }
        }
        Some(lang) => {
            let s = lang.as_bytes();
            let lang_str = 0;
            let mut limit = None;
            let mut private_use_subtag = None;
            if at(s, 0) == b'x' && at(s, 1) == b'-' {
                private_use_subtag = Some(lang_str);
            } else {
                let mut p = lang_str + 1;
                while at(s, p) != 0 {
                    if at(s, p - 1) == b'-' && at(s, p + 1) == b'-' {
                        if at(s, p) == b'x' {
                            private_use_subtag = Some(p);
                            if limit.is_none() {
                                limit = Some(p - 1);
                            }
                            break;
                        } else if limit.is_none() {
                            limit = Some(p - 1);
                        }
                    }
                    p += 1;
                }
                if limit.is_none() {
                    limit = Some(p);
                }
            }

            needs_script = !parse_private_use_subtag(
                s,
                private_use_subtag,
                has_script.then_some(&mut script_count),
                script_tags.as_deref_mut(),
                b"-hbsc",
                |c| c.to_ascii_lowercase(),
            );
            let needs_language = !parse_private_use_subtag(
                s,
                private_use_subtag,
                has_language.then_some(&mut language_count),
                language_tags.as_deref_mut(),
                b"-hbot",
                |c| c.to_ascii_uppercase(),
            );

            if needs_language && language_count != 0 {
                if let Some(tags) = language_tags.as_deref_mut() {
                    /* (C's limit stays NULL for "x-" private use languages) */
                    let limit = limit.unwrap_or(0);
                    hb_ot_tags_from_language(s, lang_str, limit, &mut language_count, tags);
                }
            }
        }
    }

    if needs_script && script_count != 0 {
        if let Some(tags) = script_tags.as_deref_mut() {
            hb_ot_all_tags_from_script(script, &mut script_count, tags);
        }
    }

    (script_count, language_count)
}

/// `hb_ot_tag_to_language`: converts a language tag to an `HbLanguage`.
pub fn hb_ot_tag_to_language(tag: HbTag) -> HbLanguage {
    if tag == HB_OT_TAG_DEFAULT_LANGUAGE {
        return HB_LANGUAGE_INVALID;
    }

    {
        if let Some((_, l)) = AMBIGUOUS_TAGS.iter().find(|(t, _)| *t == tag) {
            return hb_language_from_string(l.as_bytes());
        }
    }

    for l in ot_languages2.iter() {
        if l.tag == tag {
            let buf = hb_tag_to_string(l.language);
            return hb_language_from_string(&buf[..2]);
        }
    }

    for l in ot_languages3.iter() {
        if l.tag == tag {
            let buf = hb_tag_to_string(l.language);
            return hb_language_from_string(&buf[..3]);
        }
    }

    /* Return a custom language in the form of "x-hbot-AABBCCDD".
     * If it's three letters long, also guess it's ISO 639-3 and lower-case and
     * prepend it (if it's not a registered tag, the private use subtags will
     * ensure that calling hb_ot_tag_from_language on the result will still return
     * the same tag as the original tag).
     */
    {
        let mut buf = String::new();
        let b = |sh: u32| ((tag >> sh) & 0xFF) as u8;
        if is_alpha(b(24)) && is_alpha(b(16)) && is_alpha(b(8)) && b(0) == b' ' {
            buf.push(b(24).to_ascii_lowercase() as char);
            buf.push(b(16).to_ascii_lowercase() as char);
            buf.push(b(8).to_ascii_lowercase() as char);
            buf.push('-');
        }
        buf.push_str(&format!("x-hbot-{:08x}", tag));
        hb_language_from_string(buf.as_bytes())
    }
}
