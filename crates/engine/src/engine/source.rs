//! # Raw sources
//!
//! A source pulls a raw resource out of the world: a quarry cutting stone, a
//! forest being logged. Each tick it produces
//!
//! ```text
//! yield = rate/tick * buildings * crew factor * efficiency% * (1 +/- variance)
//! ```
//!
//! capped by the free space in its output buffer. If that buffer is full the
//! source is **Blocked**: it could work, but nothing downstream is taking its
//! output. That's the first sign of a bottleneck further down the chain.
//!
//! There are three kinds of stock:
//!
//! * **Infinite**: never runs out (a quarry face).
//! * **Depleting**: a fixed deposit. When it's empty the source is
//!   **Starved** for good.
//! * **Regrowing**: a living stock (a forest) that grows *logistically*:
//!
//!   ```text
//!   growth/tick = r * water_factor * S * (1 - S / K)
//!   ```
//!
//!   where `S` is the standing stock, `K` the maximum and `r` the regrowth
//!   lever. Growth is fastest at half capacity and slow when the forest is
//!   nearly empty *or* nearly full, so over-harvesting is punished twice:
//!   less to cut, and slower recovery. `water_factor = min(1, supply/need)`
//!   ties growth to the water lever. A small seedling floor (1% of `K`) lets a
//!   clear-cut forest eventually recover.

use crate::TICKS_PER_SEC;
use crate::fixed::{Fx, fx, mul, pct, to_f64};

use super::chain::{Chain, Kind, Node, StockSpec};
use super::model::{NodeState, State, Status};

/// Run regrowth and harvest for every source, in processing order.
pub fn tick_all(chain: &Chain, eff: &[i64], st: &mut State) {
    let water = eff[chain.g.water];
    let r = eff[chain.g.regrowth];
    let dim = eff[chain.g.diminishing];
    for &ni in &chain.order {
        let node = &chain.nodes[ni];
        let Kind::Source(spec) = &node.kind else {
            continue;
        };
        // Always draw, even if unused, so other sources' sequences don't shift.
        let u = st.rng.yields.signed();
        let ns = &mut st.nodes[ni];
        regrow(&spec.stock, ns, water, r);
        match super::crew_gate(node, eff, dim) {
            Ok(crew) => harvest(node, &spec.stock, eff, ns, crew, u),
            Err(s) => ns.status = s,
        }
    }
}

/// Logistic regrowth of a living stock.
pub fn regrow(stock: &StockSpec, ns: &mut NodeState, water: Fx, regrowth: Fx) {
    let StockSpec::Regrowing {
        max, water_need, ..
    } = *stock
    else {
        return;
    };
    if max <= 0 {
        return;
    }
    let s = to_f64(ns.stock);
    let k = to_f64(max);
    let water_factor = if water_need <= 0 {
        1.0
    } else {
        (to_f64(water) / to_f64(water_need)).clamp(0.0, 1.0)
    };
    let r = to_f64(regrowth) / TICKS_PER_SEC as f64;
    let seedlings = 0.01 * k;
    let growth = r * water_factor * s.max(seedlings) * (1.0 - s / k);
    ns.stock = (ns.stock + fx(growth.max(0.0))).min(max);
}

/// Water factor for display: how much of full regrowth speed the water
/// supply allows (fixed point, ONE = 100%).
pub fn water_factor(stock: &StockSpec, water: Fx) -> Option<Fx> {
    match *stock {
        StockSpec::Regrowing { water_need, .. } if water_need > 0 => {
            Some(crate::fixed::div(water, water_need).min(crate::fixed::ONE))
        }
        StockSpec::Regrowing { .. } => Some(crate::fixed::ONE),
        _ => None,
    }
}

fn harvest(node: &Node, stock: &StockSpec, eff: &[i64], ns: &mut NodeState, crew: Fx, u: f64) {
    let cap = node.p.out_cap.map_or(0, |p| eff[p]);
    let space = cap - ns.outputs[0];
    if space <= 0 {
        ns.status = Status::Blocked;
        return;
    }
    let units = node.p.units.map_or(1, |p| eff[p]);
    let rate_per_tick = node.p.rate.map_or(0, |p| eff[p]) * units / TICKS_PER_SEC as i64;
    let mut amt = pct(mul(rate_per_tick, crew), eff[node.p.efficiency]);
    let variance = node.p.variance.map_or(0, |p| eff[p]);
    if variance > 0 {
        amt = (amt as f64 * (1.0 + variance as f64 / 100.0 * u)).round() as Fx;
    }
    amt = amt.max(0);

    let limited = !matches!(stock, StockSpec::Infinite);
    if limited {
        if ns.stock <= 0 {
            ns.status = Status::Starved;
            return;
        }
        amt = amt.min(ns.stock);
    }
    amt = amt.min(space);
    ns.outputs[0] += amt;
    if limited {
        ns.stock -= amt;
    }
    ns.out_tick = amt;
    ns.status = Status::Working;
}
