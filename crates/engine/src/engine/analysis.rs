//! # Counterfactual analysis: payback times and A/B runs
//!
//! Because the engine is deterministic and cheap to clone, we can answer
//! "what if?" by *actually simulating it*: fork the current sim, apply the
//! upgrade for free, fast-forward a couple of minutes, and compare with a
//! fork that did nothing. Both forks share the same random sequence, so the
//! difference is purely the upgrade's effect (the "common random numbers"
//! trick from simulation studies).
//!
//! ```text
//! marginal profit/min = (profit with upgrade - profit without) / horizon
//! payback time        = cost / marginal profit
//! ```
//!
//! An upgrade that speeds up a non-bottleneck node shows ~0 marginal gain,
//! and that's the lesson.

use std::collections::BTreeMap;

use crate::config::ChainConfig;
use crate::fixed::{Fx, to_f64};

use super::Sim;
use super::model::Status;
use super::scoring;
use super::scoring::Limiter;

#[derive(Clone, Debug, PartialEq)]
pub struct Payback {
    pub upgrade: usize,
    pub cost: Fx,
    /// Change in operating profit per minute.
    pub d_profit: Fx,
    /// Change in progress per minute.
    pub d_progress: Fx,
    /// Seconds to earn back the cost, if it pays back at all.
    pub payback_secs: Option<f64>,
}

/// Default analysis horizon: two simulated minutes.
pub const HORIZON: u64 = 1_200;

/// Evaluate every currently purchasable-in-principle upgrade.
pub fn payback(sim: &Sim, horizon: u64) -> Vec<Payback> {
    let horizon = horizon.max(1);
    let measure = |s: &mut Sim| {
        let p0 = s.state.ledger.operating_profit();
        let g0 = s.state.progress;
        s.run(horizon);
        (
            s.state.ledger.operating_profit() - p0,
            s.state.progress - g0,
        )
    };
    let mut base = sim.clone();
    let (bp, bg) = measure(&mut base);
    let per_min = |x: Fx| x * 600 / horizon as i64;
    let mut out = vec![];
    for u in 0..sim.chain.upgrades.len() {
        let up = &sim.chain.upgrades[u];
        if up.tier > sim.state.tier || up.max_level.is_some_and(|m| sim.state.upgrades[u] >= m) {
            continue;
        }
        let cost = sim.upgrade_cost(u);
        let mut fork = sim.clone();
        fork.grant(u);
        let (fp, fg) = measure(&mut fork);
        let d_profit = per_min(fp - bp);
        let d_progress = per_min(fg - bg);
        let payback_secs = (d_profit > 0).then(|| to_f64(cost) / (to_f64(d_profit) / 60.0));
        out.push(Payback {
            upgrade: u,
            cost,
            d_profit,
            d_progress,
            payback_secs,
        });
    }
    out
}

/// Key numbers after a run, for A/B comparison and headless output.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSummary {
    pub label: String,
    pub ticks: u64,
    pub progress: Fx,
    pub progress_per_min: Fx,
    pub income_per_min: Fx,
    pub operating_per_min: Fx,
    pub coin: Fx,
    pub efficiency: u32,
    pub utilisation: f64,
    pub workers: i64,
    pub bottleneck: String,
    pub starved_pct: f64,
    pub blocked_pct: f64,
}

pub fn summarize(sim: &Sim, label: &str) -> RunSummary {
    let st = &sim.state;
    let c = &*sim.chain;
    let eff = sim.eff_all();
    let e = scoring::efficiency(c, eff, st);
    let b = scoring::bottleneck(c, eff, st)
        .map(|b| match b.at {
            Limiter::Node(n) => c.nodes[n].name.clone(),
            Limiter::Link(l) => format!("haul {}", c.link_label(l)),
        })
        .unwrap_or_else(|| "none (balanced)".into());
    let n = c.nodes.len().max(1) as f64;
    let mean = |s: Status| st.stats.nodes.iter().map(|x| x.share(s)).sum::<f64>() / n;
    RunSummary {
        label: label.to_string(),
        ticks: st.tick,
        progress: st.progress,
        progress_per_min: st.stats.progress_per_min(),
        income_per_min: st.stats.income_per_min(),
        operating_per_min: st.stats.operating_per_min(),
        coin: st.coin,
        efficiency: e.score,
        utilisation: e.utilisation,
        workers: sim.workers_assigned(),
        bottleneck: b,
        starved_pct: mean(Status::Starved) * 100.0,
        blocked_pct: mean(Status::Blocked) * 100.0,
    }
}

/// Run a parameter set headless from a fresh start.
pub fn run_headless(
    cfg: &ChainConfig,
    values: &BTreeMap<String, f64>,
    seed: u64,
    ticks: u64,
    label: &str,
) -> Result<RunSummary, String> {
    let mut sim = Sim::new(cfg, seed)?;
    sim.apply_values(values, false);
    sim.run(ticks);
    Ok(summarize(&sim, label))
}

/// Format two summaries side by side.
pub fn compare_table(a: &RunSummary, b: &RunSummary) -> Vec<(String, String, String, i8)> {
    let f1 = |x: Fx| format!("{:.1}", to_f64(x));
    // Last field: +1 if B is better, -1 if A is better, 0 if equal/neutral.
    let better = |x: f64, y: f64, higher_good: bool| -> i8 {
        if (x - y).abs() < 1e-9 {
            0
        } else if (y > x) == higher_good {
            1
        } else {
            -1
        }
    };
    vec![
        (
            "Progress/min".into(),
            f1(a.progress_per_min),
            f1(b.progress_per_min),
            better(to_f64(a.progress_per_min), to_f64(b.progress_per_min), true),
        ),
        (
            "Progress total".into(),
            f1(a.progress),
            f1(b.progress),
            better(to_f64(a.progress), to_f64(b.progress), true),
        ),
        (
            "Income/min".into(),
            f1(a.income_per_min),
            f1(b.income_per_min),
            better(to_f64(a.income_per_min), to_f64(b.income_per_min), true),
        ),
        (
            "Op. profit/min".into(),
            f1(a.operating_per_min),
            f1(b.operating_per_min),
            better(
                to_f64(a.operating_per_min),
                to_f64(b.operating_per_min),
                true,
            ),
        ),
        (
            "Coin".into(),
            f1(a.coin),
            f1(b.coin),
            better(to_f64(a.coin), to_f64(b.coin), true),
        ),
        (
            "Efficiency".into(),
            a.efficiency.to_string(),
            b.efficiency.to_string(),
            better(a.efficiency as f64, b.efficiency as f64, true),
        ),
        (
            "Workers used".into(),
            a.workers.to_string(),
            b.workers.to_string(),
            better(a.workers as f64, b.workers as f64, false),
        ),
        (
            "Avg starved %".into(),
            format!("{:.0}", a.starved_pct),
            format!("{:.0}", b.starved_pct),
            better(a.starved_pct, b.starved_pct, false),
        ),
        (
            "Avg blocked %".into(),
            format!("{:.0}", a.blocked_pct),
            format!("{:.0}", b.blocked_pct),
            better(a.blocked_pct, b.blocked_pct, false),
        ),
        (
            "Bottleneck".into(),
            a.bottleneck.clone(),
            b.bottleneck.clone(),
            0,
        ),
    ]
}
