// Rust translation of src/locale/SDL_locale.c, SDL_syslocale.h,
// src/locale/unix/SDL_syslocale.c, src/locale/windows/SDL_syslocale.c,
// src/locale/macos/SDL_syslocale.m, src/locale/dummy/SDL_syslocale.c and
// include/SDL3/SDL_locale.h from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The user's preferred locales.
//!
//! The Unix backend (from the `LANG` and `LANGUAGE` environment variables)
//! is used on Unix systems other than Apple's, Android and Haiku, the
//! Windows backend (`GetUserPreferredUILanguages()`, or the user locale's
//! language and country where that's missing) on Windows, and the macOS
//! one (`NSLocale.preferredLanguages`) on Apple platforms; elsewhere the
//! dummy backend reports that the operation is unsupported until the
//! platform layer arrives. `SDL_HINT_PREFERRED_LOCALES` overrides them.

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
    not(any(
        target_vendor = "apple",
        target_os = "android",
        target_os = "haiku",
        target_os = "emscripten"
    ))
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

#[cfg(windows)]
mod sys {
    use super::strlcat;
    use std::sync::OnceLock;
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Globalization::{
        GetLocaleInfoA, LOCALE_SISO3166CTRYNAME, LOCALE_SISO639LANGNAME, LOCALE_USER_DEFAULT,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

    type PfnGetUserPreferredUILanguages =
        unsafe extern "system" fn(u32, *mut u32, *mut u16, *mut u32) -> BOOL;
    const MUI_LANGUAGE_NAME: u32 = 0x8;

    /// `pGetUserPreferredUILanguages`, looked up in kernel32.dll once.
    fn p_get_user_preferred_ui_languages() -> Option<PfnGetUserPreferredUILanguages> {
        static P: OnceLock<Option<PfnGetUserPreferredUILanguages>> = OnceLock::new();
        *P.get_or_init(|| {
            let name = crate::core::windows::utf8_to_wide("kernel32.dll");
            // SAFETY: name is NUL-terminated; kernel32 stays loaded.
            unsafe {
                let kernel32 = GetModuleHandleW(name.as_ptr());
                if kernel32.is_null() {
                    return None;
                }
                let f = GetProcAddress(kernel32, c"GetUserPreferredUILanguages".as_ptr().cast())?;
                // SAFETY: the export has this signature.
                Some(std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    PfnGetUserPreferredUILanguages,
                >(f))
            }
        })
    }

    /// One `GetLocaleInfoA()` string into a 16-byte buffer, with its
    /// return value (0 on failure).
    fn get_locale_info_a(lctype: u32) -> (i32, String) {
        let mut buf = [0u8; 16];
        // SAFETY: buf holds the given number of bytes.
        let rc = unsafe {
            GetLocaleInfoA(
                LOCALE_USER_DEFAULT,
                lctype,
                buf.as_mut_ptr(),
                buf.len() as i32,
            )
        };
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        (rc, String::from_utf8_lossy(&buf[..end]).into_owned())
    }

    /// This is the fallback for WinXP...one language, not a list.
    /// Translation of `SDL_SYS_GetPreferredLocales_winxp()`.
    fn get_preferred_locales_winxp(buf: &mut String, buflen: usize) -> crate::Result<()> {
        let (langrc, lang) = get_locale_info_a(LOCALE_SISO639LANGNAME);
        let (ctryrc, country) = get_locale_info_a(LOCALE_SISO3166CTRYNAME);

        /* Win95 systems will fail, because they don't have LOCALE_SISO*NAME ... */
        if langrc == 0 {
            Err(crate::Error::new("Couldn't obtain language info"))
        } else {
            let s = if ctryrc != 0 {
                format!("{lang}_{country}")
            } else {
                lang
            };
            strlcat(buf, &s, buflen); // (snprintf() into the empty buffer)
            Ok(())
        }
    }

    /// This works on Windows Vista and later.
    /// Translation of `SDL_SYS_GetPreferredLocales_vista()`.
    fn get_preferred_locales_vista(
        p_get_user_preferred_ui_languages: PfnGetUserPreferredUILanguages,
        buf: &mut String,
        buflen: usize,
    ) -> crate::Result<()> {
        let mut numlangs: u32 = 0;
        let mut wbuflen: u32 = 0;

        // SAFETY: with no buffer, only the counts are written.
        unsafe {
            p_get_user_preferred_ui_languages(
                MUI_LANGUAGE_NAME,
                &mut numlangs,
                std::ptr::null_mut(),
                &mut wbuflen,
            )
        };

        let mut wbuf = vec![0u16; wbuflen as usize];

        // SAFETY: wbuf holds wbuflen UTF-16 units.
        let ok = unsafe {
            p_get_user_preferred_ui_languages(
                MUI_LANGUAGE_NAME,
                &mut numlangs,
                wbuf.as_mut_ptr(),
                &mut wbuflen,
            )
        } != 0;
        if !ok {
            let _ = get_preferred_locales_winxp(buf, buflen); // oh well, try the fallback.
        } else {
            let wbuflen = (wbuflen as usize).min(wbuf.len());
            buf.push_str(&super::ui_languages_to_csv(&wbuf[..wbuflen], buflen));
        }

        Ok(())
    }

    /// Translation of `SDL_SYS_GetPreferredLocales()` (Windows).
    pub(super) fn get_preferred_locales(buf: &mut String, buflen: usize) -> crate::Result<()> {
        match p_get_user_preferred_ui_languages() {
            None => get_preferred_locales_winxp(buf, buflen), // this is always available
            Some(f) => get_preferred_locales_vista(f, buf, buflen), // available on Vista and later.
        }
    }

    #[cfg(test)]
    pub(super) fn winxp() -> crate::Result<String> {
        let mut buf = String::new();
        get_preferred_locales_winxp(&mut buf, 128).map(|()| buf)
    }
}

/// The copy loop of `SDL_SYS_GetPreferredLocales_vista()`: a
/// `GetUserPreferredUILanguages()` list (NUL-separated and double-NUL
/// terminated, `wbuflen` units) into a `buflen`-byte buffer, with the
/// separators turned into commas and `-` into `_`.
///
/// Note (upstream): with an empty list (`wbuflen` 0), upstream's
/// `wbuflen - 1` wraps around and the loop reads past the buffer; here
/// there is nothing to copy.
#[cfg(any(windows, test))]
fn ui_languages_to_csv(wbuf: &[u16], buflen: usize) -> String {
    let endidx = buflen.min(wbuf.len().saturating_sub(1));
    let mut out = vec![0u8; endidx];
    let mut str_start = 0;
    for (i, &wch) in wbuf.iter().enumerate().take(endidx) {
        let ch = wch as u8; // these should all be low-ASCII, safe to cast
        if ch == b'\0' {
            out[i] = b','; // change null separators to commas
            str_start = i;
        } else if ch == b'-' {
            out[i] = b'_'; // change '-' to '_'
        } else {
            out[i] = ch; // copy through as-is.
        }
    }
    out.truncate(str_start); // terminate string, chop off final ','
    String::from_utf8_lossy(&out).into_owned()
}

/// The loop of `SDL_SYS_GetPreferredLocales()` (macOS) over `numlangs`
/// languages, `get_c_string(i, buflen)` standing for
/// `[nsstr getCString:buf maxLength:buflen encoding:NSASCIIStringEncoding]`
/// on the `i`th (`None` when it fails, which leaves upstream's buffer
/// empty where it reads it).
#[cfg(any(target_vendor = "apple", test))]
fn languages_to_csv(
    numlangs: usize,
    mut get_c_string: impl FnMut(usize, usize) -> Option<String>,
    mut buflen: usize,
) -> String {
    let mut buf = String::new();
    for i in 0..numlangs {
        let mut lang = get_c_string(i, buflen).unwrap_or_default();
        let len = lang.len();

        // convert '-' to '_'...
        //  These are always full lang-COUNTRY, so we search from the back,
        //  so things like zh-Hant-CN find the right '-' to convert.
        if let Some(ptr) = lang.rfind('-') {
            lang.replace_range(ptr..ptr + 1, "_");
        }

        if buflen <= len {
            break; // drop this one and stop, we can't fit anymore.
        }

        buf.push_str(&lang);
        buflen -= len;

        if i < numlangs - 1 {
            if buflen <= 1 {
                break; // out of room, stop looking.
            }
            buf.push(','); // add a comma between entries.
            buflen -= 1;
        }
    }
    buf
}

#[cfg(target_vendor = "apple")]
mod sys {
    use crate::core::apple::foundation::{ns_array_count, ns_array_get};
    use crate::core::apple::objc::{autoreleasepool, class, msg_send, Bool, Id, NSUInteger};

    /// `NSASCIIStringEncoding`.
    const NS_ASCII_STRING_ENCODING: NSUInteger = 1;

    /// `[nsstr getCString:buf maxLength:buflen encoding:NSASCIIStringEncoding]`:
    /// the string, if it converts to ASCII and fits in `buflen` bytes with
    /// its NUL.
    ///
    /// # Safety
    ///
    /// `nsstr` must be a valid `NSString`.
    unsafe fn get_ascii_c_string(nsstr: Id, buflen: usize) -> Option<String> {
        let mut buf = vec![0u8; buflen.max(1)];
        // SAFETY: the caller's contract; the buffer has `buflen` bytes.
        let ok: Bool = unsafe {
            msg_send![nsstr, getCString: buf.as_mut_ptr()
                             maxLength: buflen as NSUInteger
                             encoding: NS_ASCII_STRING_ENCODING]
        };
        if !ok.is_true() {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        buf.truncate(len);
        String::from_utf8(buf).ok()
    }

    /// Translation of `SDL_SYS_GetPreferredLocales()` (macOS).
    pub(super) fn get_preferred_locales(buf: &mut String, buflen: usize) -> crate::Result<()> {
        autoreleasepool(|| {
            // SAFETY: NSLocale's class property is an autoreleased NSArray
            // of NSStrings, read while this pool lives.
            unsafe {
                let languages: Id = msg_send![class!(NSLocale), preferredLanguages];
                let numlangs = ns_array_count(languages);
                let csv = super::languages_to_csv(
                    numlangs,
                    |i, buflen| get_ascii_c_string(ns_array_get(languages, i), buflen),
                    buflen,
                );
                buf.push_str(&csv);
            }
        });
        Ok(())
    }
}

/// The Emscripten backend: the browser's `navigator.languages`.
/// Translation of `locale/emscripten/SDL_syslocale.c`.
#[cfg(target_os = "emscripten")]
mod sys {
    /// Translation of `SDL_SYS_GetPreferredLocales()` (Emscripten).
    pub(super) fn get_preferred_locales(buf: &mut String, buflen: usize) -> crate::Result<()> {
        // (the caller's zeroed buffer, upstream's locbuf, on the stack: the
        // JavaScript takes its address as a signed number, so it must be
        // below 2 GB, as the stack is)
        let mut locbuf = [0u8; 128];
        let buflen = buflen.min(locbuf.len());
        // SAFETY: the JavaScript writes at most buflen bytes at the pointer.
        unsafe {
            sdl3_emscripten::locale::sdl3_get_preferred_locales(locbuf.as_mut_ptr().cast(), buflen)
        };
        // Note (upstream): a list exactly as long as the buffer has no NUL
        // after it, and C reads on past the buffer; this stops at its end.
        // The bytes are UTF-16 units cut to 8 bits: anything but ASCII
        // (which language tags are) isn't UTF-8, and is replaced here.
        let len = locbuf[..buflen]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(buflen);
        buf.push_str(&String::from_utf8_lossy(&locbuf[..len]));
        Ok(())
    }
}

#[cfg(not(any(
    windows,
    target_vendor = "apple",
    target_os = "emscripten",
    all(
        unix,
        not(any(
            target_vendor = "apple",
            target_os = "android",
            target_os = "haiku",
            target_os = "emscripten"
        ))
    )
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

    /// The copy loop of upstream's `SDL_SYS_GetPreferredLocales_vista()`,
    /// compiled from the C source, gives these for these lists.
    #[test]
    fn ui_languages_match_c() {
        let w = |s: &str| -> Vec<u16> {
            s.chars()
                .map(|c| if c == '|' { 0 } else { c as u16 })
                .collect()
        };
        let mut zh = w("zh-Hans-CN|de||");
        zh[10] = 0x100; // (its low byte is a NUL, a separator)
        let cases: [(Vec<u16>, usize, &str); 9] = [
            (w("en-US|fr-FR||"), 128, "en_US,fr_FR"),
            (w("en-US||"), 128, "en_US"),
            (w("en-US|fr-FR||"), 8, "en_US"),
            (w("en-US|fr-FR||"), 11, "en_US"),
            (w("en-US|fr-FR||"), 12, "en_US,fr_FR"),
            (w("||"), 128, ""),
            (w("ab"), 128, ""),
            (zh, 128, "zh_Hans_CN,de"),
            (w("pt-BR|pt|en||"), 128, "pt_BR,pt,en"),
        ];
        for (wbuf, buflen, expected) in cases {
            assert_eq!(ui_languages_to_csv(&wbuf, buflen), expected, "{wbuf:?}");
        }
        assert_eq!(ui_languages_to_csv(&[], 128), "");
    }

    /// `GetUserPreferredUILanguages()` and the WinXP fallback both report
    /// languages on any Windows (or Wine) installation.
    #[cfg(windows)]
    #[test]
    fn windows_locales() {
        let _l = crate::test_support::test_lock();
        let is_tag = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric());
        let locales = preferred_locales().unwrap();
        assert!(!locales.is_empty());
        for l in &locales {
            assert!(is_tag(&l.language), "{locales:?}");
            assert!(l.country.as_deref().is_none_or(is_tag), "{locales:?}");
        }
        let xp = sys::winxp().unwrap();
        let (lang, country) = xp.split_once('_').unwrap_or((&xp, ""));
        assert!(is_tag(lang), "{xp}");
        assert!(country.is_empty() || is_tag(country), "{xp}");
    }

    /// node's `navigator.languages` (its ICU locale), then lists of the
    /// test's own: what upstream's C reports for the same lists, built with
    /// emcc and run under node, but where it reads past its buffer (a list
    /// of exactly 128 bytes has no NUL) and for bytes that aren't UTF-8.
    #[cfg(target_os = "emscripten")]
    #[test]
    fn emscripten_navigator_languages() {
        use sdl3_emscripten::em_asm;
        let _l = crate::test_support::test_lock();
        hints::reset(hints::PREFERRED_LOCALES);
        let is_tag = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric());
        let locales = preferred_locales().unwrap();
        assert!(!locales.is_empty());
        for l in &locales {
            assert!(is_tag(&l.language), "{locales:?}");
            assert!(l.country.as_deref().is_none_or(is_tag), "{locales:?}");
        }

        // `property` of navigator, as the JavaScript expression `value`.
        let set = |property: &str, value: &str| {
            let js = format!("{property}\0{value}\0");
            // SAFETY: plain JavaScript; both strings are NUL-terminated.
            unsafe {
                em_asm!(
                    "Object.defineProperty(navigator, UTF8ToString($0), { value: eval(UTF8ToString($1)), configurable: true });",
                    js.as_ptr(),
                    js[property.len() + 1..].as_ptr()
                )
            };
        };
        let a120 = "a".repeat(120);
        let cases: [(&str, String); 10] = [
            ("['en-US', 'fr', 'de-CH']", "(en|US) (fr|-) (de|CH)".into()),
            ("['zh-Hant-TW', 'en']", "(zh|TW) (en|-)".into()),
            ("['a'.repeat(127)]", format!("({}|-)", "a".repeat(127))),
            // (no room for it and a NUL)
            ("['a'.repeat(128)]", String::new()),
            // (upstream's "x" is followed by what's past its buffer)
            (
                "['a'.repeat(120), 'bb-CC', 'x']",
                format!("({a120}|-) (bb|CC) (x|-)"),
            ),
            (
                "['a'.repeat(120), 'bbbbbbb']",
                format!("({a120}|-) (bbbbbbb|-)"),
            ),
            (
                "['a'.repeat(120), 'bbbbbb']",
                format!("({a120}|-) (bbbbbb|-)"),
            ),
            (
                "['a'.repeat(120), 'bbbbb', 'c']",
                format!("({a120}|-) (bbbbb|-) (c|-)"),
            ),
            // (upstream's language is the bytes E9 74 E9)
            (
                "['\\u00e9t\\u00e9-FR', 'en']",
                "(\u{FFFD}t\u{FFFD}|FR) (en|-)".into(),
            ),
            // (none: navigator.language, then)
            ("[]", "(pt|BR)".into()),
        ];
        set("language", "'pt-BR'");
        let got = || {
            preferred_locales()
                .unwrap()
                .iter()
                .map(|l| format!("({}|{})", l.language, l.country.as_deref().unwrap_or("-")))
                .collect::<Vec<_>>()
                .join(" ")
        };
        for (list, expected) in cases {
            set("languages", list);
            assert_eq!(got(), expected, "{list}");
        }
        set("languages", "undefined");
        assert_eq!(got(), "(pt|BR)");
        // Nothing at all: no locales, and no error.
        set("language", "undefined");
        assert_eq!(got(), "");

        // SAFETY: plain JavaScript (node's own properties again).
        unsafe { em_asm!("delete navigator.languages; delete navigator.language;") };
        assert_eq!(preferred_locales().unwrap(), locales);
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

    /// The macOS buffer handling, with `getCString:maxLength:encoding:`'s
    /// rules (ASCII only, with room for the NUL).
    #[test]
    fn macos_languages_to_csv() {
        let run = |langs: &[&str], buflen: usize| {
            languages_to_csv(
                langs.len(),
                |i, buflen| {
                    let s = langs[i];
                    (s.is_ascii() && s.len() < buflen).then(|| s.to_owned())
                },
                buflen,
            )
        };
        assert_eq!(
            run(&["en-US", "zh-Hant-CN", "fr"], 128),
            "en_US,zh-Hant_CN,fr"
        );
        assert_eq!(run(&[], 128), "");
        // A language that doesn't convert leaves an empty entry.
        assert_eq!(run(&["en-GB", "ελ", "de-DE"], 128), "en_GB,,de_DE");
        // Out of room: what doesn't convert in the room left is an empty
        // entry, and the list stops when there's no room for a comma.
        assert_eq!(run(&["en-US", "de-DE"], 9), "en_US,");
        assert_eq!(run(&["en-US", "de-DE"], 7), "en_US,");
        assert_eq!(run(&["en-US", "de-DE"], 6), "en_US");
        assert_eq!(run(&["en-US", "de-DE", "fr-FR"], 12), "en_US,de_DE");
        assert_eq!(
            build_locales_from_csv_string(&run(&["en-GB", "ελ", "de-DE"], 128)),
            vec![l("en", Some("GB")), l("de", Some("DE"))]
        );
    }

    /// `NSLocale.preferredLanguages`, as upstream reports it.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn macos_locales() {
        use crate::core::apple::foundation::{ns_array_items, string_from_ns};
        use crate::core::apple::objc::{autoreleasepool, class, msg_send, Id};
        let _l = crate::test_support::test_lock();
        hints::reset(hints::PREFERRED_LOCALES);
        let expected: Vec<Locale> = autoreleasepool(|| unsafe {
            let languages: Id = msg_send![class!(NSLocale), preferredLanguages];
            ns_array_items(languages)
                .into_iter()
                .filter_map(|s| string_from_ns(s))
                .map(|s| match s.rsplit_once('-') {
                    Some((lang, country)) => l(lang, Some(country)),
                    None => l(&s, None),
                })
                .collect()
        });
        let locales = preferred_locales().unwrap();
        // (all of them, unless they overflow the 128-byte buffer)
        assert!(!locales.is_empty());
        assert_eq!(locales, expected[..locales.len()]);
    }

    #[cfg(all(
        unix,
        not(any(
            target_vendor = "apple",
            target_os = "android",
            target_os = "haiku",
            target_os = "emscripten"
        ))
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
