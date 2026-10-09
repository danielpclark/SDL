// Tests of the HarfBuzz translation against upstream HarfBuzz.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).

//! `scratch_shape` shapes the cases of the file `HB_CASES` names with the
//! font `HB_FONT` names (at `HB_SIZE` points, 72 dpi), as the C tool that
//! made the reference data does, and prints the results; it only runs
//! when asked (`--ignored`).

use std::sync::Arc;

use super::hb_buffer::HbBuffer;
use super::hb_common::*;
use super::hb_font::HbFont;
use super::hb_ft::*;
use super::hb_shape::*;
use crate::freetype::base::ftinit::ft_init_freetype;
use crate::freetype::base::ftobjs::*;
use crate::freetype::fttypes::*;

/// Shapes the tab-separated cases (`dir script lang kern text`, `-` for
/// unset) of `cases` with `font` at `size` points, returning a line per
/// case: the direction, the language and the serialized glyphs.
pub(crate) fn shape_cases(font_data: &[u8], size: f64, cases: &str) -> String {
    let library = ft_init_freetype().expect("FreeType");
    let mut face = ft_new_memory_face(&library, Arc::from(font_data), 0).expect("face");
    ft_set_char_size(&mut face, 0, (size * 64.0) as FtF26Dot6, 72, 72).expect("size");
    let mut data = hb_ft_font_create(&mut face);
    hb_ft_font_set_load_flags(&mut data, FT_LOAD_DEFAULT | FT_LOAD_TARGET_NORMAL);
    let mut out = String::new();
    for line in cases.lines() {
        let f: Vec<&str> = line.splitn(5, '\t').collect();
        if f.len() < 5 {
            continue;
        }
        let mut buf = HbBuffer::new();
        if f[0] != "-" {
            buf.set_direction(hb_direction_from_string(f[0].as_bytes()));
        }
        if f[1] != "-" {
            buf.set_script(hb_script_from_string(f[1].as_bytes()));
        }
        buf.set_language(hb_language_from_string(if f[2] != "-" {
            f[2].as_bytes()
        } else {
            b""
        }));
        buf.add_utf8(f[4].as_bytes(), 0, -1);
        buf.guess_segment_properties();
        let feat = HbFeature {
            tag: hb_tag(b'k', b'e', b'r', b'n'),
            value: f[3].parse().unwrap_or(0),
            start: HB_FEATURE_GLOBAL_START,
            end: HB_FEATURE_GLOBAL_END,
        };
        let mut font = HbFont::new(&mut data, Some(&mut face));
        hb_shape(&mut font, &mut buf, &[feat]);
        let s = hb_buffer_serialize_glyphs_text(&mut buf, false, false, true);
        out.push_str(&format!(
            "{} {} {}\n",
            hb_direction_to_string(buf.get_direction()),
            hb_language_to_string(buf.get_language()).unwrap_or("(null)"),
            s
        ));
    }
    out
}

#[test]
#[ignore]
fn scratch_shape() {
    let font = std::fs::read(std::env::var("HB_FONT").expect("HB_FONT")).expect("font");
    let cases =
        std::fs::read_to_string(std::env::var("HB_CASES").expect("HB_CASES")).expect("cases");
    let size: f64 = std::env::var("HB_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24.0);
    print!("{}", shape_cases(&font, size, &cases));
}
