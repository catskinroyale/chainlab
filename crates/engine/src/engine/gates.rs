//! # Gates and unlocks
//!
//! Gates are milestones passed in order. Each one raises the **tier** by
//! one, which brings online nodes and upgrades tagged with that tier. It's
//! the "new content" hook of production games: a reason to push a metric
//! you might otherwise ignore, and a structural change (a new node or
//! recipe) that shifts where the bottleneck sits.
//!
//! The UI shows the next gate's progress and a time estimate based on how
//! fast the gate's metric is currently growing.

use crate::fixed::{Fx, to_f64};

use super::chain::{Chain, Cond};
use super::model::{LogKind, State};

/// Current value of a gate's metric.
pub fn metric(st: &State, cond: &Cond) -> Fx {
    match *cond {
        Cond::Progress(_) => st.progress,
        Cond::Coin(_) => st.coin,
        Cond::Profit(_) => st.ledger.operating_profit(),
        Cond::Produced(n, _) => st.stats.nodes[n].out_total,
    }
}

pub fn target(cond: &Cond) -> Fx {
    match *cond {
        Cond::Progress(t) | Cond::Coin(t) | Cond::Profit(t) | Cond::Produced(_, t) => t,
    }
}

/// Rate of the metric per minute, from rolling stats.
pub fn metric_rate(st: &State, cond: &Cond) -> Fx {
    match *cond {
        Cond::Progress(_) => st.stats.progress_per_min(),
        Cond::Coin(_) => st.stats.net_per_min(),
        Cond::Profit(_) => st.stats.operating_per_min(),
        Cond::Produced(n, _) => st.stats.nodes[n].per_min(),
    }
}

/// Check gates in order. Returns true if the tier changed.
pub fn tick(chain: &Chain, st: &mut State) -> bool {
    let mut changed = false;
    while let Some(g) = chain.gates.get(st.tier as usize) {
        if metric(st, &g.cond) < target(&g.cond) {
            break;
        }
        st.tier += 1;
        let text = if g.description.is_empty() {
            format!("UNLOCKED tier {}: {}", st.tier, g.name)
        } else {
            format!("UNLOCKED tier {}: {} ({})", st.tier, g.name, g.description)
        };
        st.log(LogKind::Unlock, text);
        changed = true;
    }
    changed
}

/// Next gate: (index, completion 0..1, estimated seconds if growing).
pub fn next(chain: &Chain, st: &State) -> Option<(usize, f64, Option<f64>)> {
    let i = st.tier as usize;
    let g = chain.gates.get(i)?;
    let t = target(&g.cond);
    let v = metric(st, &g.cond);
    let frac = if t <= 0 {
        1.0
    } else {
        (to_f64(v) / to_f64(t)).clamp(0.0, 1.0)
    };
    let rate = to_f64(metric_rate(st, &g.cond));
    let eta = (rate > 0.0).then(|| (to_f64(t - v) / rate * 60.0).max(0.0));
    Some((i, frac, eta))
}
