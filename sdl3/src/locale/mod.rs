// Rust translation of src/locale/SDL_locale.c, SDL_syslocale.h,
// src/locale/unix/SDL_syslocale.c, src/locale/dummy/SDL_syslocale.c and
// include/SDL3/SDL_locale.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The user's preferred locales.
//!
//! The Unix backend (from the `LANG` and `LANGUAGE` environment variables)
//! is used on Unix systems other than Apple's, Android and Haiku; elsewhere
//! the dummy backend reports that the operation is unsupported until the
//! platform layer arrives. `SDL_HINT_PREFERRED_LOCALES` overrides both.

use crate::error::Result;
use crate::hints;

/// A struct to provide locale data. Translation of `SDL_Locale`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Locale {
    /// A language name, like "en" for English.
    pub language: String,
    /// A country, like "US" for America. Can be `None`.
    pub country: Option<String>,
}

/// `SDL_isspace()`.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// `SDL_strlcat()` into a buffer of `buflen` bytes.
fn strlcat(dst: &mut String, src: &str, buflen: usize) {
    let available = buflen.saturating_sub(1).saturating_sub(dst.len());
    let mut n = src.len().min(available);
    while !src.is_char_boundary(n) {
        n -= 1;
    }
    dst.push_str(&src[..n]);
}

/// Translation of `build_locales_from_csv_string()`.
///
/// Upstream counts the commas plus one, and starts each language one byte
/// past its first character, so an empty entry swallows the comma that
/// follows it and a trailing comma reports a slot with no language. Here
/// each entry starts at its first character and only the entries that have
/// a language are reported.
fn build_locales_from_csv_string(csv: &str) -> Vec<Locale> {
    let bytes = csv.as_bytes();
    let mut ptr = bytes
        .iter()
        .position(|&c| !is_space(c))
        .unwrap_or(bytes.len());
    if ptr == bytes.len() {
        return Vec::new(); // nothing to report
    }

    // (start, end) of the language and of the country of each entry
    type Span = Option<(usize, usize)>;
    let mut entries: Vec<(Span, Span)> = Vec::new();
    loop {
        // parse out the string
        while ptr < bytes.len() && is_space(bytes[ptr]) {
            ptr += 1; // skip whitespace.
        }

        if ptr == bytes.len() {
            break;
        }
        entries.push((None, None));
        let entry = entries.len() - 1;
        let mut field_start = ptr;
        let mut in_country = false;
        let mut field_end = None;
        loop {
            let ch = bytes.get(ptr).copied().unwrap_or(0);
            if ch == b'_' {
                let end = field_end.take().unwrap_or(ptr);
                if in_country {
                    entries[entry].1 = Some((field_start, end));
                } else {
                    entries[entry].0 = Some((field_start, end));
                }
                ptr += 1;
                field_start = ptr;
                in_country = true;
            } else if is_space(ch) {
                // trim ending whitespace and keep going.
                field_end.get_or_insert(ptr);
                ptr += 1;
            } else if ch == b',' || ch == 0 {
                let end = field_end.take().unwrap_or(ptr);
                if in_country {
                    entries[entry].1 = Some((field_start, end));
                } else {
                    entries[entry].0 = Some((field_start, end));
                }
                if ch == b',' {
                    ptr += 1;
                }
                break;
            } else {
                ptr += 1; // just keep going, still a valid string
            }
        }
        // only report the entries that have a language
        if !entries[entry].0.is_some_and(|(start, end)| start < end) {
            entries.pop();
        }
    }

    let text = |span: (usize, usize)| String::from_utf8_lossy(&bytes[span.0..span.1]).into_owned();
    entries
        .into_iter()
        .map(|(language, country)| Locale {
            language: language.map(text).unwrap_or_default(),
            country: country.map(text),
        })
        .collect()
}

/// Report the user's preferred locale, most preferred first. Fails if the
/// locale can't be determined; an empty list means none is known.
/// Translation of `SDL_GetPreferredLocales()`.
pub fn preferred_locales() -> Result<Vec<Locale>> {
    const LOCBUF_SIZE: usize = 128; // enough for 21 "xx_YY," language strings.

    let mut locbuf = String::new();
    let mut error = None;
    match hints::get(hints::PREFERRED_LOCALES) {
        Some(hint) => strlcat(&mut locbuf, &hint, LOCBUF_SIZE),
        None => {
            if let Err(e) = sys_get_preferred_locales(&mut locbuf, LOCBUF_SIZE) {
                error = Some(e);
            }
        }
    }
    let locales = build_locales_from_csv_string(&locbuf);
    match error {
        Some(e) if locales.is_empty() => Err(e),
        _ => Ok(locales),
    }
}

#[cfg(all(
    unix,
    not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
))]
mod sys {
    use super::strlcat;

    /// Translation of `normalize_locale_str()`.
    fn normalize_locale_str(dst: &mut String, str: &str, buflen: usize) {
        // chop off encoding if specified.
        let str = str.split('.').next().unwrap_or("");
        // chop off extra bits if specified.
        let str = str.split('@').next().unwrap_or("");

        // The "C" locale isn't useful for our needs, ignore it if you see it.
        if str == "C" {
            return;
        }

        if !str.is_empty() {
            if !dst.is_empty() {
                strlcat(dst, ",", buflen); // SDL has these split by commas
            }
            strlcat(dst, str, buflen);
        }
    }

    /// Translation of `normalize_locales()`.
    fn normalize_locales(dst: &mut String, src: &str, buflen: usize) {
        // entries are separated by colons
        for entry in src.split(':') {
            normalize_locale_str(dst, entry, buflen);
        }
    }

    /// Translation of `SDL_SYS_GetPreferredLocales()` (Unix).
    pub(super) fn get_preferred_locales(buf: &mut String, buflen: usize) -> crate::Result<()> {
        // !!! FIXME: should we be using setlocale()? Or some D-Bus thing?
        let mut tmp = String::new();

        // LANG is the primary locale (maybe)
        if let Some(envr) = crate::stdlib::getenv("LANG") {
            strlcat(&mut tmp, &envr, buflen);
        }

        // fallback languages
        if let Some(envr) = crate::stdlib::getenv("LANGUAGE") {
            if !tmp.is_empty() {
                strlcat(&mut tmp, ":", buflen);
            }
            strlcat(&mut tmp, &envr, buflen);
        }

        if tmp.is_empty() {
            return Err(crate::Error::new("LANG environment variable isn't set"));
        }
        normalize_locales(buf, &tmp, buflen);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn normalize(src: &str) -> String {
        let mut dst = String::new();
        normalize_locales(&mut dst, src, 128);
        dst
    }
}

#[cfg(not(all(
    unix,
    not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
)))]
mod sys {
    /// Translation of `SDL_SYS_GetPreferredLocales()` (dummy).
    pub(super) fn get_preferred_locales(_buf: &mut String, _buflen: usize) -> crate::Result<()> {
        // dummy implementation. Caller already zero'd out buffer.
        Err(crate::Error::unsupported())
    }
}

fn sys_get_preferred_locales(buf: &mut String, buflen: usize) -> Result<()> {
    sys::get_preferred_locales(buf, buflen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(language: &str, country: Option<&str>) -> Locale {
        Locale {
            language: language.to_string(),
            country: country.map(str::to_string),
        }
    }

    #[test]
    fn csv_parsing() {
        assert_eq!(build_locales_from_csv_string("  "), vec![]);
        assert_eq!(
            build_locales_from_csv_string("en_US, fr ,de_CH"),
            vec![l("en", Some("US")), l("fr", None), l("de", Some("CH"))]
        );
        // A trailing comma adds no entry
        assert_eq!(
            build_locales_from_csv_string("pt_BR,"),
            vec![l("pt", Some("BR"))]
        );
        // An empty entry doesn't swallow the comma after it
        assert_eq!(
            build_locales_from_csv_string("en x_US,,fr"),
            vec![l("en", Some("US")), l("fr", None)]
        );
        assert_eq!(
            build_locales_from_csv_string("a_b_c"),
            vec![l("a", Some("c"))]
        );
        assert_eq!(build_locales_from_csv_string(",,"), vec![]);
    }

    /// The results of `SDL_GetPreferredLocales()` with these hint values,
    /// from upstream with `build_locales_from_csv_string()` fixed the same
    /// way (each entry starting at its first character, and only the entries
    /// with a language reported and counted).
    #[test]
    fn csv_matches_c() {
        let cases: [(&str, &str); 17] = [
            ("  ", ""),
            ("en_US, fr ,de_CH", "(en|US) (fr|-) (de|CH)"),
            ("pt_BR,", "(pt|BR)"),
            ("en x_US,,fr", "(en|US) (fr|-)"),
            ("a_b_c", "(a|c)"),
            (" _x", ""),
            ("x_ y_z,", "(x|z)"),
            ("ab\tcd_EF gh,ij", "(ab|EF) (ij|-)"),
            ("", ""),
            ("_", ""),
            (",", ""),
            ("a,b,c,d", "(a|-) (b|-) (c|-) (d|-)"),
            (",en_GB", "(en|GB)"),
            ("en , ,fr_CA", "(en|-) (fr|CA)"),
            ("_x,de", "(de|-)"),
            (",,", ""),
            ("fr_", "(fr|)"),
        ];
        for (csv, expected) in cases {
            let got: Vec<String> = build_locales_from_csv_string(csv)
                .iter()
                .map(|l| format!("({}|{})", l.language, l.country.as_deref().unwrap_or("-")))
                .collect();
            assert_eq!(got.join(" "), expected, "{csv:?}");
        }
    }

    #[test]
    fn hint_overrides() {
        let _l = crate::test_support::test_lock();
        hints::set(hints::PREFERRED_LOCALES, "ja_JP,en").unwrap();
        assert_eq!(
            preferred_locales().unwrap(),
            vec![l("ja", Some("JP")), l("en", None)]
        );
        hints::reset(hints::PREFERRED_LOCALES);
    }

    #[cfg(all(
        unix,
        not(any(target_vendor = "apple", target_os = "android", target_os = "haiku"))
    ))]
    #[test]
    fn unix_normalization() {
        assert_eq!(
            sys::normalize("en_US.UTF-8:fr_FR@euro:C:de"),
            "en_US,fr_FR,de"
        );
        assert_eq!(sys::normalize("C"), "");
    }
}
