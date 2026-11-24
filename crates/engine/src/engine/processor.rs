//! # Processors (recipes)
//!
//! A processor turns inputs into outputs over a cycle, like a kiln firing
//! stone into bricks:
//!
//! ```text
//! 2 Stone  --[ 4.0 s ]-->  1 Brick        (x batch, x efficiency%)
//! ```
//!
//! The cycle is a state machine:
//!
//! 1. **Start**: if every input buffer holds a full batch *and* every output
//!    buffer has room for the result, take the inputs and begin. If inputs
//!    are short the node is **Starved**. If there's no room for the output
//!    it's **Blocked** (and it does *not* take inputs, so they stay available).
//! 2. **Work**: each tick adds the crew factor to progress. A full crew adds
//!    one tick of work per tick, half a crew adds half. The cycle completes
//!    when progress reaches the cycle time.
//! 3. **Finish**: the outputs (scaled by efficiency) go into the output
//!    buffer. Anything that doesn't fit (because you shrank the buffer
//!    mid-cycle) is held back, and the node stays Blocked until it clears.
//!    Leftover progress carries straight into the next cycle, so fast crews
//!    don't lose fractional ticks.
//!
//! Theoretical max output per second = `outputs * batch * eff% * crew /
//! cycle_secs`. Comparing that number between neighbours is how you predict a
//! bottleneck before you see one.

use crate::fixed::{Fx, ONE, pct};

use super::chain::{Chain, Kind, Node, RecipeSpec};
use super::model::{NodeState, State, Status};

pub fn tick_all(chain: &Chain, eff: &[i64], st: &mut State) {
    let dim = eff[chain.g.diminishing];
    for &ni in &chain.order {
        let node = &chain.nodes[ni];
        let Kind::Processor(recipe) = &node.kind else {
            continue;
        };
        let ns = &mut st.nodes[ni];
        match super::crew_gate(node, eff, dim) {
            Ok(crew) => tick(node, recipe, eff, ns, crew),
            Err(s) => ns.status = s,
        }
    }
}

enum Start {
    Ok,
    Starved,
    Blocked,
}

/// Recipe multiples per cycle: batch size times parallel buildings.
fn multiples(node: &Node, recipe: &RecipeSpec, eff: &[i64]) -> i64 {
    recipe.batch * node.p.units.map_or(1, |p| eff[p]).max(1)
}

/// Output per cycle after efficiency.
fn batch_out(recipe: &RecipeSpec, j: usize, eff_pct: i64, mult: i64) -> Fx {
    pct(recipe.outputs[j] * mult, eff_pct)
}

fn try_start(node: &Node, recipe: &RecipeSpec, eff: &[i64], ns: &mut NodeState) -> Start {
    let out_cap = node.p.out_cap.map_or(0, |p| eff[p]);
    let eff_pct = eff[node.p.efficiency];
    let mult = multiples(node, recipe, eff);
    for j in 0..recipe.outputs.len() {
        let space = out_cap - ns.outputs[j] - ns.pending_out[j];
        if space < batch_out(recipe, j, eff_pct, mult) {
            return Start::Blocked;
        }
    }
    for (i, &q) in recipe.inputs.iter().enumerate() {
        if ns.inputs[i] < q * mult {
            return Start::Starved;
        }
    }
    for (i, &q) in recipe.inputs.iter().enumerate() {
        ns.inputs[i] -= q * mult;
    }
    ns.in_cycle = true;
    Start::Ok
}

/// Move held-back output into the buffer. Returns true if anything is still held.
fn flush(ns: &mut NodeState, out_cap: Fx) -> bool {
    let mut held = false;
    for j in 0..ns.outputs.len() {
        let space = (out_cap - ns.outputs[j]).max(0);
        let mv = ns.pending_out[j].min(space);
        ns.outputs[j] += mv;
        ns.pending_out[j] -= mv;
        held |= ns.pending_out[j] > 0;
    }
    held
}

/// One tick of one processor with the given crew factor.
pub fn tick(node: &Node, recipe: &RecipeSpec, eff: &[i64], ns: &mut NodeState, crew: Fx) {
    let out_cap = node.p.out_cap.map_or(0, |p| eff[p]);
    if flush(ns, out_cap) {
        ns.status = Status::Blocked;
        return;
    }
    if !ns.in_cycle {
        match try_start(node, recipe, eff, ns) {
            Start::Ok => {}
            Start::Starved => {
                ns.progress = 0;
                ns.status = Status::Starved;
                return;
            }
            Start::Blocked => {
                ns.progress = 0;
                ns.status = Status::Blocked;
                return;
            }
        }
    }

    ns.status = Status::Working;
    ns.progress += crew;
    let cycle_total = node.p.cycle.map_or(1, |p| eff[p]).max(1) * ONE;
    let eff_pct = eff[node.p.efficiency];
    let mult = multiples(node, recipe, eff);
    while ns.in_cycle && ns.progress >= cycle_total {
        for j in 0..recipe.outputs.len() {
            let q = batch_out(recipe, j, eff_pct, mult);
            ns.pending_out[j] += q;
            if j == 0 {
                ns.out_tick += q;
            }
        }
        ns.progress -= cycle_total;
        ns.in_cycle = false;
        if flush(ns, out_cap) {
            break;
        }
        // Chain straight into the next cycle, keeping leftover progress.
        if !matches!(try_start(node, recipe, eff, ns), Start::Ok) {
            ns.progress = 0;
            break;
        }
    }
}
