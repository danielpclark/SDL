// Rust translation of spirv_cfg.hpp and spirv_cfg.cpp from SPIRV-Cross.
// Copyright 2016-2021 Arm Limited
// SPDX-License-Identifier: Apache-2.0 OR MIT
// This is an altered (translated to Rust) version of the original
// software; see LICENSE.txt.

//! `CFG`: the control flow graph of a function, with its post-order and
//! immediate dominators, and `DominatorBuilder`.
//!
//! C++ keeps a reference to the compiler and the function in the CFG;
//! here the methods that need the compiler take it as a parameter.

use std::collections::{HashMap, HashSet};

use super::common::*;
use super::cross::Compiler;

#[derive(Clone, Copy, Debug)]
struct VisitOrder {
    order: i32,
    visited_resolve: bool,
    visited_branches: bool,
}

impl Default for VisitOrder {
    fn default() -> Self {
        VisitOrder {
            order: -1,
            visited_resolve: false,
            visited_branches: false,
        }
    }
}

/// `CFG`.
#[derive(Clone, Debug, Default)]
pub struct CFG {
    func: FunctionID,
    entry_block: BlockID,
    preceding_edges: HashMap<u32, Vec<u32>>,
    virtual_dominance_preceding_edges: HashMap<u32, Vec<u32>>,
    succeeding_edges: HashMap<u32, Vec<u32>>,
    immediate_dominators: HashMap<u32, u32>,
    visit_order: HashMap<u32, VisitOrder>,
    post_order: Vec<u32>,
    empty_vector: Vec<u32>,

    visit_count: u32,

    visit_stack: Vec<u32>,
    last_visited_size: usize,
}

fn add_unique(l: &mut Vec<u32>, value: u32) {
    if !l.contains(&value) {
        l.push(value);
    }
}

impl CFG {
    pub fn new(compiler: &Compiler, func: FunctionID) -> Result<CFG> {
        let entry_block = compiler.get::<SPIRFunction>(func)?.entry_block;
        let mut cfg = CFG {
            func,
            entry_block,
            ..Default::default()
        };
        cfg.build_post_order_visit_order(compiler)?;
        cfg.build_immediate_dominators()?;
        Ok(cfg)
    }

    /// `get_function()`: the function's id.
    pub fn get_function(&self) -> FunctionID {
        self.func
    }

    pub fn get_function_entry_block(&self) -> BlockID {
        self.entry_block
    }

    pub fn get_immediate_dominator(&self, block: u32) -> u32 {
        self.immediate_dominators.get(&block).copied().unwrap_or(0)
    }

    pub fn is_reachable(&self, block: u32) -> bool {
        self.visit_order.contains_key(&block)
    }

    pub fn get_visit_order(&self, block: u32) -> Result<u32> {
        // assert(itr != std::end(visit_order));
        // (C++ dereferences the end iterator when the block wasn't visited.)
        let Some(v) = self.visit_order.get(&block) else {
            spirv_cross_throw!("Block has no visit order.");
        };
        let v = v.order;
        // assert(v > 0);
        Ok(v as u32)
    }

    pub fn find_common_dominator(&self, mut a: u32, mut b: u32) -> Result<u32> {
        while a != b {
            if self.get_visit_order(a)? < self.get_visit_order(b)? {
                a = self.get_immediate_dominator(a);
            } else {
                b = self.get_immediate_dominator(b);
            }
        }
        Ok(a)
    }

    pub fn get_preceding_edges(&self, block: u32) -> &Vec<u32> {
        self.preceding_edges
            .get(&block)
            .unwrap_or(&self.empty_vector)
    }

    pub fn get_succeeding_edges(&self, block: u32) -> &Vec<u32> {
        self.succeeding_edges
            .get(&block)
            .unwrap_or(&self.empty_vector)
    }

    pub fn walk_from<F>(&self, seen_blocks: &mut HashSet<u32>, block: u32, op: &mut F) -> Result<()>
    where
        F: FnMut(u32) -> Result<bool>,
    {
        if seen_blocks.contains(&block) {
            return Ok(());
        }
        seen_blocks.insert(block);

        if op(block)? {
            for &b in self.get_succeeding_edges(block).clone().iter() {
                self.walk_from(seen_blocks, b, op)?;
            }
        }
        Ok(())
    }

    fn build_immediate_dominators(&mut self) -> Result<()> {
        // Traverse the post-order in reverse and build up the immediate dominator tree.
        self.immediate_dominators.clear();
        self.immediate_dominators
            .insert(self.entry_block, self.entry_block);

        for i in (1..=self.post_order.len()).rev() {
            let block = self.post_order[i - 1];

            for which in 0..2 {
                let pred = if which == 0 {
                    self.preceding_edges.entry(block).or_default().clone()
                } else {
                    self.virtual_dominance_preceding_edges
                        .entry(block)
                        .or_default()
                        .clone()
                };

                if pred.is_empty() {
                    // This is for the entry block, but we've already set up the dominators.
                    continue;
                }

                for &edge in &pred {
                    let cur = *self.immediate_dominators.entry(block).or_default();
                    if cur != 0 {
                        // assert(immediate_dominators[edge]);
                        let d = self.find_common_dominator(cur, edge)?;
                        self.immediate_dominators.insert(block, d);
                    } else {
                        self.immediate_dominators.insert(block, edge);
                    }
                }
            }
        }
        Ok(())
    }

    fn is_back_edge(&self, to: u32) -> bool {
        // We have a back edge if the visit order is set with the temporary magic value 0.
        // Crossing edges will have already been recorded with a visit order.
        match self.visit_order.get(&to) {
            Some(v) => v.visited_branches && !v.visited_resolve,
            None => false,
        }
    }

    fn has_visited_branch(&self, to: u32) -> bool {
        match self.visit_order.get(&to) {
            Some(v) => v.visited_branches,
            None => false,
        }
    }

    fn post_order_visit_entry(&mut self, compiler: &Compiler, block: u32) -> Result<()> {
        self.visit_stack.push(block);

        while !self.visit_stack.is_empty() {
            loop {
                // Reverse the order to allow for stack-like behavior and preserves the visit order from recursive algorithm.
                // Traverse depth first.
                let to_visit = *self.visit_stack.back()?;
                self.last_visited_size = self.visit_stack.len();
                self.post_order_visit_branches(compiler, to_visit)?;
                let keep_iterating = self.last_visited_size != self.visit_stack.len();
                if keep_iterating {
                    let start = self.last_visited_size;
                    self.visit_stack[start..].reverse();
                } else {
                    break;
                }
            }

            // We've reached the end of some tree leaf. Resolve the stack.
            // Any node which has been visited for real can be popped now.
            while let Some(&back) = self.visit_stack.last() {
                if !self.visit_order.entry(back).or_default().visited_branches {
                    break;
                }
                self.post_order_visit_resolve(compiler, back)?;
                self.visit_stack.pop();
            }
        }
        Ok(())
    }

    fn visit_branch(&mut self, block_id: u32) {
        // Prune obvious duplicates.
        if !self.visit_stack[self.last_visited_size..].contains(&block_id)
            && !self.has_visited_branch(block_id)
        {
            self.visit_stack.push(block_id);
        }
    }

    fn post_order_visit_branches(&mut self, compiler: &Compiler, block_id: u32) -> Result<()> {
        let block = compiler.get::<SPIRBlock>(block_id)?;

        let visit = self.visit_order.entry(block_id).or_default();
        if visit.visited_branches {
            return Ok(());
        }
        visit.visited_branches = true;

        if block.merge == Merge::MergeLoop {
            self.visit_branch(block.merge_block);
        } else if block.merge == Merge::MergeSelection {
            self.visit_branch(block.next_block);
        }

        // First visit our branch targets.
        match block.terminator {
            Terminator::Direct => self.visit_branch(block.next_block),

            Terminator::Select => {
                self.visit_branch(block.true_block);
                self.visit_branch(block.false_block);
            }

            Terminator::MultiSelect => {
                let cases = compiler.get_case_list(block)?;
                for target in cases {
                    self.visit_branch(target.block);
                }
                if block.default_block != 0 {
                    self.visit_branch(block.default_block);
                }
            }

            _ => {}
        }
        Ok(())
    }

    fn post_order_visit_resolve(&mut self, compiler: &Compiler, block_id: u32) -> Result<()> {
        let block = compiler.get::<SPIRBlock>(block_id)?;

        // assert(visit_block.visited_branches);
        if self
            .visit_order
            .entry(block_id)
            .or_default()
            .visited_resolve
        {
            return Ok(());
        }

        // If this is a loop header, add an implied branch to the merge target.
        // This is needed to avoid annoying cases with do { ... } while(false) loops often generated by inliners.
        // To the CFG, this is linear control flow, but we risk picking the do/while scope as our dominating block.
        // This makes sure that if we are accessing a variable outside the do/while, we choose the loop header as dominator.
        // We could use has_visited_forward_edge, but this break code-gen where the merge block is unreachable in the CFG.

        // Make a point out of visiting merge target first. This is to make sure that post visit order outside the loop
        // is lower than inside the loop, which is going to be key for some traversal algorithms like post-dominance analysis.
        // For selection constructs true/false blocks will end up visiting the merge block directly and it works out fine,
        // but for loops, only the header might end up actually branching to merge block.
        if block.merge == Merge::MergeLoop && !self.is_back_edge(block.merge_block) {
            self.add_branch(block_id, block.merge_block);
        }

        // Similar case as do/while loops, but expressed in a different form.
        // if (true) { foo = 1; } else { return/unreachable/kill/blah; } access(foo);
        // Only consider this branch when computing dominance to avoid breaking other analysis like
        // parameter preservation.
        if block.merge == Merge::MergeSelection && !self.is_back_edge(block.next_block) {
            self.add_virtual_dominance_branch(block_id, block.next_block);
        }

        // First visit our branch targets.
        match block.terminator {
            Terminator::Direct => {
                if !self.is_back_edge(block.next_block) {
                    self.add_branch(block_id, block.next_block);
                }
            }

            Terminator::Select => {
                if !self.is_back_edge(block.true_block) {
                    self.add_branch(block_id, block.true_block);
                }
                if !self.is_back_edge(block.false_block) {
                    self.add_branch(block_id, block.false_block);
                }
            }

            Terminator::MultiSelect => {
                let cases = compiler.get_case_list(block)?;
                for target in cases {
                    if !self.is_back_edge(target.block) {
                        self.add_branch(block_id, target.block);
                    }
                }
                if block.default_block != 0 && !self.is_back_edge(block.default_block) {
                    self.add_branch(block_id, block.default_block);
                }
            }
            _ => {}
        }

        // If this is a selection merge, add an implied branch to the merge target.
        // This is needed to avoid cases where an inner branch dominates the outer branch.
        // This can happen if one of the branches exit early, e.g.:
        // if (cond) { ...; break; } else { var = 100 } use_var(var);
        // We can use the variable without a Phi since there is only one possible parent here.
        // However, in this case, we need to hoist out the inner variable to outside the branch.
        // Use same strategy as loops.
        if block.merge == Merge::MergeSelection && !self.is_back_edge(block.next_block) {
            // If there is only one preceding edge to the merge block and it's not ourselves, we need a fixup.
            // Add a fake branch so any dominator in either the if (), or else () block, or a lone case statement
            // will be hoisted out to outside the selection merge.
            // If size > 1, the variable will be automatically hoisted, so we should not mess with it.
            // The exception here is switch blocks, where we can have multiple edges to merge block,
            // all coming from same scope, so be more conservative in this case.
            // Adding fake branches unconditionally breaks parameter preservation analysis,
            // which looks at how variables are accessed through the CFG.
            if let Some(pred) = self.preceding_edges.get(&block.next_block) {
                let pred = pred.clone();
                let num_succeeding_edges = self
                    .succeeding_edges
                    .get(&block_id)
                    .map(|s| s.len())
                    .unwrap_or(0);

                if block.terminator == Terminator::MultiSelect && num_succeeding_edges == 1 {
                    // Multiple branches can come from the same scope due to "break;", so we need to assume that all branches
                    // come from same case scope in worst case, even if there are multiple preceding edges.
                    // If we have more than one succeeding edge from the block header, it should be impossible
                    // to have a dominator be inside the block.
                    // Only case this can go wrong is if we have 2 or more edges from block header and
                    // 2 or more edges to merge block, and still have dominator be inside a case label.
                    if !pred.is_empty() {
                        self.add_branch(block_id, block.next_block);
                    }
                } else if pred.len() == 1 && pred[0] != block_id {
                    self.add_branch(block_id, block.next_block);
                }
            } else {
                // If the merge block does not have any preceding edges, i.e. unreachable, hallucinate it.
                // We're going to do code-gen for it, and domination analysis requires that we have at least one preceding edge.
                self.add_branch(block_id, block.next_block);
            }
        }

        self.visit_count += 1;
        let count = self.visit_count;
        let visit_block = self.visit_order.entry(block_id).or_default();
        visit_block.visited_resolve = true;
        visit_block.order = count as i32;
        self.post_order.push(block_id);
        Ok(())
    }

    fn build_post_order_visit_order(&mut self, compiler: &Compiler) -> Result<()> {
        let block = self.entry_block;
        self.visit_count = 0;
        self.visit_order.clear();
        self.post_order.clear();
        self.post_order_visit_entry(compiler, block)
    }

    fn add_branch(&mut self, from: u32, to: u32) {
        // assert(from && to);
        add_unique(self.preceding_edges.entry(to).or_default(), from);
        add_unique(self.succeeding_edges.entry(from).or_default(), to);
    }

    fn add_virtual_dominance_branch(&mut self, from: u32, to: u32) {
        // assert(from && to);
        add_unique(
            self.virtual_dominance_preceding_edges
                .entry(to)
                .or_default(),
            from,
        );
    }

    pub fn find_loop_dominator(&self, compiler: &Compiler, mut block_id: u32) -> Result<u32> {
        while block_id != SPIRBlock::NO_DOMINATOR {
            let Some(preds) = self.preceding_edges.get(&block_id) else {
                return Ok(SPIRBlock::NO_DOMINATOR);
            };
            if preds.is_empty() {
                return Ok(SPIRBlock::NO_DOMINATOR);
            }

            let mut pred_block_id = SPIRBlock::NO_DOMINATOR;
            let mut ignore_loop_header = false;

            // If we are a merge block, go directly to the header block.
            // Only consider a loop dominator if we are branching from inside a block to a loop header.
            // NOTE: In the CFG we forced an edge from header to merge block always to support variable scopes properly.
            for &pred in preds {
                let pred_block = compiler.get::<SPIRBlock>(pred)?;
                if pred_block.merge == Merge::MergeLoop && pred_block.merge_block == block_id {
                    pred_block_id = pred;
                    ignore_loop_header = true;
                    break;
                } else if pred_block.merge == Merge::MergeSelection
                    && pred_block.next_block == block_id
                {
                    pred_block_id = pred;
                    break;
                }
            }

            // No merge block means we can just pick any edge. Loop headers dominate the inner loop, so any path we
            // take will lead there.
            if pred_block_id == SPIRBlock::NO_DOMINATOR {
                pred_block_id = preds[0];
            }

            block_id = pred_block_id;

            if !ignore_loop_header && block_id != 0 {
                let block = compiler.get::<SPIRBlock>(block_id)?;
                if block.merge == Merge::MergeLoop {
                    return Ok(block_id);
                }
            }
        }

        Ok(block_id)
    }

    pub fn node_terminates_control_flow_in_sub_graph(
        &self,
        compiler: &Compiler,
        from: BlockID,
        mut to: BlockID,
    ) -> Result<bool> {
        // Walk backwards, starting from "to" block.
        // Only follow pred edges if they have a 1:1 relationship, or a merge relationship.
        // If we cannot find a path to "from", we must assume that to is inside control flow in some way.

        let from_block = compiler.get::<SPIRBlock>(from)?;
        let mut ignore_block_id: BlockID = 0;
        if from_block.merge == Merge::MergeLoop {
            ignore_block_id = from_block.merge_block;
        }

        while to != from {
            let Some(preds) = self.preceding_edges.get(&to) else {
                return Ok(false);
            };

            let mut builder = DominatorBuilder::new();
            for &edge in preds {
                builder.add_block(self, edge)?;
            }

            let dominator = builder.get_dominator();
            if dominator == 0 {
                return Ok(false);
            }

            let dom = compiler.get::<SPIRBlock>(dominator)?;

            let mut true_path_ignore = false;
            let mut false_path_ignore = false;

            let merges_to_nothing = dom.merge == Merge::MergeNone
                || (dom.merge == Merge::MergeSelection
                    && dom.next_block != 0
                    && compiler.get::<SPIRBlock>(dom.next_block)?.terminator
                        == Terminator::Unreachable)
                || (dom.merge == Merge::MergeLoop
                    && dom.merge_block != 0
                    && compiler.get::<SPIRBlock>(dom.merge_block)?.terminator
                        == Terminator::Unreachable);

            if dom.self_ == from || merges_to_nothing {
                // We can only ignore inner branchy paths if there is no merge,
                // i.e. no code is generated afterwards. E.g. this allows us to elide continue:
                // for (;;) { if (cond) { continue; } else { break; } }.
                // Codegen here in SPIR-V will be something like either no merge if one path directly breaks, or
                // we merge to Unreachable.
                if ignore_block_id != 0 && dom.terminator == Terminator::Select {
                    let true_block = compiler.get::<SPIRBlock>(dom.true_block)?;
                    let false_block = compiler.get::<SPIRBlock>(dom.false_block)?;
                    let ignore_block = compiler.get::<SPIRBlock>(ignore_block_id)?;
                    true_path_ignore =
                        compiler.execution_is_branchless(true_block, ignore_block)?;
                    false_path_ignore =
                        compiler.execution_is_branchless(false_block, ignore_block)?;
                }
            }

            // Cases where we allow traversal. This serves as a proxy for post-dominance in a loop body.
            // TODO: Might want to do full post-dominance analysis, but it's a lot of churn for something like this ...
            // - We're the merge block of a selection construct. Jump to header.
            // - We're the merge block of a loop. Jump to header.
            // - Direct branch. Trivial.
            // - Allow cases inside a branch if the header cannot merge execution before loop exit.
            if (dom.merge == Merge::MergeSelection && dom.next_block == to)
                || (dom.merge == Merge::MergeLoop && dom.merge_block == to)
                || (dom.terminator == Terminator::Direct && dom.next_block == to)
                || (dom.terminator == Terminator::Select
                    && dom.true_block == to
                    && false_path_ignore)
                || (dom.terminator == Terminator::Select
                    && dom.false_block == to
                    && true_path_ignore)
            {
                // Allow walking selection constructs if the other branch reaches out of a loop construct.
                // It cannot be in-scope anymore.
                to = dominator;
            } else {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

/// `DominatorBuilder`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DominatorBuilder {
    dominator: u32,
}

impl DominatorBuilder {
    pub fn new() -> Self {
        DominatorBuilder { dominator: 0 }
    }

    pub fn add_block(&mut self, cfg: &CFG, block: u32) -> Result<()> {
        if cfg.get_immediate_dominator(block) == 0 {
            // Unreachable block via the CFG, we will never emit this code anyways.
            return Ok(());
        }

        if self.dominator == 0 {
            self.dominator = block;
            return Ok(());
        }

        if block != self.dominator {
            self.dominator = cfg.find_common_dominator(block, self.dominator)?;
        }
        Ok(())
    }

    pub fn get_dominator(&self) -> u32 {
        self.dominator
    }

    pub fn lift_continue_block_dominator(&mut self, compiler: &Compiler, cfg: &CFG) -> Result<()> {
        // It is possible for a continue block to be the dominator of a variable is only accessed inside the while block of a do-while loop.
        // We cannot safely declare variables inside a continue block, so move any variable declared
        // in a continue block to the entry block to simplify.
        // It makes very little sense for a continue block to ever be a dominator, so fall back to the simplest
        // solution.

        if self.dominator == 0 {
            return Ok(());
        }

        let block = compiler.get::<SPIRBlock>(self.dominator)?;
        let post_order = cfg.get_visit_order(self.dominator)?;

        // If we are branching to a block with a higher post-order traversal index (continue blocks), we have a problem
        // since we cannot create sensible GLSL code for this, fallback to entry block.
        let mut back_edge_dominator = false;
        match block.terminator {
            Terminator::Direct => {
                if cfg.get_visit_order(block.next_block)? > post_order {
                    back_edge_dominator = true;
                }
            }

            Terminator::Select => {
                if cfg.get_visit_order(block.true_block)? > post_order {
                    back_edge_dominator = true;
                }
                if cfg.get_visit_order(block.false_block)? > post_order {
                    back_edge_dominator = true;
                }
            }

            Terminator::MultiSelect => {
                let cases = compiler.get_case_list(block)?;
                for target in cases {
                    if cfg.get_visit_order(target.block)? > post_order {
                        back_edge_dominator = true;
                    }
                }
                if block.default_block != 0
                    && cfg.get_visit_order(block.default_block)? > post_order
                {
                    back_edge_dominator = true;
                }
            }

            _ => {}
        }

        if back_edge_dominator {
            self.dominator = cfg.get_function_entry_block();
        }
        Ok(())
    }
}
