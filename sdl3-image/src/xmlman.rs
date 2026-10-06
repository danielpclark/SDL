// Rust translation of src/xmlman.c and src/xmlman.h from SDL_image.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Simple XML manager written by Xen (@lordofxen) for managing XMP data of
//! image formats: the Dublin Core description, rights, title and creator
//! and the XMP creation date, read from an XMP packet and written into one.
//!
//! The text is bytes (C strings in upstream, which end at a NUL); the
//! values read are strings, invalid UTF-8 replaced.

/// 32 MB. Translation of `MAX_XML_CONTENT_LENGTH`.
const MAX_XML_CONTENT_LENGTH: usize = 32 * 1024 * 1024;

/// Translation of `escape()`.
fn escape(s: &str) -> String {
    let mut escaped_str = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => escaped_str.push_str("&lt;"),
            '>' => escaped_str.push_str("&gt;"),
            '&' => escaped_str.push_str("&amp;"),
            '\'' => escaped_str.push_str("&apos;"),
            '"' => escaped_str.push_str("&quot;"),
            _ => escaped_str.push(c),
        }
    }
    escaped_str
}

/// Translation of `unescape_inplace()`: `s` holds the string and its
/// terminator (`len` upstream is its length); returns the unescaped
/// length, the string ending there.
fn unescape_inplace(s: &mut [u8]) -> usize {
    let len = s.len();
    if len == 0 {
        return 0;
    }

    let mut i = 0;
    let mut j = 0;
    while i < len && s[i] != 0 {
        if s[i] == b'&' && i + 3 < len {
            if s[i..].starts_with(b"&lt;") {
                s[j] = b'<';
                j += 1;
                i += 4;
            } else if s[i..].starts_with(b"&gt;") {
                s[j] = b'>';
                j += 1;
                i += 4;
            } else if i + 4 < len && s[i..].starts_with(b"&amp;") {
                s[j] = b'&';
                j += 1;
                i += 5;
            } else if i + 5 < len && s[i..].starts_with(b"&apos;") {
                s[j] = b'\'';
                j += 1;
                i += 6;
            } else if i + 5 < len && s[i..].starts_with(b"&quot;") {
                s[j] = b'"';
                j += 1;
                i += 6;
            } else {
                s[j] = s[i];
                j += 1;
                i += 1;
            }
        } else {
            s[j] = s[i];
            j += 1;
            i += 1;
        }
    }

    if j >= len {
        j = len - 1;
    }
    s[j] = 0;

    j
}

/// Translation of `find_char_in_bounds()`: the index of `c` in
/// `data[start..end]`.
fn find_char_in_bounds(data: &[u8], start: usize, end: usize, c: u8) -> Option<usize> {
    (start..end).find(|&i| data[i] == c)
}

/// Translation of `memcasecmp()`: whether the bytes are equal ignoring
/// ASCII case.
fn memcasecmp(s1: &[u8], s2: &[u8]) -> bool {
    s1.eq_ignore_ascii_case(s2)
}

/// Translation of `find_substr_in_bounds()`: the index of `needle` in
/// `data[start..end]`.
fn find_substr_in_bounds(
    data: &[u8],
    start: usize,
    end: usize,
    needle: &[u8],
    case_sensitive: bool,
) -> Option<usize> {
    let needle_len = needle.len();
    if needle_len == 0 {
        return None;
    }
    if end <= start || end - start < needle_len {
        return None;
    }

    let end_search = end - needle_len + 1;
    (start..end_search).find(|&p| {
        let hay = &data[p..p + needle_len];
        if case_sensitive {
            hay == needle
        } else {
            memcasecmp(hay, needle)
        }
    })
}

/// Translation of `find_tag_content()`: the start and end of the content
/// of the first `tag` element of `data[..data_end]`.
fn find_tag_content(
    data: &[u8],
    data_end: usize,
    tag: &str,
    case_sensitive: bool,
) -> Option<(usize, usize)> {
    // (SDL_snprintf() into char[256])
    let mut start_tag = format!("<{tag}").into_bytes();
    start_tag.truncate(255);
    let mut end_tag = format!("</{tag}>").into_bytes();
    end_tag.truncate(255);

    let start_tag_len = start_tag.len();
    let mut tag_start = 0usize;

    while tag_start < data_end {
        if (data_end - tag_start) >= 4 && data[tag_start..].starts_with(b"<!--") {
            let comment_end = find_substr_in_bounds(data, tag_start + 4, data_end, b"-->", true)?;
            tag_start = comment_end + 3;
            continue;
        }
        if (data_end - tag_start) >= 9 && data[tag_start..].starts_with(b"<![CDATA[") {
            let cdata_end = find_substr_in_bounds(data, tag_start + 9, data_end, b"]]>", true)?;
            tag_start = cdata_end + 3;
            continue;
        }

        tag_start = find_substr_in_bounds(data, tag_start, data_end, &start_tag, case_sensitive)?;

        let after_tag = tag_start + start_tag_len;
        if after_tag < data_end
            && ((9..=13).contains(&data[after_tag])
                || data[after_tag] == 32
                || data[after_tag] == b'>')
        {
            if let Some(content_start) = find_char_in_bounds(data, after_tag, data_end, b'>') {
                let content_start = content_start + 1;
                if let Some(content_end) =
                    find_substr_in_bounds(data, content_start, data_end, &end_tag, case_sensitive)
                {
                    return Some((content_start, content_end));
                }
            }
        }
        tag_start += start_tag_len;
    }
    None
}

/// Translation of `SDL_isspace()`.
fn isspace(c: u8) -> bool {
    crate::util::isspace(c)
}

/// A copy of `data[start..end]`, unescaped, as upstream's
/// `SDL_malloc()`/`SDL_memcpy()`/`unescape_inplace()` make it.
fn unescaped_copy(data: &[u8], start: usize, end: usize) -> String {
    let content_len = end - start + 1;
    let mut result = Vec::with_capacity(content_len);
    result.extend_from_slice(&data[start..end]);
    result.push(0);
    let n = unescape_inplace(&mut result);
    // (the string ends at its first NUL)
    let n = result[..n].iter().position(|&b| b == 0).unwrap_or(n);
    String::from_utf8_lossy(&result[..n]).into_owned()
}

/// Translation of `get_content_from_tag()`.
fn get_content_from_tag(data: &[u8], tag: &str) -> Option<String> {
    let len = data.len();
    if len == 0 {
        return None;
    }

    let data_end = len;
    let (mut content_start, mut content_end) = find_tag_content(data, data_end, tag, true)?;

    let alt_start = find_substr_in_bounds(data, content_start, content_end, b"<rdf:Alt>", true);
    if let Some(alt_start) = alt_start.filter(|&a| a < content_end) {
        let mut li_start = alt_start;
        let mut fallback: Option<(usize, usize)> = None;

        while let Some(found) =
            find_substr_in_bounds(data, li_start, content_end, b"<rdf:li", true)
        {
            li_start = found;
            let Some(li_content_start) = find_char_in_bounds(data, li_start, content_end, b'>')
            else {
                break;
            };
            let mut li_content_start = li_content_start + 1;

            let Some(mut li_end) =
                find_substr_in_bounds(data, li_content_start, content_end, b"</rdf:li>", true)
            else {
                break;
            };

            while li_content_start < li_end && isspace(data[li_content_start]) {
                li_content_start += 1;
            }
            while li_content_start < li_end && isspace(data[li_end - 1]) {
                li_end -= 1;
            }
            if li_content_start >= li_end {
                li_start = li_end;
                continue;
            }

            let default_lang = find_substr_in_bounds(
                data,
                li_start,
                li_content_start,
                b"xml:lang=\"x-default\"",
                false,
            );
            let en_us_lang =
                find_substr_in_bounds(data, li_start, li_content_start, b"xml:lang=\"en-us\"", false);

            if default_lang.is_some() || en_us_lang.is_some() {
                return Some(unescaped_copy(data, li_content_start, li_end));
            }

            if fallback.is_none() {
                fallback = Some((li_content_start, li_end - li_content_start));
            }

            li_start = li_end;
        }

        if let Some((mut fallback_content, mut fallback_len)) = fallback {
            // (fallback_content < fallback_content + fallback_len)
            while fallback_len > 0 && isspace(data[fallback_content]) {
                fallback_content += 1;
                fallback_len -= 1;
            }
            while fallback_len > 0 && isspace(data[fallback_content + fallback_len - 1]) {
                fallback_len -= 1;
            }

            if fallback_len == 0 {
                return None;
            }

            return Some(unescaped_copy(
                data,
                fallback_content,
                fallback_content + fallback_len,
            ));
        }
        return None;
    }

    let seq_start = find_substr_in_bounds(data, content_start, content_end, b"<rdf:Seq>", true);
    if let Some(seq_start) = seq_start.filter(|&s| s < content_end) {
        if let Some(li_start) = find_substr_in_bounds(data, seq_start, content_end, b"<rdf:li", true)
        {
            if let Some(li_content_start) = find_char_in_bounds(data, li_start, content_end, b'>') {
                let mut li_content_start = li_content_start + 1;
                if let Some(mut li_end) =
                    find_substr_in_bounds(data, li_content_start, content_end, b"</rdf:li>", true)
                {
                    while li_content_start < li_end && isspace(data[li_content_start]) {
                        li_content_start += 1;
                    }
                    while li_content_start < li_end && isspace(data[li_end - 1]) {
                        li_end -= 1;
                    }
                    if li_content_start >= li_end {
                        return None;
                    }

                    return Some(unescaped_copy(data, li_content_start, li_end));
                }
            }
        }
        return None;
    }

    while content_start < content_end && isspace(data[content_start]) {
        content_start += 1;
    }
    while content_start < content_end && isspace(data[content_end - 1]) {
        content_end -= 1;
    }
    if content_start >= content_end {
        return None;
    }

    Some(unescaped_copy(data, content_start, content_end))
}

/// Translation of `get_tag()`.
fn get_tag(data: &[u8], tag: &str) -> Option<String> {
    if data.len() < 4 {
        return None;
    }

    get_content_from_tag(data, tag)
}

/// Translation of `__xmlman_GetXMPDescription()`.
pub(crate) fn get_xmp_description(data: &[u8]) -> Option<String> {
    get_tag(data, "dc:description")
}

/// Translation of `__xmlman_GetXMPCopyright()`.
pub(crate) fn get_xmp_copyright(data: &[u8]) -> Option<String> {
    get_tag(data, "dc:rights")
}

/// Translation of `__xmlman_GetXMPTitle()`.
pub(crate) fn get_xmp_title(data: &[u8]) -> Option<String> {
    get_tag(data, "dc:title")
}

/// Translation of `__xmlman_GetXMPCreator()`.
pub(crate) fn get_xmp_creator(data: &[u8]) -> Option<String> {
    get_tag(data, "dc:creator")
}

/// Translation of `__xmlman_GetXMPCreateDate()`.
pub(crate) fn get_xmp_create_date(data: &[u8]) -> Option<String> {
    get_tag(data, "xmp:CreateDate")
}

/// `SDL_strnlen(s, MAX_XML_CONTENT_LENGTH)` of a string: its length, at
/// most the maximum.
fn strnlen(s: &str) -> usize {
    s.len().min(MAX_XML_CONTENT_LENGTH)
}

/// An XMP packet with an RDF description of the given properties, or
/// `None` without any. Translation of
/// `__xmlman_ConstructXMPWithRDFDescription()` (`*outlen` is the length).
#[allow(dead_code)] // (for the WebP and AVIF encoders)
pub(crate) fn construct_xmp_with_rdf_description(
    dctitle: Option<&str>,
    dccreator: Option<&str>,
    dcdescription: Option<&str>,
    dcrights: Option<&str>,
    xmpcreatedate: Option<&str>,
) -> Option<Vec<u8>> {
    if dctitle.is_none()
        && dccreator.is_none()
        && dcdescription.is_none()
        && dcrights.is_none()
        && xmpcreatedate.is_none()
    {
        return None;
    }

    let dctitle = dctitle.map(escape);
    let dccreator = dccreator.map(escape);
    let dcdescription = dcdescription.map(escape);
    let dcrights = dcrights.map(escape);
    let xmpcreatedate = xmpcreatedate.map(escape);

    let header = "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
        <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n  \
        <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n    \
        <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n";

    let title_prefix = "      <dc:title>\n        <rdf:Alt>\n          <rdf:li xml:lang=\"x-default\">";
    let title_suffix = "</rdf:li>\n        </rdf:Alt>\n      </dc:title>\n";

    let creator_prefix = "      <dc:creator>\n        <rdf:Seq>\n          <rdf:li>";
    let creator_suffix = "</rdf:li>\n        </rdf:Seq>\n      </dc:creator>\n";

    let description_prefix =
        "      <dc:description>\n        <rdf:Alt>\n          <rdf:li xml:lang=\"x-default\">";
    let description_suffix = "</rdf:li>\n        </rdf:Alt>\n      </dc:description>\n";

    let rights_prefix = "      <dc:rights>\n        <rdf:Alt>\n          <rdf:li xml:lang=\"x-default\">";
    let rights_suffix = "</rdf:li>\n        </rdf:Alt>\n      </dc:rights>\n";

    let createdate_prefix = "      <xmp:CreateDate>";
    let createdate_suffix = "</xmp:CreateDate>\n";

    let footer = "    </rdf:Description>\n  </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>";

    let mut total_size = strnlen(header) + strnlen(footer) + 1;
    let parts = [
        (&dctitle, title_prefix, title_suffix),
        (&dccreator, creator_prefix, creator_suffix),
        (&dcdescription, description_prefix, description_suffix),
        (&dcrights, rights_prefix, rights_suffix),
        (&xmpcreatedate, createdate_prefix, createdate_suffix),
    ];
    for (value, prefix, suffix) in &parts {
        if let Some(value) = value {
            total_size += strnlen(prefix) + strnlen(value) + strnlen(suffix);
        }
    }

    let mut buffer = Vec::new();
    buffer.try_reserve_exact(total_size).ok()?;
    buffer.extend_from_slice(header.as_bytes());
    for (value, prefix, suffix) in &parts {
        if let Some(value) = value {
            buffer.extend_from_slice(prefix.as_bytes());
            buffer.extend_from_slice(value.as_bytes());
            buffer.extend_from_slice(suffix.as_bytes());
        }
    }
    buffer.extend_from_slice(footer.as_bytes());
    // (SDL_snprintf() into total_size bytes, with the terminator: the text
    // is cut there)
    buffer.truncate(total_size - 1);
    Some(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_it_writes() {
        let xmp = construct_xmp_with_rdf_description(
            Some("a <title>"),
            Some("someone & co"),
            Some("it's \"described\""),
            None,
            Some("2024-01-02"),
        )
        .unwrap();
        assert_eq!(get_xmp_title(&xmp).as_deref(), Some("a <title>"));
        assert_eq!(get_xmp_creator(&xmp).as_deref(), Some("someone & co"));
        assert_eq!(
            get_xmp_description(&xmp).as_deref(),
            Some("it's \"described\"")
        );
        assert_eq!(get_xmp_copyright(&xmp), None);
        assert_eq!(get_xmp_create_date(&xmp).as_deref(), Some("2024-01-02"));
        assert_eq!(construct_xmp_with_rdf_description(None, None, None, None, None), None);
    }

    #[test]
    fn alternatives_comments_and_plain_content() {
        let xmp = b"<!-- <dc:title>no</dc:title> --><dc:title><rdf:Alt>\
            <rdf:li xml:lang='fr'> bonjour </rdf:li>\
            <rdf:li xml:lang=\"EN-US\">hello</rdf:li></rdf:Alt></dc:title>\
            <dc:rights><rdf:Alt><rdf:li xml:lang='de'>  frei </rdf:li></rdf:Alt></dc:rights>\
            <xmp:CreateDate>  2020 &amp; &lt;b&gt;  </xmp:CreateDate>";
        assert_eq!(get_xmp_title(xmp).as_deref(), Some("hello"));
        assert_eq!(get_xmp_copyright(xmp).as_deref(), Some("frei"));
        assert_eq!(get_xmp_create_date(xmp).as_deref(), Some("2020 & <b>"));
        assert_eq!(get_xmp_creator(xmp), None);
        assert_eq!(get_xmp_title(b"<dc:title"), None);
        assert_eq!(get_xmp_title(b"ab"), None);
    }
}
