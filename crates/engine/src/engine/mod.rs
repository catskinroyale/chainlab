//! The simulation: [`Sim`] ties together the static [`chain::Chain`], the
//! live [`params::ParamSet`] and the dynamic [`model::State`].
//!
//! ## Tick order
//!
//! One tick = 1/10 simulated second. Every tick runs the same phases in the
//! same order, and inside each phase nodes and links go in a fixed order:
//!
//! 1. **Events**: expire old ones, maybe start a new one.
//! 2. **Sources**: regrow living stock, then harvest into output buffers.
//! 3. **Logistics**: unload arrived carriers, free returned ones, dispatch.
//! 4. **Processors**: advance recipe cycles (upstream nodes first).
//! 5. **Sinks**: consume input sets into Progress.
//! 6. **Market**: update prices, sell per policy.
//! 7. **Upkeep**: pay the worker pool.
//! 8. **Gates**: check milestones, unlock tiers.
//! 9. **Stats**: record into rolling windows and history.
//!
//! ## Effective parameters
//!
//! The tick never reads raw lever values. It reads `eff`, rebuilt whenever
//! something changes:
//!
//! ```text
//! eff = lever value
//!       + upgrade Adds, then * upgrade Muls      (economy)
//!       -> nodes above the current tier forced off (gates)
//!       -> drought / breakdown modifiers         (events)
//!       -> workers clipped to the pool, upstream nodes first (workers)
//! ```
//!
//! Commands (lever changes, purchases) are applied *between* ticks via
//! [`Sim::command`], so a run is fully reproducible from
//! `(config, seed, [(tick, command)...])`.

pub mod analysis;
pub mod chain;
pub mod economy;
pub mod events;
pub mod gates;
pub mod logistics;
pub mod market;
pub mod model;
pub mod params;
pub mod processor;
pub mod report;
pub mod scoring;
pub mod sink;
pub mod source;
pub mod stats;
pub mod workers;

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::config::ChainConfig;
use crate::fixed::{Fx, pct};
use crate::rng::Rngs;

use chain::{Chain, Effect, Node};
use economy::BuyBlock;
use model::{EventKind, LogKind, State, Status};
use params::{ParamId, ParamSet};

/// Simulated ticks per simulated second.
pub const TICKS_PER_SEC: u64 = 10;

/// A change requested by the player (or a script). Applied between ticks.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Set a parameter to a raw value (clamped to its range).
    Set(ParamId, i64),
    /// Nudge a parameter by `steps` fine or coarse steps.
    Adjust {
        id: ParamId,
        steps: i64,
        coarse: bool,
    },
    /// Reset one parameter to its default.
    Reset(ParamId),
    /// Reset every parameter to its default.
    ResetAll,
    /// Re-seed all random streams from this tick on.
    Reseed(u64),
    /// Buy one level of an upgrade.
    Buy(usize),
    /// Force an event (scripts and challenges). `pick` (0..1) picks the target.
    Trigger {
        kind: EventKind,
        severity: i64,
        pick: f64,
    },
}

#[derive(Clone, Debug)]
pub struct Sim {
    pub chain: Arc<Chain>,
    pub params: ParamSet,
    pub state: State,
    pub seed: u64,
    eff: Vec<i64>,
    /// For coalescing repeated lever nudges into one log line.
    last_lever: Option<(ParamId, i64, u64)>,
}

impl Sim {
    pub fn new(cfg: &ChainConfig, seed: u64) -> Result<Self, String> {
        let (chain, params) = chain::compile(cfg)?;
        let mut sim = Sim {
            chain: Arc::new(chain),
            params,
            state: State::new(&Chain::empty(), seed, &[]),
            seed,
            eff: vec![],
            last_lever: None,
        };
        sim.restart();
        Ok(sim)
    }

    /// Fresh state (tick 0) keeping the current lever values and seed.
    pub fn restart(&mut self) {
        let prices: Vec<Fx> = self
            .chain
            .market
            .as_ref()
            .map(|m| {
                m.goods
                    .iter()
                    .map(|g| self.params.get(g.base_price))
                    .collect()
            })
            .unwrap_or_default();
        self.state = State::new(&self.chain, self.seed, &prices);
        self.last_lever = None;
        self.refresh_eff();
        self.state.log(
            LogKind::Info,
            format!("Start: {} (seed {})", self.chain.name, self.seed),
        );
    }

    /// Effective value of a parameter.
    pub fn eff(&self, id: ParamId) -> i64 {
        self.eff[id]
    }

    pub fn eff_all(&self) -> &[i64] {
        &self.eff
    }

    /// Apply a command immediately (between ticks).
    pub fn command(&mut self, c: Command) {
        let n = self.params.len();
        match c {
            Command::Set(id, v) if id < n => {
                let before = self.params.list[id].value;
                self.params.list[id].set(v);
                self.log_lever(id, before);
            }
            Command::Adjust { id, steps, coarse } if id < n => {
                let p = &mut self.params.list[id];
                let before = p.value;
                let step = if coarse { p.coarse } else { p.fine };
                p.set(p.value + steps * step);
                self.log_lever(id, before);
            }
            Command::Reset(id) if id < n => {
                let before = self.params.list[id].value;
                let p = &mut self.params.list[id];
                p.value = p.default;
                self.log_lever(id, before);
            }
            Command::ResetAll => {
                for p in &mut self.params.list {
                    p.value = p.default;
                }
                self.state
                    .log(LogKind::Lever, "All levers reset to defaults");
            }
            Command::Reseed(s) => {
                self.seed = s;
                self.state.rng = Rngs::new(s);
                self.state.log(LogKind::Lever, format!("Reseeded: {s}"));
            }
            Command::Buy(u) if u < self.chain.upgrades.len() => {
                let _ = self.buy(u);
            }
            Command::Trigger {
                kind,
                severity,
                pick,
            } => {
                events::spawn(&self.chain, &mut self.state, kind, severity, pick, 0.5);
            }
            _ => {}
        }
        self.refresh_eff();
    }

    fn log_lever(&mut self, id: ParamId, before: i64) {
        let p = &self.params.list[id];
        let now = self.state.tick;
        let from = match self.last_lever {
            Some((lid, from, t)) if lid == id && now.saturating_sub(t) < 30 => {
                if self
                    .state
                    .log
                    .back()
                    .is_some_and(|e| e.kind == LogKind::Lever)
                {
                    self.state.log.pop_back();
                }
                from
            }
            _ => before,
        };
        if from == p.value {
            self.last_lever = None;
            return;
        }
        let text = format!("{}: {} -> {}", p.label, p.unit.format(from), p.display());
        self.last_lever = Some((id, from, now));
        self.state.log(LogKind::Lever, text);
    }

    /// Try to buy one level of upgrade `u`.
    pub fn buy(&mut self, u: usize) -> Result<Fx, BuyBlock> {
        match economy::check_buy(&self.chain, &self.eff, &self.state, u) {
            Ok(cost) => {
                self.state.coin -= cost;
                self.state.ledger.purchase_tick += cost;
                self.state.ledger.purchase_total += cost;
                self.state.upgrades[u] += 1;
                let text = format!(
                    "BUY {} (lvl {}) for {:.1}c",
                    self.chain.upgrades[u].name,
                    self.state.upgrades[u],
                    crate::fixed::to_f64(cost)
                );
                self.state.log(LogKind::Purchase, text);
                self.refresh_eff();
                Ok(cost)
            }
            Err(e) => Err(e),
        }
    }

    /// Give an upgrade level for free (analysis forks only).
    pub fn grant(&mut self, u: usize) {
        self.state.upgrades[u] += 1;
        self.refresh_eff();
    }

    pub fn upgrade_cost(&self, u: usize) -> Fx {
        economy::upgrade_cost(&self.chain, &self.eff, &self.state, u)
    }

    pub fn check_buy(&self, u: usize) -> Result<Fx, BuyBlock> {
        economy::check_buy(&self.chain, &self.eff, &self.state, u)
    }

    /// Apply saved lever values (human units, by key). With `as_default`
    /// they also become the defaults (scenario presets). Unknown keys are
    /// ignored and returned.
    pub fn apply_values(
        &mut self,
        values: &BTreeMap<String, f64>,
        as_default: bool,
    ) -> Vec<String> {
        let mut unknown = vec![];
        for (k, v) in values {
            if self.params.set_human(k, *v).is_err() {
                unknown.push(k.clone());
            }
        }
        if as_default {
            self.params.bake_defaults();
        }
        // Market prices start at the (new) base price.
        if self.state.tick == 0 {
            self.restart();
        } else {
            self.refresh_eff();
        }
        unknown
    }

    /// Rebuild effective values (after editing params or state directly).
    pub fn refresh_eff(&mut self) {
        self.eff = compute_eff(&self.chain, &self.params, &self.state);
    }

    /// Advance one tick.
    pub fn step(&mut self) {
        for ns in self.state.nodes.iter_mut() {
            ns.out_tick = 0;
        }
        let coin_before = self.state.coin;
        if events::tick(&self.chain, &self.eff, &mut self.state) {
            self.refresh_eff();
        }
        {
            let chain = &*self.chain;
            let eff = &self.eff;
            let st = &mut self.state;
            source::tick_all(chain, eff, st);
            logistics::tick(chain, eff, st);
            processor::tick_all(chain, eff, st);
            sink::tick_all(chain, eff, st);
            market::tick(chain, eff, st);
            economy::upkeep(chain, eff, st);
        }
        if gates::tick(&self.chain, &mut self.state) {
            self.refresh_eff();
        }
        if coin_before >= 0 && self.state.coin < 0 {
            self.state.log(LogKind::Warn, "IN DEBT: coin below zero");
        }
        stats::record(&self.chain, &self.eff, &mut self.state);
        let l = &mut self.state.ledger;
        l.income_tick = 0;
        l.upkeep_tick = 0;
        l.purchase_tick = 0;
        self.state.tick += 1;
    }

    /// Advance `n` ticks.
    pub fn run(&mut self, n: u64) {
        for _ in 0..n {
            self.step();
        }
    }

    /// Simulated seconds elapsed.
    pub fn time_secs(&self) -> f64 {
        self.state.tick as f64 / TICKS_PER_SEC as f64
    }

    /// Effective workers on a node (0 if it needs none).
    pub fn workers(&self, node: &Node) -> i64 {
        node.p.workers.map_or(0, |p| self.eff[p])
    }

    /// Workers actually placed on nodes (after pool clipping).
    pub fn workers_assigned(&self) -> i64 {
        self.chain.nodes.iter().map(|n| self.workers(n)).sum()
    }

    /// Workers requested by the levers (may exceed the pool).
    pub fn workers_requested(&self) -> i64 {
        self.chain
            .nodes
            .iter()
            .filter(|n| n.tier <= self.state.tier)
            .filter_map(|n| n.p.workers.map(|w| self.params.get(w)))
            .sum()
    }

    pub fn worker_pool(&self) -> i64 {
        self.eff[self.chain.g.workers_total]
    }

    pub fn is_locked(&self, n: usize) -> bool {
        self.chain.nodes[n].tier > self.state.tier
    }

    /// Is node `n` currently broken down by an event?
    pub fn is_broken(&self, n: usize) -> bool {
        self.state
            .events
            .iter()
            .any(|e| e.kind == EventKind::Breakdown && e.target == Some(n))
    }

    /// Theoretical max primary output per minute of node `n` with current
    /// effective parameters (ignoring starvation/blocking).
    pub fn capacity_per_min(&self, n: usize) -> Fx {
        node_capacity_per_min(&self.chain, &self.eff, n)
    }
}

/// Theoretical output per minute of a node at its current crew.
pub fn node_capacity_per_min(chain: &Chain, eff: &[i64], n: usize) -> Fx {
    let node = &chain.nodes[n];
    let crew = match crew_gate(node, eff, eff[chain.g.diminishing]) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let units = node.p.units.map_or(1, |p| eff[p]);
    let e = eff[node.p.efficiency];
    let per_sec = match &node.kind {
        chain::Kind::Source(_) => pct(
            crate::fixed::mul(node.p.rate.map_or(0, |p| eff[p]) * units, crew),
            e,
        ),
        chain::Kind::Processor(r) => {
            let cycle = node.p.cycle.map_or(1, |p| eff[p]).max(1);
            let per_cycle = pct(r.outputs[0] * r.batch * units, e);
            crate::fixed::mul(per_cycle, crew) * TICKS_PER_SEC as i64 / cycle
        }
        chain::Kind::Sink(s) => pct(
            crate::fixed::mul(
                crate::fixed::mul(node.p.lift.map_or(0, |p| eff[p]) * units, crew),
                s.progress,
            ),
            e,
        ),
        chain::Kind::Market => chain
            .market
            .as_ref()
            .map_or(0, |m| eff[m.sell_rate] * m.goods.len() as i64),
    };
    per_sec * 60
}

/// Build effective parameter values. See the module docs for the layers.
pub fn compute_eff(chain: &Chain, params: &ParamSet, st: &State) -> Vec<i64> {
    let mut eff = params.values();
    let mut touched = vec![];
    for (u, up) in chain.upgrades.iter().enumerate() {
        let lvl = st.upgrades.get(u).copied().unwrap_or(0) as i64;
        if lvl == 0 {
            continue;
        }
        for (pid, e) in &up.effects {
            if let Effect::Add(a) = e {
                eff[*pid] += a * lvl;
                touched.push(*pid);
            }
        }
    }
    for (u, up) in chain.upgrades.iter().enumerate() {
        let lvl = st.upgrades.get(u).copied().unwrap_or(0) as i32;
        if lvl == 0 {
            continue;
        }
        for (pid, e) in &up.effects {
            if let Effect::Mul(f) = e {
                eff[*pid] = (eff[*pid] as f64 * f.powi(lvl)).round() as i64;
                touched.push(*pid);
            }
        }
    }
    for pid in touched {
        let p = &params.list[pid];
        eff[pid] = eff[pid].clamp(p.min, p.max);
    }

    for node in &chain.nodes {
        if node.tier > st.tier {
            eff[node.p.enabled] = 0;
        }
    }
    for e in &st.events {
        match e.kind {
            EventKind::Drought => {
                eff[chain.g.water] = pct(eff[chain.g.water], 100 - e.severity);
            }
            EventKind::Breakdown => {
                if let Some(n) = e.target {
                    eff[chain.nodes[n].p.enabled] = 0;
                }
            }
            _ => {}
        }
    }

    let mut remaining = eff[chain.g.workers_total].max(0);
    for &n in &chain.order {
        let node = &chain.nodes[n];
        let Some(w) = node.p.workers else { continue };
        if node.tier > st.tier {
            eff[w] = 0;
            continue;
        }
        let give = eff[w].min(remaining).max(0);
        eff[w] = give;
        remaining -= give;
    }
    eff
}

/// Shared gate for every node kind: off nodes are Off, crewed nodes with no
/// workers are Unstaffed, otherwise return the crew factor (which accounts
/// for the number of parallel buildings).
pub(crate) fn crew_gate(node: &Node, eff: &[i64], diminishing: i64) -> Result<Fx, Status> {
    if eff[node.p.enabled] == 0 {
        return Err(Status::Off);
    }
    let w = node.p.workers.map_or(0, |p| eff[p]);
    let units = node.p.units.map_or(1, |p| eff[p]).max(1);
    let crew = workers::crew_factor(w, node.optimal_workers * units, diminishing);
    if crew == 0 {
        Err(Status::Unstaffed)
    } else {
        Ok(crew)
    }
}

impl Chain {
    /// An empty chain, used only as a placeholder during construction.
    fn empty() -> Chain {
        Chain {
            name: String::new(),
            resources: vec![],
            nodes: vec![],
            links: vec![],
            order: vec![],
            depth: vec![],
            g: chain::GlobalParams {
                workers_total: 0,
                upkeep: 0,
                diminishing: 0,
                water: 0,
                regrowth: 0,
                discount: 0,
                ev_enabled: 0,
                ev_frequency: 0,
                ev_severity: 0,
            },
            market: None,
            upgrades: vec![],
            gates: vec![],
            start_coin: 0,
        }
    }
}
