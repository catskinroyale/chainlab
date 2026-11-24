//! Dynamic simulation state: everything that changes tick to tick.
//!
//! Only integers live here (no `f64`), so two states can be compared exactly
//! with `==`, which is how the determinism tests work.

use crate::fixed::Fx;
use crate::rng::Rngs;

use super::chain::{Chain, Kind, StockSpec};
use super::stats::Stats;

/// What a node did this tick. Every node is in exactly one state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Producing.
    Working,
    /// Wants to work but an input buffer is short (or a deposit is empty).
    Starved,
    /// Wants to work but its output buffer has no room.
    Blocked,
    /// Switched off by the player (or an event).
    Off,
    /// Needs workers and has none.
    Unstaffed,
}

impl Status {
    pub const ALL: [Status; 5] = [
        Status::Working,
        Status::Starved,
        Status::Blocked,
        Status::Off,
        Status::Unstaffed,
    ];

    pub fn idx(self) -> usize {
        self as usize
    }

    pub fn tag(self) -> &'static str {
        match self {
            Status::Working => "[WORK]",
            Status::Starved => "[STARVED]",
            Status::Blocked => "[BLOCKED]",
            Status::Off => "[OFF]",
            Status::Unstaffed => "[NOCREW]",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Status::Working => "WORK",
            Status::Starved => "STRV",
            Status::Blocked => "BLKD",
            Status::Off => "OFF",
            Status::Unstaffed => "CREW",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeState {
    /// Input buffers, aligned with `Node::in_res`.
    pub inputs: Vec<Fx>,
    /// Output buffers, aligned with `Node::out_res`.
    pub outputs: Vec<Fx>,
    /// Finished goods that did not fit in the output buffer yet.
    pub pending_out: Vec<Fx>,
    /// Work done on the current cycle, in milli-ticks of full-crew work.
    pub progress: Fx,
    pub in_cycle: bool,
    /// Remaining deposit / standing stock for depleting or regrowing sources.
    pub stock: Fx,
    pub status: Status,
    /// Primary output produced this tick (for throughput stats).
    pub out_tick: Fx,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shipment {
    pub qty: Fx,
    pub depart: u64,
    pub arrive: u64,
    /// Set once fully unloaded: the carrier is back and free at this tick.
    pub return_at: Option<u64>,
}

impl Shipment {
    pub fn outbound(&self) -> bool {
        self.return_at.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LinkState {
    /// Every busy carrier: loaded and travelling/waiting, or returning empty.
    pub shipments: Vec<Shipment>,
    pub delivered_total: Fx,
}

impl LinkState {
    pub fn in_transit(&self) -> Fx {
        self.shipments
            .iter()
            .filter(|s| s.outbound())
            .map(|s| s.qty)
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub tick: u64,
    pub nodes: Vec<NodeState>,
    pub links: Vec<LinkState>,
    pub rng: Rngs,
    pub stats: Stats,
}

impl State {
    pub fn new(chain: &Chain, seed: u64) -> Self {
        let nodes = chain
            .nodes
            .iter()
            .map(|n| NodeState {
                inputs: vec![0; n.in_res.len()],
                outputs: vec![0; n.out_res.len()],
                pending_out: vec![0; n.out_res.len()],
                progress: 0,
                in_cycle: false,
                stock: match &n.kind {
                    Kind::Source(s) => match s.stock {
                        StockSpec::Infinite => 0,
                        StockSpec::Depleting { initial } => initial,
                        StockSpec::Regrowing { initial, .. } => initial,
                    },
                    _ => 0,
                },
                status: Status::Starved,
                out_tick: 0,
            })
            .collect();
        State {
            tick: 0,
            nodes,
            links: vec![LinkState::default(); chain.links.len()],
            rng: Rngs::new(seed),
            stats: Stats::new(chain.nodes.len()),
        }
    }
}
