//! Data-driven chain definitions (RON).
//!
//! A chain file lists resources, nodes and the links between them. Values are
//! in human units (units, seconds, percent); the engine converts them to fixed
//! point when it compiles the chain. Rename anything, add nodes or re-wire
//! links here without recompiling.
//!
//! ```ron
//! (
//!   name: "Tiny",
//!   resources: [(id: "stone", name: "Stone"), (id: "bricks", name: "Bricks")],
//!   nodes: [
//!     (id: "quarry", name: "Quarry",
//!      kind: Source(output: "stone", rate: 1.0), out_cap: 20.0, workers: 2, optimal_workers: 2),
//!     (id: "kiln", name: "Kiln",
//!      kind: Processor(inputs: [("stone", 2.0)], outputs: [("bricks", 1.0)], cycle_secs: 4.0),
//!      in_cap: 20.0, out_cap: 20.0, workers: 2, optimal_workers: 2),
//!   ],
//!   links: [(from: "quarry", to: "kiln", resource: "stone", carriers: 2, capacity: 4.0, distance_secs: 3.0)],
//! )
//! ```

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainConfig {
    pub name: String,
    pub resources: Vec<ResourceDef>,
    pub nodes: Vec<NodeDef>,
    #[serde(default)]
    pub links: Vec<LinkDef>,
    #[serde(default)]
    pub globals: GlobalDef,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceDef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeDef {
    pub id: String,
    pub name: String,
    pub kind: NodeKindDef,
    /// Capacity of each input buffer (one buffer per input resource).
    #[serde(default = "d_cap")]
    pub in_cap: f64,
    /// Capacity of each output buffer.
    #[serde(default = "d_cap")]
    pub out_cap: f64,
    /// Output multiplier in percent (100 = recipe nominal).
    #[serde(default = "d_100")]
    pub efficiency: u32,
    /// Workers assigned at start.
    #[serde(default)]
    pub workers: u32,
    /// Crew size that gives 100% speed. 0 = node needs no workers.
    #[serde(default)]
    pub optimal_workers: u32,
    #[serde(default = "d_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeKindDef {
    Source(SourceDef),
    Processor(RecipeDef),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceDef {
    pub output: String,
    /// Units per second at full crew and 100% efficiency.
    pub rate: f64,
    /// Random yield variance in percent (0 = none). Each tick's yield is
    /// multiplied by a uniform factor in `1 +/- variance`.
    #[serde(default)]
    pub variance: u32,
    #[serde(default)]
    pub stock: StockDef,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum StockDef {
    /// Never runs out.
    #[default]
    Infinite,
    /// A fixed deposit that is used up.
    Depleting { amount: f64 },
    /// A living stock that regrows logistically towards `max`, needing
    /// `water_need` water per second for full growth speed.
    Regrowing {
        standing: f64,
        max: f64,
        water_need: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipeDef {
    pub inputs: Vec<(String, f64)>,
    pub outputs: Vec<(String, f64)>,
    pub cycle_secs: f64,
    /// Recipe multiples per cycle.
    #[serde(default = "d_one")]
    pub batch: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkDef {
    pub from: String,
    pub to: String,
    pub resource: String,
    pub carriers: u32,
    /// Units per carrier load.
    pub capacity: f64,
    /// One-way travel time.
    pub distance_secs: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlobalDef {
    /// Percent of a worker's contribution each extra worker beyond optimal
    /// adds, compounding (50 = 0.5, 0.25, 0.125...).
    #[serde(default = "d_50")]
    pub diminishing: u32,
    /// Water supplied per second to regrowing sources.
    #[serde(default = "d_one_f")]
    pub water_supply: f64,
    /// Intrinsic logistic regrowth rate per second.
    #[serde(default = "d_regrowth")]
    pub regrowth: f64,
}

impl Default for GlobalDef {
    fn default() -> Self {
        GlobalDef {
            diminishing: d_50(),
            water_supply: d_one_f(),
            regrowth: d_regrowth(),
        }
    }
}

fn d_cap() -> f64 {
    20.0
}
fn d_100() -> u32 {
    100
}
fn d_50() -> u32 {
    50
}
fn d_true() -> bool {
    true
}
fn d_one() -> u32 {
    1
}
fn d_one_f() -> f64 {
    1.0
}
fn d_regrowth() -> f64 {
    0.05
}

/// RON options used for all CHAINLAB files: newtype enum variants can be
/// written without double parentheses, and `Some(..)` may be omitted.
pub fn ron_options() -> ron::Options {
    use ron::extensions::Extensions;
    ron::Options::default()
        .with_default_extension(Extensions::UNWRAP_VARIANT_NEWTYPES | Extensions::IMPLICIT_SOME)
}

/// Parse a chain definition from RON text.
pub fn parse_chain(text: &str) -> Result<ChainConfig, String> {
    ron_options()
        .from_str(text)
        .map_err(|e| format!("chain config: {e}"))
}
