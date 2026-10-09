// Rust translation of src/hb-shape.cc, src/hb-shape-plan.cc and the text
// serialization of src/hb-buffer-serialize.cc from HarfBuzz (8.5.0, as
// SDL_ttf's external/harfbuzz pins it).
// Copyright © 2009  Red Hat, Inc.
// Copyright © 2012  Google, Inc.
// This is an altered (translated) version of the original software; see
// LICENSE.txt (HarfBuzz's "Old MIT" license).
//
// Red Hat Author(s): Behdad Esfahbod
// Google Author(s): Behdad Esfahbod

//! Shaping: `hb_shape` and the shape plans.
//!
//! Translation notes: the only shaper is "ot" (SDL_ttf's build lists no
//! other one before it). Shape plans are cached on the face as C caches
//! them (`hb_shape_plan_create_cached2`), which only saves work.

use std::sync::{Arc, Mutex};

use super::hb_buffer::*;
use super::hb_common::*;
use super::hb_face::HbFace;
use super::hb_font::HbFont;
use super::hb_ot_shape::*;

/// `hb_shape_plan_t`
#[derive(Debug)]
pub(crate) struct HbShapePlan {
    /* key */
    pub(crate) props: HbSegmentProperties,
    pub(crate) user_features: Vec<HbFeature>,
    pub(crate) ot_key: HbOtShapePlanKey,

    pub(crate) ot: HbOtShapePlan,
}

/// The shape plans cached on a face (C's `face->shape_plans`).
#[derive(Debug, Default)]
pub(crate) struct HbShapePlanCache {
    plans: Mutex<Vec<Arc<HbShapePlan>>>,
}

/// `hb_shape_plan_key_t::user_features_match`
fn user_features_match(a: &[HbFeature], b: &[HbFeature]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (x, y) in a.iter().zip(b) {
        if x.tag != y.tag
            || x.value != y.value
            || (x.start == HB_FEATURE_GLOBAL_START && x.end == HB_FEATURE_GLOBAL_END)
                != (y.start == HB_FEATURE_GLOBAL_START && y.end == HB_FEATURE_GLOBAL_END)
        {
            return false;
        }
    }
    true
}

/// `hb_shape_plan_create2`: `None` is the empty plan.
fn hb_shape_plan_create2(
    face: &HbFace,
    props: &HbSegmentProperties,
    user_features: &[HbFeature],
    coords: &[i32],
) -> Option<HbShapePlan> {
    if props.direction == HB_DIRECTION_INVALID {
        return None;
    }

    /* hb_shape_plan_key_t::init */
    let mut features = user_features.to_vec();
    /* Make start/end uniform to easier catch bugs. */
    for _ in 0..features.len() {
        if features[0].start != HB_FEATURE_GLOBAL_START {
            features[0].start = 1;
        }
        if features[0].end != HB_FEATURE_GLOBAL_END {
            features[0].end = 2;
        }
    }
    let ot_key = HbOtShapePlanKey::init(face, coords);

    let ot = HbOtShapePlan::init0(face, props, &features, &ot_key)?;

    Some(HbShapePlan {
        props: *props,
        user_features: features,
        ot_key,
        ot,
    })
}

/// `hb_shape_plan_create_cached2`
fn hb_shape_plan_create_cached2(
    face: &HbFace,
    props: &HbSegmentProperties,
    user_features: &[HbFeature],
    coords: &[i32],
) -> Option<Arc<HbShapePlan>> {
    if props.direction == HB_DIRECTION_INVALID {
        return None;
    }

    let ot_key = HbOtShapePlanKey::init(face, coords);

    let cache = face.shape_plans();
    {
        let plans = cache.plans.lock().unwrap_or_else(|e| e.into_inner());
        for plan in plans.iter() {
            if hb_segment_properties_equal(&plan.props, props)
                && user_features_match(&plan.user_features, user_features)
                && plan.ot_key == ot_key
            {
                return Some(plan.clone());
            }
        }
    }

    let plan = Arc::new(hb_shape_plan_create2(face, props, user_features, coords)?);

    /* Don't add to the cache if face is inert. */
    /* Don't add the plan to the cache if there were user features with non-global ranges */
    let all_global = user_features
        .iter()
        .all(|f| f.start == HB_FEATURE_GLOBAL_START && f.end == HB_FEATURE_GLOBAL_END);
    if all_global {
        let mut plans = cache.plans.lock().unwrap_or_else(|e| e.into_inner());
        plans.push(plan.clone());
    }

    Some(plan)
}

/// `hb_shape_plan_execute`
fn hb_shape_plan_execute(
    plan: Option<&HbShapePlan>,
    font: &mut HbFont,
    buffer: &mut HbBuffer,
    features: &[HbFeature],
) -> bool {
    if buffer.len == 0 {
        return true;
    }

    let Some(plan) = plan else {
        return false;
    };

    let ret = _hb_ot_shape(&plan.ot, font, buffer, features);

    if ret {
        buffer.content_type = HB_BUFFER_CONTENT_TYPE_GLYPHS;
    }

    ret
}

/// `hb_shape_full`
pub fn hb_shape_full(font: &mut HbFont, buffer: &mut HbBuffer, features: &[HbFeature]) -> bool {
    if buffer.len == 0 {
        return true;
    }

    buffer.enter();

    let face = font.p.face.clone();
    let coords = font.p.coords.clone();
    let plan = hb_shape_plan_create_cached2(&face, &buffer.props, features, &coords);
    let res = hb_shape_plan_execute(plan.as_deref(), font, buffer, features);

    if buffer.max_ops <= 0 {
        buffer.shaping_failed = true;
    }

    res
}

/// `hb_shape`: shapes `buffer` using `font`, turning its Unicode
/// characters content into positioned glyphs.
pub fn hb_shape(font: &mut HbFont, buffer: &mut HbBuffer, features: &[HbFeature]) {
    hb_shape_full(font, buffer, features);
}

/// `hb_buffer_serialize_glyphs` in the text format with
/// `HB_BUFFER_SERIALIZE_FLAG_NO_GLYPH_NAMES` (and the given flags of
/// `HB_BUFFER_SERIALIZE_FLAG_NO_CLUSTERS`, `NO_POSITIONS`,
/// `GLYPH_FLAGS`): `[gid=cluster@x_offset,y_offset+x_advance|...]`.
pub fn hb_buffer_serialize_glyphs_text(
    buffer: &mut HbBuffer,
    no_clusters: bool,
    no_positions: bool,
    glyph_flags: bool,
) -> String {
    let len = buffer.len as usize;
    let infos = buffer.info[..len].to_vec();
    let pos = if no_positions {
        Vec::new()
    } else {
        buffer.get_glyph_positions()[..len].to_vec()
    };
    let mut out = String::new();
    let (x, y) = (0, 0);
    for (i, info) in infos.iter().enumerate() {
        out.push(if i != 0 { '|' } else { '[' });
        out.push_str(&info.codepoint.to_string());
        if !no_clusters {
            out.push_str(&format!("={}", info.cluster));
        }
        if !no_positions {
            let p = pos[i];
            if x + p.x_offset != 0 || y + p.y_offset != 0 {
                out.push_str(&format!("@{},{}", x + p.x_offset, y + p.y_offset));
            }
            /* (HB_BUFFER_SERIALIZE_FLAG_NO_ADVANCES is not set) */
            out.push_str(&format!("+{}", p.x_advance));
            if p.y_advance != 0 {
                out.push_str(&format!(",{}", p.y_advance));
            }
        }
        if glyph_flags && (info.mask & HB_GLYPH_FLAG_DEFINED) != 0 {
            out.push_str(&format!("#{:X}", info.mask & HB_GLYPH_FLAG_DEFINED));
        }
        if i == len - 1 {
            out.push(']');
        }
    }
    out
}

impl HbFace {
    /// The face's shape plan cache.
    pub(crate) fn shape_plans(&self) -> &HbShapePlanCache {
        &self.shape_plans
    }
}
