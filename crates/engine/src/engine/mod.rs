//! The simulation: [`Sim`] ties together the static [`chain::Chain`], the
//! live [`params::ParamSet`] and the dynamic [`model::State`].
//!
//! ## Tick order
//!
//! One tick = 1/10 simulated second. Every tick runs the same phases in the
//! same order, and inside each phase nodes and links go in a fixed order:
//!
//! 1. **Sources**: regrow living stock, then harvest into output buffers.
//! 2. **Logistics**: unload arrived carriers, free returned ones, dispatch.
//! 3. **Processors**: advance recipe cycles (upstream nodes first).
//! 4. **Stats**: record statuses and output into the rolling window.
//!
//! Commands (lever changes) are applied *between* ticks via
//! [`Sim::command`], so a run is fully reproducible from
//! `(config, seed, [(tick, command)...])`.

pub mod chain;
pub mod logistics;
pub mod model;
pub mod params;
pub mod processor;
pub mod report;
pub mod source;
pub mod stats;
pub mod workers;

use std::sync::Arc;

use crate::config::ChainConfig;
use crate::fixed::Fx;
use crate::rng::Rngs;

use chain::{Chain, Node};
use model::{State, Status};
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
}

#[derive(Clone, Debug)]
pub struct Sim {
    pub chain: Arc<Chain>,
    pub params: ParamSet,
    pub state: State,
    pub seed: u64,
    /// Effective parameter values used by the tick. Right now these equal the
    /// raw params; upgrades and events layer on top of them later.
    eff: Vec<i64>,
}

impl Sim {
    pub fn new(cfg: &ChainConfig, seed: u64) -> Result<Self, String> {
        let (chain, params) = chain::compile(cfg)?;
        let state = State::new(&chain, seed);
        let eff = params.values();
        Ok(Sim {
            chain: Arc::new(chain),
            params,
            state,
            seed,
            eff,
        })
    }

    /// Effective value of a parameter.
    pub fn eff(&self, id: ParamId) -> i64 {
        self.eff[id]
    }

    /// Apply a command immediately (between ticks).
    pub fn command(&mut self, c: Command) {
        let n = self.params.len();
        match c {
            Command::Set(id, v) if id < n => self.params.list[id].set(v),
            Command::Adjust { id, steps, coarse } if id < n => {
                let p = &mut self.params.list[id];
                let step = if coarse { p.coarse } else { p.fine };
                p.set(p.value + steps * step);
            }
            Command::Reset(id) if id < n => {
                let p = &mut self.params.list[id];
                p.value = p.default;
            }
            Command::ResetAll => {
                for p in &mut self.params.list {
                    p.value = p.default;
                }
            }
            Command::Reseed(s) => {
                self.seed = s;
                self.state.rng = Rngs::new(s);
            }
            _ => {}
        }
        self.refresh_eff();
    }

    fn refresh_eff(&mut self) {
        self.eff = self.params.values();
    }

    /// Advance one tick.
    pub fn step(&mut self) {
        let chain = &*self.chain;
        let eff = &self.eff;
        let st = &mut self.state;
        for ns in st.nodes.iter_mut() {
            ns.out_tick = 0;
        }
        source::tick_all(chain, eff, st);
        logistics::tick(chain, eff, st);
        processor::tick_all(chain, eff, st);
        stats::record(st);
        st.tick += 1;
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

    /// Workers assigned to a node (0 if it needs none).
    pub fn workers(&self, node: &Node) -> i64 {
        node.p.workers.map_or(0, |p| self.eff[p])
    }
}

/// Shared gate for every node kind: off nodes are Off, crewed nodes with no
/// workers are Unstaffed, otherwise return the crew factor.
pub(crate) fn crew_gate(node: &Node, eff: &[i64], diminishing: i64) -> Result<Fx, Status> {
    if eff[node.p.enabled] == 0 {
        return Err(Status::Off);
    }
    let w = node.p.workers.map_or(0, |p| eff[p]);
    let crew = workers::crew_factor(w, node.optimal_workers, diminishing);
    if crew == 0 {
        Err(Status::Unstaffed)
    } else {
        Ok(crew)
    }
}
