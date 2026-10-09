// Rust translation of src/autofit/afshaper.c and afshaper.h from FreeType
// (2.13.2, as SDL_ttf's external/freetype pins it), with HarfBuzz
// (`FT_CONFIG_OPTION_USE_HARFBUZZ` is defined, as SDL_ttf builds its
// bundled FreeType along with its bundled HarfBuzz); with afblue.h's
// `GET_UTF8_CHAR` and afcover.h's coverages.
// Copyright (C) 2013-2023 by David Turner, Robert Wilhelm, and Werner Lemberg.
// This is an altered (translated) version of the original software; it is
// used under the FreeType License (see FTL.TXT and LICENSE.txt).

//! HarfBuzz interface for accessing OpenType features.
//!
//! Translation notes: HarfBuzz is the translated one
//! (`crate::harfbuzz`). The trace output is not translated.

use super::super::base::ftobjs::*;
use super::super::fttypes::*;
use super::afglobal::{AfFaceGlobalsRec, AF_STYLE_UNASSIGNED};
use super::aftypes::*;
use crate::harfbuzz::hb_buffer::HbBuffer;
use crate::harfbuzz::hb_common::*;
use crate::harfbuzz::hb_face::{HB_OT_TAG_GPOS, HB_OT_TAG_GSUB};
use crate::harfbuzz::hb_font::HbFont;
use crate::harfbuzz::hb_ot_layout::{
    hb_ot_layout_collect_lookups, hb_ot_layout_lookup_collect_glyphs,
    hb_ot_layout_lookup_would_substitute, HB_OT_TAG_DEFAULT_SCRIPT,
};
use crate::harfbuzz::hb_ot_tag::hb_ot_tags_from_script_and_language;
use crate::harfbuzz::hb_set::{HbSet, HB_SET_VALUE_INVALID};
use crate::harfbuzz::hb_shape::hb_shape;

/// `GET_UTF8_CHAR`: an auxiliary macro to decode a UTF-8 character --
/// since we only use hard-coded, self-converted data, no error checking
/// is performed
pub fn get_utf8_char(s: &[u8], p: &mut usize) -> FtULong {
    let mut ch = s[*p] as FtULong;
    *p += 1;
    if ch >= 0x80 {
        let mut len_: FtUInt;

        if ch < 0xE0 {
            len_ = 1;
            ch &= 0x1F;
        } else if ch < 0xF0 {
            len_ = 2;
            ch &= 0x0F;
        } else {
            len_ = 3;
            ch &= 0x07;
        }

        while len_ > 0 {
            ch = (ch << 6) | (s[*p] as FtULong & 0x3F);
            *p += 1;
            len_ -= 1;
        }
    }
    ch
}

/*
 * We use `sets' (in the HarfBuzz sense, which comes quite near to the
 * usual mathematical meaning) to manage both lookups and glyph indices.
 *
 * 1. For each coverage, collect lookup IDs in a set.  Note that an
 *    auto-hinter `coverage' is represented by one `feature', and a
 *    feature consists of an arbitrary number of (font specific) `lookup's
 *    that actually do the mapping job.  Please check the OpenType
 *    specification for more details on features and lookups.
 *
 * 2. Create glyph ID sets from the corresponding lookup sets.
 *
 * 3. The glyph set corresponding to AF_COVERAGE_DEFAULT is computed
 *    with all lookups specific to the OpenType script activated.  It
 *    relies on the order of AF_DEFINE_STYLE_CLASS entries so that
 *    special coverages (like `oldstyle figures') don't get overwritten.
 *
 */

/* load coverage tags */
/* define mapping between coverage tags and AF_Coverage */

/// `coverages`: the OpenType feature of each coverage (afcover.h; `None`
/// for `AF_COVERAGE_DEFAULT`)
static COVERAGES: [Option<HbTag>; 11] = [
    Some(hb_tag(b'c', b'2', b'c', b'p')), /* petite capitals from capitals */
    Some(hb_tag(b'c', b'2', b's', b'c')), /* small capitals from capitals */
    Some(hb_tag(b'o', b'r', b'd', b'n')), /* ordinals */
    Some(hb_tag(b'p', b'c', b'a', b'p')), /* petite capitals */
    Some(hb_tag(b'r', b'u', b'b', b'y')), /* ruby */
    Some(hb_tag(b's', b'i', b'n', b'f')), /* scientific inferiors */
    Some(hb_tag(b's', b'm', b'c', b'p')), /* small capitals */
    Some(hb_tag(b's', b'u', b'b', b's')), /* subscript */
    Some(hb_tag(b's', b'u', b'p', b's')), /* superscript */
    Some(hb_tag(b't', b'i', b't', b'l')), /* titling */
    None,                                 /* AF_COVERAGE_DEFAULT */
];

/* load HarfBuzz script tags */

/// `scripts`: the HarfBuzz script of each auto-fitter script (afscript.h)
static SCRIPTS: [HbScript; AF_SCRIPT_MAX_ as usize] = [
    HB_SCRIPT_ADLAM,
    HB_SCRIPT_ARABIC,
    HB_SCRIPT_ARMENIAN,
    HB_SCRIPT_AVESTAN,
    HB_SCRIPT_BAMUM,
    HB_SCRIPT_BENGALI,
    HB_SCRIPT_BUHID,
    HB_SCRIPT_CHAKMA,
    HB_SCRIPT_CANADIAN_SYLLABICS,
    HB_SCRIPT_CARIAN,
    HB_SCRIPT_CHEROKEE,
    HB_SCRIPT_COPTIC,
    HB_SCRIPT_CYPRIOT,
    HB_SCRIPT_CYRILLIC,
    HB_SCRIPT_DEVANAGARI,
    HB_SCRIPT_DESERET,
    HB_SCRIPT_ETHIOPIC,
    HB_SCRIPT_GEORGIAN,
    HB_SCRIPT_INVALID, /* geok */
    HB_SCRIPT_GLAGOLITIC,
    HB_SCRIPT_GOTHIC,
    HB_SCRIPT_GREEK,
    HB_SCRIPT_GUJARATI,
    HB_SCRIPT_GURMUKHI,
    HB_SCRIPT_HEBREW,
    HB_SCRIPT_KAYAH_LI,
    HB_SCRIPT_KHMER,
    HB_SCRIPT_INVALID, /* khms */
    HB_SCRIPT_KANNADA,
    HB_SCRIPT_LAO,
    HB_SCRIPT_LATIN,
    HB_SCRIPT_INVALID, /* latb */
    HB_SCRIPT_INVALID, /* latp */
    HB_SCRIPT_LISU,
    HB_SCRIPT_MALAYALAM,
    HB_SCRIPT_MEDEFAIDRIN,
    HB_SCRIPT_MONGOLIAN,
    HB_SCRIPT_MYANMAR,
    HB_SCRIPT_NKO,
    HB_SCRIPT_INVALID, /* none */
    HB_SCRIPT_OL_CHIKI,
    HB_SCRIPT_OLD_TURKIC,
    HB_SCRIPT_OSAGE,
    HB_SCRIPT_OSMANYA,
    HB_SCRIPT_HANIFI_ROHINGYA,
    HB_SCRIPT_SAURASHTRA,
    HB_SCRIPT_SHAVIAN,
    HB_SCRIPT_SINHALA,
    HB_SCRIPT_SUNDANESE,
    HB_SCRIPT_TAMIL,
    HB_SCRIPT_TAI_VIET,
    HB_SCRIPT_TELUGU,
    HB_SCRIPT_TIFINAGH,
    HB_SCRIPT_THAI,
    HB_SCRIPT_VAI,
    HB_SCRIPT_LIMBU,
    HB_SCRIPT_ORIYA,
    HB_SCRIPT_SYLOTI_NAGRI,
    HB_SCRIPT_TIBETAN,
    HB_SCRIPT_HAN,
];

/// The number of auto-fitter scripts (`AF_SCRIPT_MAX`).
const AF_SCRIPT_MAX_: AfScript = super::afscript::AF_SCRIPT_MAX;

/// A zero-terminated tag list, as a slice of its tags.
fn tags(list: &[HbTag]) -> &[HbTag] {
    let n = list
        .iter()
        .position(|&t| t == HB_TAG_NONE)
        .unwrap_or(list.len());
    &list[..n]
}

/// `af_shaper_get_coverage`
pub fn af_shaper_get_coverage(
    globals: &mut AfFaceGlobalsRec,
    face: &mut FtFace,
    style_class: &AfStyleClassRec,
    gstyles: &mut [FtUShort],
    default_script: bool,
) -> FtResult<()> {
    let hb_face = globals.hb_font.get_face().clone();

    let coverage_tags = COVERAGES[style_class.coverage as usize];
    let script = SCRIPTS[style_class.script as usize];

    let mut script_tags: [HbTag; 4] = [HB_TAG_NONE, HB_TAG_NONE, HB_TAG_NONE, HB_TAG_NONE];

    /* Convert a HarfBuzz script tag into the corresponding OpenType */
    /* tag or tags -- some Indic scripts like Devanagari have an old */
    /* and a new set of features.                                    */
    {
        let mut tags_: [HbTag; 3] = [0; 3];

        let (tags_count, _) = hb_ot_tags_from_script_and_language(
            script,
            HB_LANGUAGE_INVALID,
            Some(&mut tags_),
            None,
        );
        script_tags[0] = if tags_count > 0 {
            tags_[0]
        } else {
            HB_TAG_NONE
        };
        script_tags[1] = if tags_count > 1 {
            tags_[1]
        } else {
            HB_TAG_NONE
        };
        script_tags[2] = if tags_count > 2 {
            tags_[2]
        } else {
            HB_TAG_NONE
        };
    }

    /* If the second tag is HB_OT_TAG_DEFAULT_SCRIPT, change that to     */
    /* HB_TAG_NONE except for the default script.                        */
    if default_script {
        if script_tags[0] == HB_TAG_NONE {
            script_tags[0] = HB_OT_TAG_DEFAULT_SCRIPT;
        } else if script_tags[1] == HB_TAG_NONE {
            script_tags[1] = HB_OT_TAG_DEFAULT_SCRIPT;
        } else if script_tags[1] != HB_OT_TAG_DEFAULT_SCRIPT {
            script_tags[2] = HB_OT_TAG_DEFAULT_SCRIPT;
        }
    } else {
        /* we use non-standard tags like `khms' for special purposes;       */
        /* HarfBuzz maps them to `DFLT', which we don't want to handle here */
        if script_tags[0] == HB_OT_TAG_DEFAULT_SCRIPT {
            return Ok(());
        }
    }

    let features: Option<[HbTag; 1]> = coverage_tags.map(|t| [t]);

    let mut gsub_lookups = HbSet::new(); /* GSUB lookups for a given script */
    hb_ot_layout_collect_lookups(
        &hb_face,
        HB_OT_TAG_GSUB,
        Some(tags(&script_tags)),
        None,
        features.as_ref().map(|f| &f[..]),
        &mut gsub_lookups,
    );

    if gsub_lookups.is_empty() {
        return Ok(()); /* nothing to do */
    }

    let mut gsub_glyphs = HbSet::new(); /* glyphs covered by GSUB lookups  */
    let mut idx = HB_SET_VALUE_INVALID;
    while gsub_lookups.next(&mut idx) {
        /* get output coverage of GSUB feature */
        hb_ot_layout_lookup_collect_glyphs(
            &hb_face,
            HB_OT_TAG_GSUB,
            idx,
            None,
            None,
            None,
            Some(&mut gsub_glyphs),
        );
    }

    let mut gpos_lookups = HbSet::new(); /* GPOS lookups for a given script */
    hb_ot_layout_collect_lookups(
        &hb_face,
        HB_OT_TAG_GPOS,
        Some(tags(&script_tags)),
        None,
        features.as_ref().map(|f| &f[..]),
        &mut gpos_lookups,
    );

    let mut gpos_glyphs = HbSet::new(); /* glyphs covered by GPOS lookups  */
    let mut idx = HB_SET_VALUE_INVALID;
    while gpos_lookups.next(&mut idx) {
        /* get input coverage of GPOS feature */
        hb_ot_layout_lookup_collect_glyphs(
            &hb_face,
            HB_OT_TAG_GPOS,
            idx,
            None,
            Some(&mut gpos_glyphs),
            None,
            None,
        );
    }

    /*
     * We now check whether we can construct blue zones, using glyphs
     * covered by the feature only.  In case there is not a single zone
     * (that is, not a single character is covered), we skip this coverage.
     *
     */
    if style_class.coverage != AF_COVERAGE_DEFAULT {
        let bss = style_class.blue_stringset;
        let mut bs = bss as usize;

        let mut found = false;

        while super::afblue::AF_BLUE_STRINGSETS[bs].string != super::afblue::AF_BLUE_STRING_MAX {
            let p_str = &super::afblue::AF_BLUE_STRINGS[..];
            let mut p = super::afblue::AF_BLUE_STRINGSETS[bs].string as usize;

            while p_str[p] != 0 {
                let ch = get_utf8_char(p_str, &mut p);

                let mut idx = HB_SET_VALUE_INVALID;
                while gsub_lookups.next(&mut idx) {
                    let gidx = ft_get_char_index(face, ch) as HbCodepoint;

                    if hb_ot_layout_lookup_would_substitute(&hb_face, idx, &[gidx], true) {
                        found = true;
                        break;
                    }
                }
            }
            bs += 1;
        }

        if !found {
            /* no blue characters found; style skipped */
            return Ok(());
        }
    }

    /*
     * Various OpenType features might use the same glyphs at different
     * vertical positions; for example, superscript and subscript glyphs
     * could be the same.  However, the auto-hinter is completely
     * agnostic of OpenType features after the feature analysis has been
     * completed: The engine then simply receives a glyph index and returns a
     * hinted and usually rendered glyph.
     *
     * Consider the superscript feature of font `pala.ttf': Some of the
     * glyphs are `real', that is, they have a zero vertical offset, but
     * most of them are small caps glyphs shifted up to the superscript
     * position (that is, the `sups' feature is present in both the GSUB and
     * GPOS tables).  The code for blue zones computation actually uses a
     * feature's y offset so that the `real' glyphs get correct hints.  But
     * later on it is impossible to decide whether a glyph index belongs to,
     * say, the small caps or superscript feature.
     *
     * For this reason, we don't assign a style to a glyph if the current
     * feature covers the glyph in both the GSUB and the GPOS tables.  This
     * is quite a broad condition, assuming that
     *
     *   (a) glyphs that get used in multiple features are present in a
     *       feature without vertical shift,
     *
     * and
     *
     *   (b) a feature's GPOS data really moves the glyph vertically.
     *
     * Not fulfilling condition (a) makes a font larger; it would also
     * reduce the number of glyphs that could be addressed directly without
     * using OpenType features, so this assumption is rather strong.
     *
     * Condition (b) is much weaker, and there might be glyphs which get
     * missed.  However, the OpenType features we are going to handle are
     * primarily located in GSUB, and HarfBuzz doesn't provide an API to
     * directly get the necessary information from the GPOS table.  A
     * possible solution might be to directly parse the GPOS table to find
     * out whether a glyph gets shifted vertically, but this is something I
     * would like to avoid if not really necessary.
     *
     * Note that we don't follow this logic for the default coverage.
     * Complex scripts like Devanagari have mandatory GPOS features to
     * position many glyph elements, using mark-to-base or mark-to-ligature
     * tables; the number of glyphs missed due to condition (b) would be far
     * too large.
     *
     */
    if style_class.coverage != AF_COVERAGE_DEFAULT {
        gsub_glyphs.subtract(&gpos_glyphs);
    }

    let mut idx = HB_SET_VALUE_INVALID;
    while gsub_glyphs.next(&mut idx) {
        /* glyph indices returned by `hb_ot_layout_lookup_collect_glyphs' */
        /* can be arbitrary: some fonts use fake indices for processing   */
        /* internal to GSUB or GPOS, which is fully valid                 */
        if idx >= globals.glyph_count as HbCodepoint {
            continue;
        }

        if gstyles[idx as usize] == AF_STYLE_UNASSIGNED {
            gstyles[idx as usize] = style_class.style as FtUShort;
        }
    }

    /* Exit: */
    Ok(())
}

/* construct HarfBuzz features */
/* define mapping between HarfBuzz features and AF_Coverage */

/// `features`: the feature of a coverage (`{ tag, 1, 0, (unsigned int)-1 }`)
fn feature_of(coverage: AfCoverage) -> Option<HbFeature> {
    COVERAGES[coverage as usize].map(|tag| HbFeature {
        tag,
        value: 1,
        start: 0,
        end: u32::MAX,
    })
}

/// `af_shaper_buf_create`
pub fn af_shaper_buf_create(_face: &FtFace) -> HbBuffer {
    HbBuffer::new()
}

/// `af_shaper_buf_destroy`
pub fn af_shaper_buf_destroy(_face: &FtFace, _buf: HbBuffer) {}

/// `af_shaper_get_cluster`: returns the new position in `s` and the
/// number of glyphs of the cluster (`metrics` is the style's class and
/// the face's units per EM)
pub fn af_shaper_get_cluster(
    s: &[u8],
    mut p: usize,
    style_class: &AfStyleClassRec,
    units_per_em: FtUShort,
    globals: &mut AfFaceGlobalsRec,
    buf: &mut HbBuffer,
) -> (usize, FtUInt) {
    let upem = units_per_em as i32;
    let feature = feature_of(style_class.coverage);
    let features: Vec<HbFeature> = feature.into_iter().collect();

    /* we shape at a size of units per EM; this means font units */
    globals.hb_font.set_scale(upem, upem);

    while s[p] == b' ' {
        p += 1;
    }

    /* count bytes up to next space (or end of buffer) */
    let mut q = p;
    while !(s[q] == b' ' || s[q] == 0) {
        let _dummy = get_utf8_char(s, &mut q);
    }
    let len = (q - p) as i32;

    /* feed character(s) to the HarfBuzz buffer */
    buf.clear_contents();
    buf.add_utf8(&s[p..q], 0, len);

    /* we let HarfBuzz guess the script and writing direction */
    buf.guess_segment_properties();

    /* shape buffer, which means conversion from character codes to */
    /* glyph indices, possibly applying a feature                   */
    {
        let mut font = HbFont::new(&mut globals.hb_font, None);
        hb_shape(&mut font, buf, &features);
    }

    if feature.is_some() {
        let hb_buf = &mut globals.hb_buf;

        /* we have to check whether applying a feature does actually change */
        /* glyph indices; otherwise the affected glyph or glyphs aren't     */
        /* available at all in the feature                                  */

        hb_buf.clear_contents();
        hb_buf.add_utf8(&s[p..q], 0, len);
        hb_buf.guess_segment_properties();
        {
            let mut font = HbFont::new(&mut globals.hb_font, None);
            hb_shape(&mut font, &mut globals.hb_buf, &[]);
        }

        let ginfo = buf.get_glyph_infos();
        let hb_ginfo = globals.hb_buf.get_glyph_infos();
        let gcount = ginfo.len();
        let hb_gcount = hb_ginfo.len();

        if gcount == hb_gcount {
            let mut i = 0;
            while i < gcount {
                if ginfo[i].codepoint != hb_ginfo[i].codepoint {
                    break;
                }
                i += 1;
            }

            if i == gcount {
                /* both buffers have identical glyph indices */
                buf.clear_contents();
            }
        }
    }

    let count = buf.get_length();

    (q, count)
}

/// `af_shaper_get_elem`
pub fn af_shaper_get_elem(
    buf: &mut HbBuffer,
    idx: FtUInt,
    advance: Option<&mut FtLong>,
    y_offset: Option<&mut FtLong>,
) -> FtULong {
    let gcount = buf.get_glyph_infos().len() as FtUInt;

    if idx >= gcount {
        return 0;
    }

    let gpos = buf.get_glyph_positions()[idx as usize];
    if let Some(advance) = advance {
        *advance = gpos.x_advance as FtLong;
    }
    if let Some(y_offset) = y_offset {
        *y_offset = gpos.y_offset as FtLong;
    }

    buf.get_glyph_infos()[idx as usize].codepoint as FtULong
}
