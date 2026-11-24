//! Dynamic simulation state: everything that changes tick to tick.
//!
//! Only integers and strings live here (no `f64`), so two states can be
//! compared exactly with `==`, which is how the determinism tests work.

use std::collections::VecDeque;

use crate::fixed::{Fx, ONE};
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
    /// Wants to work but its output buffer has no room (markets: holding
    /// stock because the price is below the sell policy's threshold).
    Blocked,
    /// Switched off, broken down, or locked behind a tier.
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
    pub delivered_tick: Fx,
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
pub struct GoodState {
    /// Recent sales volume that is depressing the price.
    pub pressure: Fx,
    /// Random-walk multiplier on the base price (ONE = no drift).
    pub drift: Fx,
    /// Current price per unit.
    pub price: Fx,
    pub sold_tick: Fx,
    pub revenue_tick: Fx,
    pub sold_total: Fx,
    pub revenue_total: Fx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// Water supply cut by severity%.
    Drought,
    /// One good's base price cut hard.
    PriceCrash,
    /// A processor stops working.
    Breakdown,
    /// A one-off coin bonus.
    Windfall,
}

impl EventKind {
    pub const ALL: [EventKind; 4] = [
        EventKind::Drought,
        EventKind::PriceCrash,
        EventKind::Breakdown,
        EventKind::Windfall,
    ];
    pub fn name(self) -> &'static str {
        match self {
            EventKind::Drought => "Drought",
            EventKind::PriceCrash => "Price crash",
            EventKind::Breakdown => "Breakdown",
            EventKind::Windfall => "Windfall",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveEvent {
    pub kind: EventKind,
    /// Node (breakdown) or market good index (price crash).
    pub target: Option<usize>,
    pub severity: i64,
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Info,
    Lever,
    Purchase,
    Event,
    Unlock,
    Challenge,
    Warn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub tick: u64,
    pub kind: LogKind,
    pub text: String,
}

/// Money in and out, this tick and since start.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Ledger {
    pub income_tick: Fx,
    pub upkeep_tick: Fx,
    pub purchase_tick: Fx,
    pub income_total: Fx,
    pub upkeep_total: Fx,
    pub purchase_total: Fx,
    /// Sub-milli remainder of upkeep, so per-tick rounding never loses coin.
    pub upkeep_acc: i64,
}

impl Ledger {
    /// Operating profit since start: income minus upkeep (purchases are
    /// investments, not running costs).
    pub fn operating_profit(&self) -> Fx {
        self.income_total - self.upkeep_total
    }
}

pub const LOG_CAP: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub tick: u64,
    pub nodes: Vec<NodeState>,
    pub links: Vec<LinkState>,
    pub rng: Rngs,
    pub stats: Stats,
    pub coin: Fx,
    pub ledger: Ledger,
    /// Levels owned per upgrade.
    pub upgrades: Vec<u32>,
    /// Gates passed.
    pub tier: u32,
    /// Total progress made by sinks.
    pub progress: Fx,
    pub market: Vec<GoodState>,
    pub events: Vec<ActiveEvent>,
    pub log: VecDeque<LogEntry>,
}

impl State {
    pub fn new(chain: &Chain, seed: u64, base_prices: &[Fx]) -> Self {
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
        let market = base_prices
            .iter()
            .map(|&p| GoodState {
                pressure: 0,
                drift: ONE,
                price: p,
                sold_tick: 0,
                revenue_tick: 0,
                sold_total: 0,
                revenue_total: 0,
            })
            .collect();
        State {
            tick: 0,
            nodes,
            links: vec![LinkState::default(); chain.links.len()],
            rng: Rngs::new(seed),
            stats: Stats::new(chain),
            coin: chain.start_coin,
            ledger: Ledger::default(),
            upgrades: vec![0; chain.upgrades.len()],
            tier: 0,
            progress: 0,
            market,
            events: vec![],
            log: VecDeque::with_capacity(LOG_CAP),
        }
    }

    pub fn log(&mut self, kind: LogKind, text: impl Into<String>) {
        if self.log.len() == LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(LogEntry {
            tick: self.tick,
            kind,
            text: text.into(),
        });
    }
}
