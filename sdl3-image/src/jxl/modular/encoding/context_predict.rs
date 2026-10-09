// Rust translation of lib/jxl/modular/encoding/context_predict.h from libjxl
// (https://github.com/libjxl/libjxl, at the revision SDL_image's
// external/libjxl pins: libjxl 0.7.3 with SDL's patches).
// Copyright (c) the JPEG XL Project Authors. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The modular predictors, the weighted predictor and the context
//! properties. The channels' pixels are reached through the plane's storage
//! and the index of the current pixel (upstream's pointer `pp`). (The
//! encoder's predictor presets are not translated.)

use super::super::super::base::floor_log2_nonzero_u64;
use super::super::modular_image::{Channel, Image, PixelType, PixelTypeW};
use super::super::options::{
    Predictor, Properties, PropertyVal, K_NUM_MODULAR_PREDICTORS, K_NUM_STATIC_PROPERTIES,
};

pub(crate) mod weighted {
    use super::super::super::super::base::Status;
    use super::super::super::super::fields::{bundle_init, Fields, Visitor};
    use super::super::super::modular_image::{PixelType, PixelTypeW};
    use super::super::super::options::{Properties, PropertyVal};
    use super::floor_log2_nonzero_u64;

    pub(crate) const K_NUM_PREDICTORS: usize = 4;
    pub(crate) const K_PRED_EXTRA_BITS: i64 = 3;
    pub(crate) const K_PREDICTION_ROUND: i64 = ((1 << K_PRED_EXTRA_BITS) >> 1) - 1;
    pub(crate) const K_NUM_PROPERTIES: usize = 1;

    /// Translation of `weighted::Header`.
    #[derive(Clone, Debug, Default)]
    pub(crate) struct Header {
        pub all_default: bool,
        pub p1c: PixelType,
        pub p2c: PixelType,
        pub p3ca: PixelType,
        pub p3cb: PixelType,
        pub p3cc: PixelType,
        pub p3cd: PixelType,
        pub p3ce: PixelType,
        pub w: [u32; K_NUM_PREDICTORS],
    }

    impl Header {
        // TODO(janwas): move to cc file, avoid including fields.h.
        pub(crate) fn new() -> Self {
            let mut h = Header::default();
            bundle_init(&mut h);
            h
        }
    }

    impl Fields for Header {
        fn visit_fields(&mut self, visitor: &mut dyn Visitor) -> Status {
            if visitor.all_default(&mut self.all_default) {
                // Overwrite all serialized fields, but not any nonserialized_*.
                visitor.set_default(self);
                return Ok(());
            }
            let visit_p =
                |visitor: &mut dyn Visitor, val: PixelType, p: &mut PixelType| -> Status {
                    let mut up = *p as u32;
                    visitor.bits(5, val as u32, &mut up)?;
                    *p = up as PixelType;
                    Ok(())
                };
            visit_p(visitor, 16, &mut self.p1c)?;
            visit_p(visitor, 10, &mut self.p2c)?;
            visit_p(visitor, 7, &mut self.p3ca)?;
            visit_p(visitor, 7, &mut self.p3cb)?;
            visit_p(visitor, 7, &mut self.p3cc)?;
            visit_p(visitor, 0, &mut self.p3cd)?;
            visit_p(visitor, 0, &mut self.p3ce)?;
            visitor.bits(4, 0xd, &mut self.w[0])?;
            visitor.bits(4, 0xc, &mut self.w[1])?;
            visitor.bits(4, 0xc, &mut self.w[2])?;
            visitor.bits(4, 0xc, &mut self.w[3])?;
            Ok(())
        }
    }

    /// Translation of `weighted::State`.
    pub(crate) struct State {
        pub prediction: [PixelTypeW; K_NUM_PREDICTORS],
        pub pred: PixelTypeW, // *before* removing the added bits.
        pub pred_errors: [Vec<u32>; K_NUM_PREDICTORS],
        pub error: Vec<i32>,
        pub header: Header,

        // Allows to approximate division by a number from 1 to 64.
        pub divlookup: [u32; 64],
    }

    #[inline]
    pub(crate) fn add_bits(x: PixelTypeW) -> PixelTypeW {
        ((x as u64) << K_PRED_EXTRA_BITS) as PixelTypeW
    }

    impl State {
        pub(crate) fn new(header: &Header, xsize: usize, _ysize: usize) -> State {
            // Extra margin to avoid out-of-bounds writes.
            // All have space for two rows of data.
            let n = (xsize + 2) * 2;
            let mut divlookup = [0u32; 64];
            // Initialize division lookup table.
            for (i, d) in divlookup.iter_mut().enumerate() {
                *d = (1u32 << 24) / (i as u32 + 1);
            }
            State {
                prediction: [0; K_NUM_PREDICTORS],
                pred: 0,
                pred_errors: [vec![0; n], vec![0; n], vec![0; n], vec![0; n]],
                error: vec![0; n],
                header: header.clone(),
                divlookup,
            }
        }

        // Approximates 4+(maxweight<<24)/(x+1), avoiding division
        #[inline]
        pub(crate) fn error_weight(&self, x: u64, maxweight: u32) -> u32 {
            let mut shift = floor_log2_nonzero_u64(x + 1) as i32 - 5;
            if shift < 0 {
                shift = 0;
            }
            4u32.wrapping_add(
                maxweight.wrapping_mul(self.divlookup[(x >> shift) as usize]) >> shift,
            )
        }

        // Approximates the weighted average of the input values with the given
        // weights, avoiding division. Weights must sum to at least 16.
        #[inline]
        pub(crate) fn weighted_average(
            &self,
            p: &[PixelTypeW; K_NUM_PREDICTORS],
            mut w: [u32; K_NUM_PREDICTORS],
        ) -> PixelTypeW {
            let mut weight_sum: u32 = 0;
            for &wi in w.iter() {
                weight_sum = weight_sum.wrapping_add(wi);
            }
            let log_weight = floor_log2_nonzero_u64(weight_sum as u64) as u32; // at least 4.
            weight_sum = 0;
            for wi in w.iter_mut() {
                *wi >>= log_weight.wrapping_sub(4);
                weight_sum = weight_sum.wrapping_add(*wi);
            }
            // for rounding.
            let mut sum: PixelTypeW = (weight_sum >> 1).wrapping_sub(1) as i64;
            for i in 0..K_NUM_PREDICTORS {
                sum = sum.wrapping_add(p[i].wrapping_mul(w[i] as i64));
            }
            sum.wrapping_mul(self.divlookup[(weight_sum.wrapping_sub(1)) as usize] as i64) >> 24
        }

        /// Translation of `Predict<compute_properties>()`.
        #[allow(clippy::too_many_arguments)]
        #[inline]
        pub(crate) fn predict<const COMPUTE_PROPERTIES: bool>(
            &mut self,
            x: usize,
            y: usize,
            xsize: usize,
            n: PixelTypeW,
            w: PixelTypeW,
            ne: PixelTypeW,
            nw: PixelTypeW,
            nn: PixelTypeW,
            properties: &mut Properties,
            offset: usize,
        ) -> PixelTypeW {
            let cur_row = if y & 1 != 0 { 0 } else { xsize + 2 };
            let prev_row = if y & 1 != 0 { xsize + 2 } else { 0 };
            let pos_n = prev_row + x;
            let pos_ne = if x < xsize - 1 { pos_n + 1 } else { pos_n };
            let pos_nw = if x > 0 { pos_n - 1 } else { pos_n };
            let mut weights = [0u32; K_NUM_PREDICTORS];
            for i in 0..K_NUM_PREDICTORS {
                // pred_errors[pos_N] also contains the error of pixel W.
                // pred_errors[pos_NW] also contains the error of pixel WW.
                weights[i] = self.pred_errors[i][pos_n]
                    .wrapping_add(self.pred_errors[i][pos_ne])
                    .wrapping_add(self.pred_errors[i][pos_nw]);
                weights[i] = self.error_weight(weights[i] as u64, self.header.w[i]);
            }

            let n = add_bits(n);
            let w = add_bits(w);
            let ne = add_bits(ne);
            let nw = add_bits(nw);
            let nn = add_bits(nn);

            let te_w: PixelTypeW = if x == 0 {
                0
            } else {
                self.error[cur_row + x - 1] as i64
            };
            let te_n: PixelTypeW = self.error[pos_n] as i64;
            let te_nw: PixelTypeW = self.error[pos_nw] as i64;
            let sum_wn: PixelTypeW = te_n.wrapping_add(te_w);
            let te_ne: PixelTypeW = self.error[pos_ne] as i64;

            if COMPUTE_PROPERTIES {
                let mut p = te_w;
                if te_n.wrapping_abs() > p.wrapping_abs() {
                    p = te_n;
                }
                if te_nw.wrapping_abs() > p.wrapping_abs() {
                    p = te_nw;
                }
                if te_ne.wrapping_abs() > p.wrapping_abs() {
                    p = te_ne;
                }
                properties[offset] = p as PropertyVal;
            }

            let h = &self.header;
            self.prediction[0] = w.wrapping_add(ne).wrapping_sub(n);
            self.prediction[1] =
                n.wrapping_sub(sum_wn.wrapping_add(te_ne).wrapping_mul(h.p1c as i64) >> 5);
            self.prediction[2] =
                w.wrapping_sub(sum_wn.wrapping_add(te_nw).wrapping_mul(h.p2c as i64) >> 5);
            self.prediction[3] = n.wrapping_sub(
                te_nw
                    .wrapping_mul(h.p3ca as i64)
                    .wrapping_add(te_n.wrapping_mul(h.p3cb as i64))
                    .wrapping_add(te_ne.wrapping_mul(h.p3cc as i64))
                    .wrapping_add(nn.wrapping_sub(n).wrapping_mul(h.p3cd as i64))
                    .wrapping_add(nw.wrapping_sub(w).wrapping_mul(h.p3ce as i64))
                    >> 5,
            );

            self.pred = self.weighted_average(&self.prediction, weights);

            // If all three have the same sign, skip clamping.
            if ((te_n ^ te_w) | (te_n ^ te_nw)) > 0 {
                return self.pred.wrapping_add(K_PREDICTION_ROUND) >> K_PRED_EXTRA_BITS;
            }

            // Otherwise, clamp to min/max of neighbouring pixels (just W, NE, N).
            let mx = w.max(ne.max(n));
            let mn = w.min(ne.min(n));
            self.pred = mn.max(mx.min(self.pred));
            self.pred.wrapping_add(K_PREDICTION_ROUND) >> K_PRED_EXTRA_BITS
        }

        /// Translation of `UpdateErrors()`.
        #[inline]
        pub(crate) fn update_errors(&mut self, val: PixelTypeW, x: usize, y: usize, xsize: usize) {
            let cur_row = if y & 1 != 0 { 0 } else { xsize + 2 };
            let prev_row = if y & 1 != 0 { xsize + 2 } else { 0 };
            let val = add_bits(val);
            self.error[cur_row + x] = self.pred.wrapping_sub(val) as i32;
            for i in 0..K_NUM_PREDICTORS {
                let err: PixelTypeW = (self.prediction[i]
                    .wrapping_sub(val)
                    .wrapping_abs()
                    .wrapping_add(K_PREDICTION_ROUND))
                    >> K_PRED_EXTRA_BITS;
                // For predicting in the next row.
                self.pred_errors[i][cur_row + x] = err as u32;
                // Add the error on this pixel to the error on the NE pixel. This has the
                // effect of adding the error on this pixel to the E and EE pixels.
                let e = &mut self.pred_errors[i][prev_row + x + 1];
                *e = e.wrapping_add(err as u32);
            }
        }
    }

    #[allow(dead_code)]
    fn _unused(_: &Properties) {}
}

/// Stores a node and its two children at the same time. This significantly
/// reduces the number of branches needed during decoding. Translation of
/// `FlatDecisionNode` (its unions as separate fields: a leaf reads the
/// predictor ones, an inner node the split ones).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FlatDecisionNode {
    // Property + splitval of the top node.
    pub property0: i32, // -1 if leaf.
    pub splitval0: PropertyVal,
    pub predictor: Predictor,
    pub child_id: u32, // childID is ctx id if leaf.
    // Property+splitval of the two child nodes.
    pub splitvals: [PropertyVal; 2],
    pub multiplier: i32,
    pub properties: [i32; 2],
    pub predictor_offset: i64,
}

pub(crate) type FlatTree = Vec<FlatDecisionNode>;

/// Translation of `MATreeLookup::LookupResult`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LookupResult {
    pub context: u32,
    pub predictor: Predictor,
    pub offset: i64,
    pub multiplier: i32,
}

/// Translation of `MATreeLookup`.
pub(crate) struct MaTreeLookup<'a> {
    nodes: &'a FlatTree,
}

impl<'a> MaTreeLookup<'a> {
    pub(crate) fn new(tree: &'a FlatTree) -> Self {
        MaTreeLookup { nodes: tree }
    }

    #[inline]
    pub(crate) fn lookup(&self, properties: &Properties) -> LookupResult {
        let mut pos: u32 = 0;
        loop {
            let node = &self.nodes[pos as usize];
            if node.property0 < 0 {
                return LookupResult {
                    context: node.child_id,
                    predictor: node.predictor,
                    offset: node.predictor_offset,
                    multiplier: node.multiplier,
                };
            }
            let p0 = properties[node.property0 as usize] <= node.splitval0;
            let off0 = (properties[node.properties[0] as usize] <= node.splitvals[0]) as u32;
            let off1 = 2 | if properties[node.properties[1] as usize] <= node.splitvals[1] {
                1
            } else {
                0
            };
            pos = node.child_id + if p0 { off1 } else { off0 };
        }
    }
}

pub(crate) const K_EXTRA_PROPS_PER_CHANNEL: usize = 4;
pub(crate) const K_NUM_NONREF_PROPERTIES: usize =
    K_NUM_STATIC_PROPERTIES + 13 + weighted::K_NUM_PROPERTIES;

pub(crate) const K_WP_PROP: usize = K_NUM_NONREF_PROPERTIES - weighted::K_NUM_PROPERTIES;
pub(crate) const K_GRADIENT_PROP: usize = 9;

/// Clamps gradient to the min/max of n, w (and l, implicitly). Translation
/// of `ClampedGradient()`.
#[inline]
pub(crate) fn clamped_gradient(n: i32, w: i32, l: i32) -> i32 {
    let m = n.min(w);
    let mm = n.max(w);
    // The end result of this operation doesn't overflow or underflow if the
    // result is between m and M, but the intermediate value may overflow, so we
    // do the intermediate operations in uint32_t and check later if we had an
    // overflow or underflow condition comparing m, M and l directly.
    // grad = M + m - l = n + w - l
    let grad = (n as u32).wrapping_add(w as u32).wrapping_sub(l as u32) as i32;
    // We use two sets of ternary operators to force the evaluation of them in
    // any case, allowing the compiler to avoid branches and use cmovl/cmovg in
    // x86.
    let grad_clamp_m = if l < m { mm } else { grad };
    if l > mm {
        m
    } else {
        grad_clamp_m
    }
}

/// Translation of `Select()`.
#[inline]
pub(crate) fn select(a: PixelTypeW, b: PixelTypeW, c: PixelTypeW) -> PixelTypeW {
    let p = a.wrapping_add(b).wrapping_sub(c);
    let pa = p.wrapping_sub(a).wrapping_abs();
    let pb = p.wrapping_sub(b).wrapping_abs();
    if pa < pb {
        a
    } else {
        b
    }
}

/// Translation of `PrecomputeReferences()`.
pub(crate) fn precompute_references(
    ch: &Channel,
    y: usize,
    image: &Image,
    i: u32,
    references: &mut Channel,
) {
    super::super::super::image::zero_fill_image(&mut references.plane);
    let mut offset: u32 = 0;
    let num_extra_props = references.w;
    let onerow = references.plane.pixels_per_row();
    let mut j = i as i32 - 1;
    while j >= 0 && (offset as usize) < num_extra_props {
        let cj = &image.channel[j as usize];
        let ci = &image.channel[i as usize];
        if cj.w != ci.w || cj.h != ci.h || cj.hshift != ci.hshift || cj.vshift != ci.vshift {
            j -= 1;
            continue;
        }
        let rpp = cj.row(y);
        let rpprev = cj.row(if y != 0 { y - 1 } else { 0 });
        let rdata = references.plane.data_mut();
        let mut rp = offset as usize;
        for x in 0..ch.w {
            let v: PixelTypeW = rpp[x] as i64;
            rdata[rp] = v.wrapping_abs() as PixelType;
            rdata[rp + 1] = v as PixelType;
            let vleft: PixelTypeW = if x != 0 { rpp[x - 1] as i64 } else { 0 };
            let vtop: PixelTypeW = if y != 0 { rpprev[x] as i64 } else { vleft };
            let vtopleft: PixelTypeW = if x != 0 && y != 0 {
                rpprev[x - 1] as i64
            } else {
                vleft
            };
            let vpredicted = clamped_gradient(vleft as i32, vtop as i32, vtopleft as i32) as i64;
            rdata[rp + 2] = v.wrapping_sub(vpredicted).wrapping_abs() as PixelType;
            rdata[rp + 3] = v.wrapping_sub(vpredicted) as PixelType;
            rp += onerow;
        }

        offset += K_EXTRA_PROPS_PER_CHANNEL as u32;
        j -= 1;
    }
}

/// Translation of `PredictionResult`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PredictionResult {
    pub context: i32,
    pub guess: PixelTypeW,
    pub predictor: Predictor,
    pub multiplier: i32,
}

/// Translation of `InitPropsRow()`.
#[inline]
pub(crate) fn init_props_row(
    p: &mut Properties,
    static_props: &[PixelType; K_NUM_STATIC_PROPERTIES],
    y: i32,
) {
    for i in 0..K_NUM_STATIC_PROPERTIES {
        p[i] = static_props[i];
    }
    p[2] = y;
    p[9] = 0; // local gradient.
}

pub(crate) mod detail {
    use super::*;

    pub(crate) const K_USE_TREE: u32 = 1;
    pub(crate) const K_USE_WP: u32 = 2;
    pub(crate) const K_FORCE_COMPUTE_PROPERTIES: u32 = 4;
    pub(crate) const K_ALL_PREDICTIONS: u32 = 8;
    pub(crate) const K_NO_EDGE_CASES: u32 = 16;

    /// Translation of `PredictOne()`.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn predict_one(
        p: Predictor,
        left: PixelTypeW,
        top: PixelTypeW,
        toptop: PixelTypeW,
        topleft: PixelTypeW,
        topright: PixelTypeW,
        leftleft: PixelTypeW,
        toprightright: PixelTypeW,
        wp_pred: PixelTypeW,
    ) -> PixelTypeW {
        match p {
            Predictor::Zero => 0,
            Predictor::Left => left,
            Predictor::Top => top,
            Predictor::Select => select(left, top, topleft),
            Predictor::Weighted => wp_pred,
            Predictor::Gradient => clamped_gradient(left as i32, top as i32, topleft as i32) as i64,
            Predictor::TopLeft => topleft,
            Predictor::TopRight => topright,
            Predictor::LeftLeft => leftleft,
            Predictor::Average0 => left.wrapping_add(top) / 2,
            Predictor::Average1 => left.wrapping_add(topleft) / 2,
            Predictor::Average2 => topleft.wrapping_add(top) / 2,
            Predictor::Average3 => top.wrapping_add(topright) / 2,
            Predictor::Average4 => {
                (6i64
                    .wrapping_mul(top)
                    .wrapping_sub(2i64.wrapping_mul(toptop))
                    .wrapping_add(7i64.wrapping_mul(left))
                    .wrapping_add(leftleft)
                    .wrapping_add(toprightright)
                    .wrapping_add(3i64.wrapping_mul(topright))
                    .wrapping_add(8))
                    / 16
            }
        }
    }

    /// Translation of `Predict<mode>()`: `data` is the channel's storage,
    /// `pos` the index of the current pixel.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn predict<const MODE: u32>(
        p: &mut Properties,
        w: usize,
        data: &[PixelType],
        pos: usize,
        onerow: usize,
        x: usize,
        y: usize,
        mut predictor: Predictor,
        lookup: Option<&MaTreeLookup<'_>>,
        references: Option<&Channel>,
        wp_state: Option<&mut weighted::State>,
        predictions: Option<&mut [PixelTypeW]>,
    ) -> PredictionResult {
        // We start in position 3 because of 2 static properties + y.
        let mut offset = 3usize;
        let compute_properties = MODE & K_USE_TREE != 0 || MODE & K_FORCE_COMPUTE_PROPERTIES != 0;
        let nec = MODE & K_NO_EDGE_CASES != 0;
        let at = |i: isize| -> PixelTypeW { data[(pos as isize + i) as usize] as i64 };
        let onerow = onerow as isize;
        let left: PixelTypeW = if nec || x != 0 {
            at(-1)
        } else if y != 0 {
            at(-onerow)
        } else {
            0
        };
        let top: PixelTypeW = if nec || y != 0 { at(-onerow) } else { left };
        let topleft: PixelTypeW = if nec || (x != 0 && y != 0) {
            at(-1 - onerow)
        } else {
            left
        };
        let topright: PixelTypeW = if nec || (x + 1 < w && y != 0) {
            at(1 - onerow)
        } else {
            top
        };
        let leftleft: PixelTypeW = if nec || x > 1 { at(-2) } else { left };
        let toptop: PixelTypeW = if nec || y > 1 {
            at(-onerow - onerow)
        } else {
            top
        };
        let toprightright: PixelTypeW = if nec || (x + 2 < w && y != 0) {
            at(2 - onerow)
        } else {
            topright
        };

        if compute_properties {
            // location
            p[offset] = x as PropertyVal;
            offset += 1;
            // neighbors
            p[offset] = top.wrapping_abs() as PropertyVal;
            offset += 1;
            p[offset] = left.wrapping_abs() as PropertyVal;
            offset += 1;
            p[offset] = top as PropertyVal;
            offset += 1;
            p[offset] = left as PropertyVal;
            offset += 1;

            // local gradient
            p[offset] = (left as PropertyVal).wrapping_sub(p[offset + 1]);
            offset += 1;
            // local gradient
            p[offset] = left.wrapping_add(top).wrapping_sub(topleft) as PropertyVal;
            offset += 1;

            // FFV1 context properties
            p[offset] = left.wrapping_sub(topleft) as PropertyVal;
            offset += 1;
            p[offset] = topleft.wrapping_sub(top) as PropertyVal;
            offset += 1;
            p[offset] = top.wrapping_sub(topright) as PropertyVal;
            offset += 1;
            p[offset] = top.wrapping_sub(toptop) as PropertyVal;
            offset += 1;
            p[offset] = left.wrapping_sub(leftleft) as PropertyVal;
            offset += 1;
        }

        let mut wp_pred: PixelTypeW = 0;
        if MODE & K_USE_WP != 0 {
            let wp_state = wp_state.expect("weighted predictor state");
            wp_pred = if compute_properties {
                wp_state.predict::<true>(x, y, w, top, left, topright, topleft, toptop, p, offset)
            } else {
                wp_state.predict::<false>(x, y, w, top, left, topright, topleft, toptop, p, offset)
            };
        }
        if !nec && compute_properties {
            offset += weighted::K_NUM_PROPERTIES;
            // Extra properties.
            let references = references.expect("references");
            let rp = references.row(x);
            for i in 0..references.w {
                p[offset] = rp[i];
                offset += 1;
            }
        }
        let mut result = PredictionResult::default();
        if MODE & K_USE_TREE != 0 {
            let lr = lookup.expect("tree").lookup(p);
            result.context = lr.context as i32;
            result.guess = lr.offset;
            result.multiplier = lr.multiplier;
            predictor = lr.predictor;
        }
        if MODE & K_ALL_PREDICTIONS != 0 {
            let predictions = predictions.expect("predictions");
            for i in 0..K_NUM_MODULAR_PREDICTORS {
                predictions[i] = predict_one(
                    Predictor::from_u32(i as u32),
                    left,
                    top,
                    toptop,
                    topleft,
                    topright,
                    leftleft,
                    toprightright,
                    wp_pred,
                );
            }
        }
        result.guess = result.guess.wrapping_add(predict_one(
            predictor,
            left,
            top,
            toptop,
            topleft,
            topright,
            leftleft,
            toprightright,
            wp_pred,
        ));
        result.predictor = predictor;

        result
    }
}

/// Translation of `PredictNoTreeNoWP()`.
#[inline]
pub(crate) fn predict_no_tree_no_wp(
    w: usize,
    data: &[PixelType],
    pos: usize,
    onerow: usize,
    x: usize,
    y: usize,
    predictor: Predictor,
) -> PredictionResult {
    let mut p = Properties::new();
    detail::predict::<0>(
        &mut p, w, data, pos, onerow, x, y, predictor, None, None, None, None,
    )
}

/// Translation of `PredictNoTreeWP()`.
#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) fn predict_no_tree_wp(
    w: usize,
    data: &[PixelType],
    pos: usize,
    onerow: usize,
    x: usize,
    y: usize,
    predictor: Predictor,
    wp_state: &mut weighted::State,
) -> PredictionResult {
    let mut p = Properties::new();
    detail::predict::<{ detail::K_USE_WP }>(
        &mut p,
        w,
        data,
        pos,
        onerow,
        x,
        y,
        predictor,
        None,
        None,
        Some(wp_state),
        None,
    )
}

/// Translation of `PredictTreeNoWP()`.
#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) fn predict_tree_no_wp(
    p: &mut Properties,
    w: usize,
    data: &[PixelType],
    pos: usize,
    onerow: usize,
    x: usize,
    y: usize,
    tree_lookup: &MaTreeLookup<'_>,
    references: &Channel,
) -> PredictionResult {
    detail::predict::<{ detail::K_USE_TREE }>(
        p,
        w,
        data,
        pos,
        onerow,
        x,
        y,
        Predictor::Zero,
        Some(tree_lookup),
        Some(references),
        None,
        None,
    )
}

/// Only use for y > 1, x > 1, x < w-2, and empty references. Translation
/// of `PredictTreeNoWPNEC()`.
#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) fn predict_tree_no_wp_nec(
    p: &mut Properties,
    w: usize,
    data: &[PixelType],
    pos: usize,
    onerow: usize,
    x: usize,
    y: usize,
    tree_lookup: &MaTreeLookup<'_>,
    references: &Channel,
) -> PredictionResult {
    detail::predict::<{ detail::K_USE_TREE | detail::K_NO_EDGE_CASES }>(
        p,
        w,
        data,
        pos,
        onerow,
        x,
        y,
        Predictor::Zero,
        Some(tree_lookup),
        Some(references),
        None,
        None,
    )
}

/// Translation of `PredictTreeWP()`.
#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) fn predict_tree_wp(
    p: &mut Properties,
    w: usize,
    data: &[PixelType],
    pos: usize,
    onerow: usize,
    x: usize,
    y: usize,
    tree_lookup: &MaTreeLookup<'_>,
    references: &Channel,
    wp_state: &mut weighted::State,
) -> PredictionResult {
    detail::predict::<{ detail::K_USE_TREE | detail::K_USE_WP }>(
        p,
        w,
        data,
        pos,
        onerow,
        x,
        y,
        Predictor::Zero,
        Some(tree_lookup),
        Some(references),
        Some(wp_state),
        None,
    )
}

// (PredictLearn(), PredictLearnAll() and PredictAllNoWP() are the
// encoder's.)
