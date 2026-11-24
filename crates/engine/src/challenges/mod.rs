//! # Challenges
//!
//! Optional goals that score how well you've tuned the system. Nothing ends
//! the simulation: a challenge just watches the sim, shows progress, and
//! reports a pass (with a 0-100 score) or a fail. Retry restarts the
//! challenge's scenario instantly.
//!
//! Challenges lock the "free capability" levers (rates, cycle times,
//! prices, carriers...) so the only ways to win are the ones a player of a
//! real production game has: **allocating workers, switching nodes,
//! sizing buffers, choosing a sell policy, and buying upgrades**.
//! *Find the Bottleneck* is the exception: you get the node and link levers
//! so you can find and repair the hidden fault.
//!
//! Driving a challenge:
//!
//! ```no_run
//! # use chainlab_engine::challenges::{ChallengeId, ChallengeRun};
//! let (mut sim, mut run) = ChallengeRun::start(ChallengeId::SteadyState, 1).unwrap();
//! while run.is_running() && sim.state.tick < 36_000 {
//!     run.before_step(&mut sim);
//!     sim.step();
//!     run.after_step(&sim);
//! }
//! ```

use crate::TICKS_PER_SEC;
use crate::engine::chain::Kind;
use crate::engine::model::{EventKind, LogKind, Status};
use crate::engine::{Command, Sim};
use crate::fixed::{Fx, fx, to_f64};
use crate::rng::Rng;
use crate::scenarios;

/// Market Shock: when the crash hits (seconds), after the chain is steady.
pub const SHOCK_AT: u64 = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChallengeId {
    Sandbox,
    SteadyState,
    FindBottleneck,
    LeanChain,
    MarketShock,
    CostWall,
}

impl ChallengeId {
    pub const ALL: [ChallengeId; 6] = [
        ChallengeId::Sandbox,
        ChallengeId::SteadyState,
        ChallengeId::FindBottleneck,
        ChallengeId::LeanChain,
        ChallengeId::MarketShock,
        ChallengeId::CostWall,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ChallengeId::Sandbox => "Sandbox",
            ChallengeId::SteadyState => "Steady State",
            ChallengeId::FindBottleneck => "Find the Bottleneck",
            ChallengeId::LeanChain => "Lean Chain",
            ChallengeId::MarketShock => "Market Shock",
            ChallengeId::CostWall => "Cost Wall",
        }
    }

    pub fn goal(self) -> &'static str {
        match self {
            ChallengeId::Sandbox => "No goals. Every lever unlocked.",
            ChallengeId::SteadyState => {
                "Hold operating profit >= 35/min with no production node starved or blocked more \
                 than 10% of the time, for 2 simulated minutes. Score: speed."
            }
            ChallengeId::FindBottleneck => {
                "One parameter has been secretly mis-tuned and the bottleneck marker is hidden. \
                 Diagnose it and get Tower progress back to >= 7.0/min for 45 s (counted from 3:00). Score: speed."
            }
            ChallengeId::LeanChain => {
                "Reach Tower progress >= 7.0/min, held for 60 s, using as few workers as \
                 possible. Starts with 300 coin for upgrades. Score: workers used."
            }
            ChallengeId::MarketShock => {
                "At 3:00 prices crash and base prices drop 40% for good. Get operating profit \
                 back to 80% of its pre-crash level within 5 minutes. Score: speed."
            }
            ChallengeId::CostWall => {
                "Upgrade costs grow x1.5 per level. Own 15 upgrade levels within 15 minutes \
                 without going into debt for more than 10 s. Score: speed."
            }
        }
    }

    /// Time limit in ticks, if any.
    pub fn limit(self) -> Option<u64> {
        let m = 60 * TICKS_PER_SEC;
        match self {
            ChallengeId::Sandbox => None,
            ChallengeId::SteadyState => Some(20 * m),
            ChallengeId::FindBottleneck => Some(10 * m),
            ChallengeId::LeanChain => Some(15 * m),
            ChallengeId::MarketShock => Some(SHOCK_AT * TICKS_PER_SEC + 5 * m),
            ChallengeId::CostWall => Some(15 * m),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Running,
    Passed {
        tick: u64,
        score: u32,
        detail: String,
    },
    Failed {
        tick: u64,
        reason: String,
    },
}

#[derive(Clone, Debug)]
pub struct ChallengeRun {
    pub id: ChallengeId,
    pub outcome: Outcome,
    /// 0..1 towards the goal.
    pub progress: f64,
    /// One-line live status.
    pub status: String,
    /// Seconds the goal condition has been held.
    held: u64,
    debt_secs: u64,
    /// Find the Bottleneck: what was broken (revealed at the end).
    pub hidden: Option<String>,
    /// Market Shock: operating profit/min just before the crash.
    pre_shock: Option<Fx>,
    seed: u64,
}

/// Hidden faults for Find the Bottleneck: (key, value, explanation).
const FAULTS: &[(&str, f64, &str)] = &[
    (
        "node.sawmill.efficiency",
        30.0,
        "Sawmill efficiency was 30% (wasting logs)",
    ),
    (
        "link.1.capacity",
        0.5,
        "Forest haul carts carried only 0.5 logs each",
    ),
    (
        "link.2.distance",
        30.0,
        "Brick haul to the Tower was 30 s each way",
    ),
    ("node.kiln.efficiency", 50.0, "Kiln efficiency was 50%"),
    (
        "node.tower.workers",
        1.0,
        "Tower Builder was half-crewed (1 of 2)",
    ),
];

impl ChallengeRun {
    /// Build the challenge's sim and its tracker.
    pub fn start(id: ChallengeId, seed: u64) -> Result<(Sim, ChallengeRun), String> {
        let loaded = scenarios::load("balanced")?;
        let mut sim = loaded.build(Some(seed))?;
        let mut run = ChallengeRun {
            id,
            outcome: Outcome::Running,
            progress: 0.0,
            status: String::new(),
            held: 0,
            debt_secs: 0,
            hidden: None,
            pre_shock: None,
            seed,
        };
        let mut set = |k: &str, v: f64| sim.params.set_human(k, v);
        match id {
            ChallengeId::FindBottleneck => {
                let mut r = Rng::new(seed, 99);
                let (k, v, why) = FAULTS[(r.next_u64() % FAULTS.len() as u64) as usize];
                set(k, v)?;
                run.hidden = Some(why.to_string());
            }
            ChallengeId::LeanChain => {
                sim.state.coin = fx(300.0);
            }
            ChallengeId::CostWall => {
                for u in sim.chain.upgrades.clone() {
                    sim.params.list[u.growth].set(fx(1.5));
                }
                sim.state.coin = fx(150.0);
            }
            _ => {}
        }
        // Baked so a hidden fault doesn't show a "changed" marker.
        sim.params.bake_defaults();
        sim.command(Command::ResetAll);
        sim.state.log.clear();
        sim.state
            .log(LogKind::Challenge, format!("CHALLENGE: {}", id.name()));
        run.status = id.goal().to_string();
        Ok((sim, run))
    }

    /// A tracker for free play (no goal, nothing locked).
    pub fn sandbox(seed: u64) -> ChallengeRun {
        ChallengeRun {
            id: ChallengeId::Sandbox,
            outcome: Outcome::Running,
            progress: 0.0,
            status: ChallengeId::Sandbox.goal().to_string(),
            held: 0,
            debt_secs: 0,
            hidden: None,
            pre_shock: None,
            seed,
        }
    }

    pub fn is_running(&self) -> bool {
        self.outcome == Outcome::Running
    }

    /// The seed this run was started with (for retry).
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Should the UI hide the bottleneck marker?
    pub fn hide_bottleneck(&self) -> bool {
        self.id == ChallengeId::FindBottleneck && self.is_running()
    }

    /// May the player change this lever during the challenge?
    pub fn lever_allowed(&self, key: &str) -> bool {
        if self.id == ChallengeId::Sandbox || !self.is_running() {
            return true;
        }
        let allocation = key.ends_with(".workers")
            || key.ends_with(".enabled")
            || key.ends_with(".in_cap")
            || key.ends_with(".out_cap")
            || matches!(key, "market.policy" | "market.threshold" | "market.reserve");
        match self.id {
            ChallengeId::FindBottleneck => {
                allocation || key.starts_with("node.") || key.starts_with("link.")
            }
            _ => allocation,
        }
    }

    /// Scripted commands before a tick.
    pub fn before_step(&mut self, sim: &mut Sim) {
        if self.id != ChallengeId::MarketShock || !self.is_running() {
            return;
        }
        let crash_at = SHOCK_AT * TICKS_PER_SEC;
        if sim.state.tick == crash_at {
            self.pre_shock = Some(sim.state.stats.operating_per_min());
            if let Some(m) = sim.chain.market.clone() {
                let n = m.goods.len();
                for (k, g) in m.goods.iter().enumerate() {
                    sim.command(Command::Trigger {
                        kind: EventKind::PriceCrash,
                        severity: 90,
                        pick: (k as f64 + 0.5) / n as f64,
                    });
                    let base = sim.params.get(g.base_price);
                    sim.params.list[g.base_price].set(base * 6 / 10);
                    sim.params.list[g.base_price].default = base * 6 / 10;
                }
                sim.refresh_eff();
                sim.state
                    .log(LogKind::Challenge, "SHOCK: base prices -40% permanently");
            }
        }
    }

    /// Evaluate after a tick (once per simulated second).
    pub fn after_step(&mut self, sim: &Sim) {
        if !self.is_running() || self.id == ChallengeId::Sandbox {
            return;
        }
        let tick = sim.state.tick;
        if tick % TICKS_PER_SEC != 0 {
            return;
        }
        let secs = tick / TICKS_PER_SEC;
        let st = &sim.state;
        let s = &st.stats;
        match self.id {
            ChallengeId::Sandbox => {}
            ChallengeId::SteadyState => {
                let profit = s.operating_per_min();
                let target = fx(35.0);
                let mut worst = (0.0, String::from("-"));
                for (i, n) in sim.chain.nodes.iter().enumerate() {
                    if matches!(n.kind, Kind::Market) || sim.is_locked(i) {
                        continue;
                    }
                    let ns = &s.nodes[i];
                    let (sv, bv) = (ns.share(Status::Starved), ns.share(Status::Blocked));
                    let (v, what) = if sv >= bv {
                        (sv, "starved")
                    } else {
                        (bv, "blocked")
                    };
                    if v > worst.0 {
                        worst = (v, format!("{} {what} {:.0}%", n.name, v * 100.0));
                    }
                }
                let ok = profit >= target && worst.0 <= 0.10 && secs >= 60;
                self.held = if ok { self.held + 1 } else { 0 };
                self.progress = self.held as f64 / 120.0;
                self.status = format!(
                    "held {}/120 s | profit {:.1}/35 per min | worst: {}",
                    self.held,
                    to_f64(profit),
                    worst.1
                );
                if self.held >= 120 {
                    let score = time_score(secs, 240, 1200);
                    self.pass(sim, score, format!("steady after {}", mmss(secs)));
                }
            }
            ChallengeId::FindBottleneck => {
                // Warm-up: early stockpiles can carry the Tower for a while,
                // so the hold only counts once buffers have settled.
                let rate = s.progress_per_min();
                let ok = rate >= fx(7.0) && secs >= 180;
                self.held = if ok { self.held + 1 } else { 0 };
                self.progress = (to_f64(rate) / 7.0).min(1.0) * 0.6 + self.held as f64 / 45.0 * 0.4;
                self.status = format!(
                    "progress {:.1}/7.0 per min | held {}/45 s{}",
                    to_f64(rate),
                    self.held,
                    if secs < 180 {
                        format!(" | counts from 3:00 ({})", mmss(secs))
                    } else {
                        String::new()
                    }
                );
                if self.held >= 45 {
                    let score = time_score(secs, 240, 600);
                    let why = self.hidden.clone().unwrap_or_default();
                    self.pass(sim, score, format!("fixed in {}: {why}", mmss(secs)));
                }
            }
            ChallengeId::LeanChain => {
                let rate = s.progress_per_min();
                let workers = sim.workers_assigned();
                let ok = rate >= fx(7.0) && secs >= 60;
                self.held = if ok { self.held + 1 } else { 0 };
                self.progress = (to_f64(rate) / 7.0).min(1.0) * 0.5 + self.held as f64 / 60.0 * 0.5;
                self.status = format!(
                    "progress {:.1}/7.0 per min | held {}/60 s | workers placed {workers}",
                    to_f64(rate),
                    self.held
                );
                if self.held >= 60 {
                    let score = (100 - (workers - 6).max(0) * 12).clamp(5, 100) as u32;
                    self.pass(sim, score, format!("{workers} workers"));
                }
            }
            ChallengeId::MarketShock => {
                let crash: u64 = SHOCK_AT;
                let profit = s.operating_per_min();
                match self.pre_shock {
                    None => {
                        self.progress = secs as f64 / crash as f64 * 0.2;
                        self.status = format!(
                            "crash in {} s | profit now {:.1}/min",
                            crash.saturating_sub(secs),
                            to_f64(profit)
                        );
                    }
                    Some(pre) => {
                        let target = pre * 8 / 10;
                        let since = secs.saturating_sub(crash);
                        // Must wait for the window to fill with post-crash data.
                        let ok = since >= 60 && profit >= target;
                        self.progress = 0.2
                            + 0.8 * (to_f64(profit).max(0.0) / to_f64(target).max(0.01)).min(1.0);
                        self.status = format!(
                            "{} since crash | profit {:.1} / target {:.1} per min",
                            mmss(since),
                            to_f64(profit),
                            to_f64(target)
                        );
                        if ok {
                            let score = time_score(since, 60, 300);
                            self.pass(sim, score, format!("recovered {} after crash", mmss(since)));
                        }
                    }
                }
            }
            ChallengeId::CostWall => {
                let owned: u32 = st.upgrades.iter().sum();
                self.debt_secs = if st.coin < 0 { self.debt_secs + 1 } else { 0 };
                self.progress = owned as f64 / 15.0;
                self.status = format!(
                    "upgrades {owned}/15 | coin {:.0} | net {:.1}/min{}",
                    to_f64(st.coin),
                    to_f64(s.operating_per_min()),
                    if self.debt_secs > 0 {
                        format!(" | IN DEBT {}s/10", self.debt_secs)
                    } else {
                        String::new()
                    }
                );
                if self.debt_secs > 10 {
                    self.fail(sim, "cash flow collapsed: in debt for over 10 s".into());
                    return;
                }
                if owned >= 15 {
                    let score = time_score(secs, 300, 900);
                    self.pass(sim, score, format!("15 upgrades in {}", mmss(secs)));
                }
            }
        }
        if self.is_running()
            && let Some(limit) = self.id.limit()
            && tick >= limit
        {
            self.fail(
                sim,
                format!("time limit {} reached", mmss(limit / TICKS_PER_SEC)),
            );
        }
    }

    fn pass(&mut self, sim: &Sim, score: u32, detail: String) {
        self.progress = 1.0;
        self.status = format!("PASSED, score {score}/100: {detail}");
        self.outcome = Outcome::Passed {
            tick: sim.state.tick,
            score,
            detail,
        };
    }

    fn fail(&mut self, sim: &Sim, reason: String) {
        let extra = self
            .hidden
            .as_ref()
            .map(|h| format!(" (the fault: {h})"))
            .unwrap_or_default();
        self.status = format!("FAILED: {reason}{extra}");
        self.outcome = Outcome::Failed {
            tick: sim.state.tick,
            reason,
        };
    }
}

/// 100 at or below `best` seconds, falling linearly to 10 at `worst`.
fn time_score(secs: u64, best: u64, worst: u64) -> u32 {
    if secs <= best {
        100
    } else if secs >= worst {
        10
    } else {
        (100.0 - 90.0 * (secs - best) as f64 / (worst - best) as f64).round() as u32
    }
}

pub fn mmss(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_score_shape() {
        assert_eq!(time_score(10, 60, 600), 100);
        assert_eq!(time_score(600, 60, 600), 10);
        assert_eq!(time_score(330, 60, 600), 55);
    }
}
