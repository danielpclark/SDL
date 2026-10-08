// Rust translation of src/enc/backward_references_cost_enc.c from libwebp
// (https://chromium.googlesource.com/webm/libwebp, as SDL_image's
// external/libwebp pins it).
// Copyright 2017 Google Inc. All Rights Reserved.
// SPDX-License-Identifier: BSD-3-Clause (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! Improves a given set of backward references by analyzing its bit cost.
//! The algorithm is similar to the Zopfli compression algorithm but tailored
//! to images.
//!
//! The cost manager's intervals live in a `Vec` (an arena) linked by
//! indices; upstream's fixed free list and its recycled malloc'd
//! intervals become one free list of arena slots (which slot an interval
//! takes never changes the costs).

use crate::webp::decode::NUM_DISTANCE_CODES;
use crate::webp::dsp::lossless_enc::{vp8l_fast_log2, vp8l_prefix_encode_bits};
use crate::webp::enc::backward_references_enc::{
    vp8l_backward_refs_cursor_add, vp8l_clear_backward_refs, vp8l_distance_to_plane_code,
    PixOrCopy, VP8LBackwardRefs, VP8LHashChain, MAX_LENGTH,
};
use crate::webp::enc::histogram_enc::{
    vp8l_histogram_add_single_pix_or_copy, vp8l_histogram_num_codes, VP8LHistogram,
};
use crate::webp::utils::color_cache_utils::{
    vp8l_color_cache_clear, vp8l_color_cache_init, VP8LColorCache,
};

const VALUES_IN_BYTE: usize = 256;
const NUM_LENGTH_CODES: usize = 24;

/// Translation of `CostModel`.
struct CostModel {
    alpha: [f32; VALUES_IN_BYTE],
    red: [f32; VALUES_IN_BYTE],
    blue: [f32; VALUES_IN_BYTE],
    distance: [f32; NUM_DISTANCE_CODES as usize],
    literal: Vec<f32>,
}

/// Translation of `ConvertPopulationCountTableToBitEstimates()`.
fn convert_population_count_table_to_bit_estimates(
    num_symbols: usize,
    population_counts: &[u32],
    output: &mut [f32],
) {
    let mut sum = 0u32;
    let mut nonzeros = 0;
    for &p in &population_counts[..num_symbols] {
        sum = sum.wrapping_add(p);
        if p > 0 {
            nonzeros += 1;
        }
    }
    if nonzeros <= 1 {
        output[..num_symbols].fill(0.0);
    } else {
        let logsum = vp8l_fast_log2(sum);
        for i in 0..num_symbols {
            output[i] = logsum - vp8l_fast_log2(population_counts[i]);
        }
    }
}

/// Translation of `CostModelBuild()`.
fn cost_model_build(
    m: &mut CostModel,
    xsize: i32,
    cache_bits: i32,
    refs: &VP8LBackwardRefs,
) -> bool {
    let mut histo = VP8LHistogram::new(cache_bits);

    // The following code is similar to VP8LHistogramCreate but converts the
    // distance to plane code.
    for v in &refs.refs {
        vp8l_histogram_add_single_pix_or_copy(
            &mut histo,
            v,
            Some(vp8l_distance_to_plane_code),
            xsize,
        );
    }

    convert_population_count_table_to_bit_estimates(
        vp8l_histogram_num_codes(histo.palette_code_bits) as usize,
        &histo.literal,
        &mut m.literal,
    );
    convert_population_count_table_to_bit_estimates(VALUES_IN_BYTE, &histo.red, &mut m.red);
    convert_population_count_table_to_bit_estimates(VALUES_IN_BYTE, &histo.blue, &mut m.blue);
    convert_population_count_table_to_bit_estimates(VALUES_IN_BYTE, &histo.alpha, &mut m.alpha);
    convert_population_count_table_to_bit_estimates(
        NUM_DISTANCE_CODES as usize,
        &histo.distance,
        &mut m.distance,
    );
    true
}

/// Translation of `GetLiteralCost()`.
fn get_literal_cost(m: &CostModel, v: u32) -> f32 {
    m.alpha[(v >> 24) as usize]
        + m.red[((v >> 16) & 0xff) as usize]
        + m.literal[((v >> 8) & 0xff) as usize]
        + m.blue[(v & 0xff) as usize]
}

/// Translation of `GetCacheCost()`.
fn get_cache_cost(m: &CostModel, idx: u32) -> f32 {
    let literal_idx = VALUES_IN_BYTE + NUM_LENGTH_CODES + idx as usize;
    m.literal[literal_idx]
}

/// Translation of `GetLengthCost()`.
fn get_length_cost(m: &CostModel, length: u32) -> f32 {
    let (code, extra_bits) = vp8l_prefix_encode_bits(length as i32);
    m.literal[VALUES_IN_BYTE + code as usize] + extra_bits as f32
}

/// Translation of `GetDistanceCost()`.
fn get_distance_cost(m: &CostModel, distance: u32) -> f32 {
    let (code, extra_bits) = vp8l_prefix_encode_bits(distance as i32);
    m.distance[code as usize] + extra_bits as f32
}

/// Translation of `AddSingleLiteralWithCostModel()`.
#[allow(clippy::too_many_arguments)]
fn add_single_literal_with_cost_model(
    argb: &[u32],
    hashers: &mut VP8LColorCache,
    cost_model: &CostModel,
    idx: usize,
    use_color_cache: bool,
    prev_cost: f32,
    cost: &mut [f32],
    dist_array: &mut [u16],
) {
    let mut cost_val = prev_cost;
    let color = argb[idx];
    let ix = if use_color_cache {
        hashers.contains(color)
    } else {
        -1
    };
    if ix >= 0 {
        // use_color_cache is true and hashers contains color
        let mul0 = 0.68f32;
        cost_val += get_cache_cost(cost_model, ix as u32) * mul0;
    } else {
        let mul1 = 0.82f32;
        if use_color_cache {
            hashers.insert(color);
        }
        cost_val += get_literal_cost(cost_model, color) * mul1;
    }
    if cost[idx] > cost_val {
        cost[idx] = cost_val;
        dist_array[idx] = 1; // only one is inserted.
    }
}

// -----------------------------------------------------------------------------
// CostManager and interval handling

/// Empirical value to avoid high memory consumption but good for
/// performance.
const COST_CACHE_INTERVAL_SIZE_MAX: i32 = 500;

/// To perform backward reference every pixel at index index_ is considered
/// and the cost for the MAX_LENGTH following pixels computed. Those
/// following pixels at index index_ + k (k from 0 to MAX_LENGTH) have a
/// cost of:
///     cost_ = distance cost at index + GetLengthCost(cost_model, k)
/// and the minimum value is kept. GetLengthCost(cost_model, k) is cached in
/// an array of size MAX_LENGTH.
/// Instead of performing MAX_LENGTH comparisons per pixel, we keep track of
/// the minimal values using intervals of constant cost.
/// An interval is defined by the index_ of the pixel that generated it and
/// is only useful in a range of indices from start_ to end_ (exclusive),
/// i.e. it contains the minimum value for pixels between start_ and end_.
/// Intervals are stored in a linked list and ordered by start_. When a new
/// interval has a better value, old intervals are split or removed. There
/// are therefore no overlapping intervals. Translation of `CostInterval`
/// (the links are arena indices).
#[derive(Clone, Copy, Default)]
struct CostInterval {
    cost: f32,
    start: i32,
    end: i32,
    index: i32,
    previous: Option<usize>,
    next: Option<usize>,
}

/// The GetLengthCost(cost_model, k) are cached in a CostCacheInterval.
/// Translation of `CostCacheInterval`.
#[derive(Clone, Copy, Default)]
struct CostCacheInterval {
    cost: f32,
    start: i32,
    /// Exclusive.
    end: i32,
}

/// This structure is in charge of managing intervals and costs.
/// It caches the different CostCacheInterval, caches the different
/// GetLengthCost(cost_model, k) in cost_cache_ and the CostInterval's
/// (whose count_ is limited by COST_CACHE_INTERVAL_SIZE_MAX). Translation
/// of `CostManager`.
struct CostManager<'d> {
    head: Option<usize>,
    /// The number of stored intervals.
    count: i32,
    cache_intervals: Vec<CostCacheInterval>,
    /// Contains the GetLengthCost(cost_model, k).
    cost_cache: Vec<f32>,
    costs: Vec<f32>,
    dist_array: &'d mut [u16],
    /// The intervals (the free ones are in `free_intervals`).
    intervals: Vec<CostInterval>,
    free_intervals: Vec<usize>,
}

impl<'d> CostManager<'d> {
    /// Translation of `CostManagerInit()`.
    fn new(dist_array: &'d mut [u16], pix_count: usize, cost_model: &CostModel) -> CostManager<'d> {
        let cost_cache_size = if pix_count > MAX_LENGTH as usize {
            MAX_LENGTH as usize
        } else {
            pix_count
        };
        let mut manager = CostManager {
            head: None,
            count: 0,
            cache_intervals: Vec::new(),
            cost_cache: vec![0.0; MAX_LENGTH as usize],
            costs: Vec::new(),
            dist_array,
            intervals: Vec::new(),
            free_intervals: Vec::new(),
        };

        // Fill in the cost_cache_.
        // Has to be done in two passes due to a GCC bug on i686
        // related to https://gcc.gnu.org/bugzilla/show_bug.cgi?id=323
        for i in 0..cost_cache_size {
            manager.cost_cache[i] = get_length_cost(cost_model, i as u32);
        }
        let mut cache_intervals_size = 1;
        for i in 1..cost_cache_size {
            // Get the number of bound intervals.
            if manager.cost_cache[i] != manager.cost_cache[i - 1] {
                cache_intervals_size += 1;
            }
        }

        // With the current cost model, we usually have below 20 intervals.
        // The worst case scenario with a cost model would be if every length has a
        // different cost, hence MAX_LENGTH but that is impossible with the current
        // implementation that spirals around a pixel.
        debug_assert!(cache_intervals_size <= MAX_LENGTH as usize);

        // Fill in the cache_intervals_.
        {
            // Consecutive values in cost_cache_ are compared and if a big enough
            // difference is found, a new interval is created and bounded.
            let mut cur = CostCacheInterval {
                start: 0,
                end: 1,
                cost: manager.cost_cache[0],
            };
            for i in 1..cost_cache_size {
                let cost_val = manager.cost_cache[i];
                if cost_val != cur.cost {
                    manager.cache_intervals.push(cur);
                    // Initialize an interval.
                    cur.start = i as i32;
                    cur.cost = cost_val;
                }
                cur.end = i as i32 + 1;
            }
            manager.cache_intervals.push(cur);
            debug_assert!(manager.cache_intervals.len() == cache_intervals_size);
        }

        // Set the initial costs_ high for every pixel as we will keep the minimum.
        manager.costs = vec![f32::MAX; pix_count];

        manager
    }

    /// Given the cost and the position that define an interval, update the
    /// cost at pixel 'i' if it is smaller than the previously computed
    /// value. Translation of `UpdateCost()`.
    fn update_cost(&mut self, i: i32, position: i32, cost: f32) {
        let k = i - position;
        debug_assert!(k >= 0 && k < MAX_LENGTH);

        if self.costs[i as usize] > cost {
            self.costs[i as usize] = cost;
            self.dist_array[i as usize] = (k + 1) as u16;
        }
    }

    /// Given the cost and the position that define an interval, update the
    /// cost for all the pixels between 'start' and 'end' excluded.
    /// Translation of `UpdateCostPerInterval()`.
    fn update_cost_per_interval(&mut self, start: i32, end: i32, position: i32, cost: f32) {
        for i in start..end {
            self.update_cost(i, position, cost);
        }
    }

    /// Given two intervals, make 'prev' be the previous one of 'next' in
    /// 'manager'. Translation of `ConnectIntervals()`.
    fn connect_intervals(&mut self, prev: Option<usize>, next: Option<usize>) {
        if let Some(prev) = prev {
            self.intervals[prev].next = next;
        } else {
            self.head = next;
        }

        if let Some(next) = next {
            self.intervals[next].previous = prev;
        }
    }

    /// Pop an interval in the manager. Translation of `PopInterval()`.
    fn pop_interval(&mut self, interval: Option<usize>) {
        let Some(interval) = interval else {
            return;
        };

        let CostInterval { previous, next, .. } = self.intervals[interval];
        self.connect_intervals(previous, next);
        self.free_intervals.push(interval);
        self.count -= 1;
        debug_assert!(self.count >= 0);
    }

    /// Update the cost at index i by going over all the stored intervals
    /// that overlap with i. If 'do_clean_intervals' is set to something
    /// different than 0, intervals that end before 'i' will be popped.
    /// Translation of `UpdateCostAtIndex()`.
    fn update_cost_at_index(&mut self, i: i32, do_clean_intervals: bool) {
        let mut current = self.head;

        while let Some(c) = current {
            if self.intervals[c].start > i {
                break;
            }
            let next = self.intervals[c].next;
            if self.intervals[c].end <= i {
                if do_clean_intervals {
                    // We have an outdated interval, remove it.
                    self.pop_interval(Some(c));
                }
            } else {
                let CostInterval { index, cost, .. } = self.intervals[c];
                self.update_cost(i, index, cost);
            }
            current = next;
        }
    }

    /// Given a current orphan interval and its previous interval, before
    /// it was orphaned (which can be NULL), set it at the right place in
    /// the list of intervals using the start_ ordering and the previous
    /// interval as a hint. Translation of `PositionOrphanInterval()`.
    fn position_orphan_interval(&mut self, current: usize, mut previous: Option<usize>) {
        if previous.is_none() {
            previous = self.head;
        }
        let current_start = self.intervals[current].start;
        while let Some(p) = previous {
            if current_start < self.intervals[p].start {
                previous = self.intervals[p].previous;
            } else {
                break;
            }
        }
        while let Some(p) = previous {
            match self.intervals[p].next {
                Some(n) if self.intervals[n].start < current_start => previous = Some(n),
                _ => break,
            }
        }

        if let Some(p) = previous {
            let next = self.intervals[p].next;
            self.connect_intervals(Some(current), next);
        } else {
            let head = self.head;
            self.connect_intervals(Some(current), head);
        }
        self.connect_intervals(previous, Some(current));
    }

    /// Insert an interval in the list contained in the manager by starting
    /// at interval_in as a hint. The intervals are sorted by start_ value.
    /// Translation of `InsertInterval()`.
    fn insert_interval(
        &mut self,
        interval_in: Option<usize>,
        cost: f32,
        position: i32,
        start: i32,
        end: i32,
    ) {
        if start >= end {
            return;
        }
        if self.count >= COST_CACHE_INTERVAL_SIZE_MAX {
            // Serialize the interval if we cannot store it.
            self.update_cost_per_interval(start, end, position, cost);
            return;
        }
        let interval_new = match self.free_intervals.pop() {
            Some(i) => i,
            None => {
                self.intervals.push(CostInterval::default());
                self.intervals.len() - 1
            }
        };

        self.intervals[interval_new] = CostInterval {
            cost,
            index: position,
            start,
            end,
            previous: None,
            next: None,
        };
        self.position_orphan_interval(interval_new, interval_in);

        self.count += 1;
    }

    /// Given a new cost interval defined by its start at position, its
    /// length value and distance_cost, add its contributions to the
    /// previous intervals and costs. If handling the interval or one of its
    /// subintervals becomes to heavy, its contribution is added to the
    /// costs right away. Translation of `PushInterval()`.
    fn push_interval(&mut self, distance_cost: f32, position: i32, len: i32) {
        let mut interval = self.head;
        // If the interval is small enough, no need to deal with the heavy
        // interval logic, just serialize it right away. This constant is empirical.
        let k_skip_distance = 10;

        if len < k_skip_distance {
            for j in position..position + len {
                let k = j - position;
                debug_assert!(k >= 0 && k < MAX_LENGTH);
                let cost_tmp = distance_cost + self.cost_cache[k as usize];

                if self.costs[j as usize] > cost_tmp {
                    self.costs[j as usize] = cost_tmp;
                    self.dist_array[j as usize] = (k + 1) as u16;
                }
            }
            return;
        }

        let mut i = 0;
        while i < self.cache_intervals.len() && self.cache_intervals[i].start < len {
            // Define the intersection of the ith interval with the new one.
            let cci = self.cache_intervals[i];
            let mut start = position + cci.start;
            let end = position + if cci.end > len { len } else { cci.end };
            let cost = distance_cost + cci.cost;

            while let Some(iv) = interval {
                if self.intervals[iv].start >= end {
                    break;
                }
                let interval_next = self.intervals[iv].next;

                // Make sure we have some overlap
                if start >= self.intervals[iv].end {
                    interval = interval_next;
                    continue;
                }

                if cost >= self.intervals[iv].cost {
                    // When intervals are represented, the lower, the better.
                    // [**********************************************************[
                    // start                                                    end
                    //                   [----------------------------------[
                    //                   interval->start_       interval->end_
                    // If we are worse than what we already have, add whatever we have so
                    // far up to interval.
                    let start_new = self.intervals[iv].end;
                    let iv_start = self.intervals[iv].start;
                    self.insert_interval(Some(iv), cost, position, start, iv_start);
                    start = start_new;
                    if start >= end {
                        break;
                    }
                    interval = interval_next;
                    continue;
                }

                if start <= self.intervals[iv].start {
                    if self.intervals[iv].end <= end {
                        //                   [----------------------------------[
                        //                   interval->start_       interval->end_
                        // [**************************************************************[
                        // start                                                        end
                        // We can safely remove the old interval as it is fully included.
                        self.pop_interval(Some(iv));
                    } else {
                        //              [------------------------------------[
                        //              interval->start_        interval->end_
                        // [*****************************[
                        // start                       end
                        self.intervals[iv].start = end;
                        break;
                    }
                } else if end < self.intervals[iv].end {
                    // [--------------------------------------------------------------[
                    // interval->start_                                  interval->end_
                    //                     [*****************************[
                    //                     start                       end
                    // We have to split the old interval as it fully contains the new one.
                    let end_original = self.intervals[iv].end;
                    self.intervals[iv].end = start;
                    let CostInterval {
                        cost: iv_cost,
                        index: iv_index,
                        ..
                    } = self.intervals[iv];
                    self.insert_interval(Some(iv), iv_cost, iv_index, end, end_original);
                    interval = self.intervals[iv].next;
                    break;
                } else {
                    // [------------------------------------[
                    // interval->start_        interval->end_
                    //                     [*****************************[
                    //                     start                       end
                    self.intervals[iv].end = start;
                }
                interval = interval_next;
            }
            // Insert the remaining interval from start to end.
            self.insert_interval(interval, cost, position, start, end);
            i += 1;
        }
    }
}

/// Translation of `BackwardReferencesHashChainDistanceOnly()`.
fn backward_references_hash_chain_distance_only(
    xsize: i32,
    ysize: i32,
    argb: &[u32],
    cache_bits: i32,
    hash_chain: &VP8LHashChain,
    refs: &VP8LBackwardRefs,
    dist_array: &mut [u16],
) -> bool {
    let pix_count = (xsize * ysize) as usize;
    let use_color_cache = cache_bits > 0;
    let literal_array_size = vp8l_histogram_num_codes(cache_bits) as usize;
    let mut cost_model = CostModel {
        alpha: [0.0; VALUES_IN_BYTE],
        red: [0.0; VALUES_IN_BYTE],
        blue: [0.0; VALUES_IN_BYTE],
        distance: [0.0; NUM_DISTANCE_CODES as usize],
        literal: vec![0.0; literal_array_size],
    };
    let mut hashers = VP8LColorCache::default();
    let mut offset_prev: i32 = -1;
    let mut len_prev: i32 = -1;
    let mut offset_cost: f32 = -1.0;
    let mut first_offset_is_constant: i32 = -1; // initialized with 'impossible' value
    let mut reach: i32 = 0;

    if use_color_cache && !vp8l_color_cache_init(&mut hashers, cache_bits) {
        return false;
    }

    if !cost_model_build(&mut cost_model, xsize, cache_bits, refs) {
        return false;
    }

    let mut cost_manager = CostManager::new(dist_array, pix_count, &cost_model);

    // We loop one pixel at a time, but store all currently best points to
    // non-processed locations from this point.
    cost_manager.dist_array[0] = 0;
    // Add first pixel as literal.
    add_single_literal_with_cost_model(
        argb,
        &mut hashers,
        &cost_model,
        0,
        use_color_cache,
        0.0,
        &mut cost_manager.costs,
        cost_manager.dist_array,
    );

    for i in 1..pix_count as i32 {
        let prev_cost = cost_manager.costs[i as usize - 1];
        let (offset, len) = hash_chain.find_copy(i as usize);

        // Try adding the pixel as a literal.
        add_single_literal_with_cost_model(
            argb,
            &mut hashers,
            &cost_model,
            i as usize,
            use_color_cache,
            prev_cost,
            &mut cost_manager.costs,
            cost_manager.dist_array,
        );

        // If we are dealing with a non-literal.
        if len >= 2 {
            if offset != offset_prev {
                let code = vp8l_distance_to_plane_code(xsize, offset);
                offset_cost = get_distance_cost(&cost_model, code as u32);
                first_offset_is_constant = 1;
                cost_manager.push_interval(prev_cost + offset_cost, i, len);
            } else {
                debug_assert!(offset_cost >= 0.0);
                debug_assert!(len_prev >= 0);
                debug_assert!(first_offset_is_constant == 0 || first_offset_is_constant == 1);
                // Instead of considering all contributions from a pixel i by calling:
                //         PushInterval(cost_manager, prev_cost + offset_cost, i, len);
                // we optimize these contributions in case offset_cost stays the same
                // for consecutive pixels. This describes a set of pixels similar to a
                // previous set (e.g. constant color regions).
                if first_offset_is_constant != 0 {
                    reach = i - 1 + len_prev - 1;
                    first_offset_is_constant = 0;
                }

                if i + len - 1 > reach {
                    // We can only be go further with the same offset if the previous
                    // length was maxed, hence len_prev == len == MAX_LENGTH.
                    // TODO(vrabaud), bump i to the end right away (insert cache and
                    // update cost).
                    // TODO(vrabaud), check if one of the points in between does not have
                    // a lower cost.
                    // Already consider the pixel at "reach" to add intervals that are
                    // better than whatever we add.
                    let mut len_j = 0;
                    debug_assert!(len == MAX_LENGTH || len == pix_count as i32 - i);
                    // Figure out the last consecutive pixel within [i, reach + 1] with
                    // the same offset.
                    let mut j = i;
                    while j <= reach {
                        let (offset_j, l) = hash_chain.find_copy(j as usize + 1);
                        len_j = l;
                        if offset_j != offset {
                            let (_, l) = hash_chain.find_copy(j as usize);
                            len_j = l;
                            break;
                        }
                        j += 1;
                    }
                    // Update the cost at j - 1 and j.
                    cost_manager.update_cost_at_index(j - 1, false);
                    cost_manager.update_cost_at_index(j, false);

                    let c = cost_manager.costs[j as usize - 1] + offset_cost;
                    cost_manager.push_interval(c, j, len_j);
                    reach = j + len_j - 1;
                }
            }
        }

        cost_manager.update_cost_at_index(i, true);
        offset_prev = offset;
        len_prev = len;
    }

    let ok = !refs.error;
    if use_color_cache {
        vp8l_color_cache_clear(&mut hashers);
    }
    ok
}

/// We pack the path at the end of *dist_array and return
/// a pointer to this part of the array. Example:
/// dist_array = [1x2xx3x2] => packed [1x2x1232], chosen_path = [1232]
/// Translation of `TraceBackwards()`: the start of the chosen path in
/// `dist_array`.
fn trace_backwards(dist_array: &mut [u16]) -> usize {
    let dist_array_size = dist_array.len();
    let mut path = dist_array_size;
    let mut cur = dist_array_size as isize - 1;
    while cur >= 0 {
        let k = dist_array[cur as usize];
        path -= 1;
        dist_array[path] = k;
        cur -= k as isize;
    }
    path
}

/// Translation of `BackwardReferencesHashChainFollowChosenPath()`.
fn backward_references_hash_chain_follow_chosen_path(
    argb: &[u32],
    cache_bits: i32,
    chosen_path: &[u16],
    hash_chain: &VP8LHashChain,
    refs: &mut VP8LBackwardRefs,
) -> bool {
    let use_color_cache = cache_bits > 0;
    let mut i = 0usize;
    let mut hashers = VP8LColorCache::default();

    if use_color_cache && !vp8l_color_cache_init(&mut hashers, cache_bits) {
        return false;
    }

    vp8l_clear_backward_refs(refs);
    for &len in chosen_path {
        let len = len as usize;
        if len != 1 {
            let offset = hash_chain.find_offset(i);
            vp8l_backward_refs_cursor_add(refs, PixOrCopy::create_copy(offset as u32, len as u16));
            if use_color_cache {
                for k in 0..len {
                    hashers.insert(argb[i + k]);
                }
            }
            i += len;
        } else {
            let idx = if use_color_cache {
                hashers.contains(argb[i])
            } else {
                -1
            };
            let v = if idx >= 0 {
                // use_color_cache is true and hashers contains argb[i]
                // push pixel as a color cache index
                PixOrCopy::create_cache_idx(idx)
            } else {
                if use_color_cache {
                    hashers.insert(argb[i]);
                }
                PixOrCopy::create_literal(argb[i])
            };
            vp8l_backward_refs_cursor_add(refs, v);
            i += 1;
        }
    }
    let ok = !refs.error;
    if use_color_cache {
        vp8l_color_cache_clear(&mut hashers);
    }
    ok
}

/// Returns 1 on success. Translation of
/// `VP8LBackwardReferencesTraceBackwards()`.
pub(crate) fn vp8l_backward_references_trace_backwards(
    xsize: i32,
    ysize: i32,
    argb: &[u32],
    cache_bits: i32,
    hash_chain: &VP8LHashChain,
    refs_src: &VP8LBackwardRefs,
    refs_dst: &mut VP8LBackwardRefs,
) -> bool {
    let dist_array_size = (xsize * ysize) as usize;
    let mut dist_array = vec![0u16; dist_array_size];

    if !backward_references_hash_chain_distance_only(
        xsize,
        ysize,
        argb,
        cache_bits,
        hash_chain,
        refs_src,
        &mut dist_array,
    ) {
        return false;
    }
    let chosen_path = trace_backwards(&mut dist_array);
    backward_references_hash_chain_follow_chosen_path(
        argb,
        cache_bits,
        &dist_array[chosen_path..],
        hash_chain,
        refs_dst,
    )
}
