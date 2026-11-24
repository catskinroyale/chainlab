//! # Sink (the Tower Builder)
//!
//! The end of the chain. A sink doesn't make goods; it *consumes* them to
//! make **Progress**, the long-term goal number. It eats complete **sets**
//! (for the Tower: 2 Bricks + 1 Plank), so the scarcer input decides the pace.
//! Surplus of the other input just piles up in its buffer. That's the
//! "convergence" lesson: two branches have to be *balanced*, not just fast.
//!
//! A sink is also limited by its **lift** (scaffold) rate: the most sets per
//! second it can physically move into place:
//!
//! ```text
//! sets this tick = min(lift/tick * crew * buildings, complete sets in buffers)
//! progress       = sets * progress_per_set * efficiency%
//! ```
//!
//! Status: **Working** when the lift is the limit (inputs are piling up, so
//! upgrade the lift); **Starved** when it had spare lift capacity this tick
//! (inputs are the limit, so look upstream). Both can make progress.

use crate::TICKS_PER_SEC;
use crate::fixed::{Fx, div, mul, pct};

use super::chain::{Chain, Kind, Node, SinkSpec};
use super::model::{NodeState, State, Status};

pub fn tick_all(chain: &Chain, eff: &[i64], st: &mut State) {
    let dim = eff[chain.g.diminishing];
    for &ni in &chain.order {
        let node = &chain.nodes[ni];
        let Kind::Sink(spec) = &node.kind else {
            continue;
        };
        let ns = &mut st.nodes[ni];
        match super::crew_gate(node, eff, dim) {
            Ok(crew) => st.progress += tick(node, spec, eff, ns, crew),
            Err(s) => ns.status = s,
        }
    }
}

/// One tick of one sink. Returns progress made.
pub fn tick(node: &Node, spec: &SinkSpec, eff: &[i64], ns: &mut NodeState, crew: Fx) -> Fx {
    let units = node.p.units.map_or(1, |p| eff[p]);
    let lift_tick = node.p.lift.map_or(0, |p| eff[p]) * units / TICKS_PER_SEC as i64;
    let cap = mul(lift_tick, crew);
    let avail = spec
        .inputs
        .iter()
        .enumerate()
        .map(|(i, &q)| div(ns.inputs[i], q))
        .min()
        .unwrap_or(0);
    let sets = cap.min(avail);
    if sets <= 0 {
        ns.status = Status::Starved;
        return 0;
    }
    for (i, &q) in spec.inputs.iter().enumerate() {
        ns.inputs[i] -= mul(sets, q);
    }
    let progress = pct(mul(sets, spec.progress), eff[node.p.efficiency]);
    ns.out_tick = progress;
    ns.status = if sets < cap {
        Status::Starved
    } else {
        Status::Working
    };
    progress
}
