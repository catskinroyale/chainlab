//! Data-driven chain definitions (RON).
//!
//! A chain file lists resources, nodes, the links between them, market goods,
//! upgrades, gates and event settings. Values are in human units (units,
//! seconds, percent, coin); the engine converts them to fixed point when it
//! compiles the chain. Rename anything, add nodes or re-wire links here
//! without recompiling. See `data/chain_default.ron` for a full example and
//! the README for a walkthrough of adding a node.

use std::collections::BTreeMap;

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
    /// Sellable goods (only used if the chain has a Market node).
    #[serde(default)]
    pub market: MarketDef,
    #[serde(default)]
    pub upgrades: Vec<UpgradeDef>,
    /// Milestones in order. Passing gate k raises the tier to k+1.
    #[serde(default)]
    pub gates: Vec<GateDef>,
    #[serde(default)]
    pub events: EventsDef,
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
    /// Crew size per building that gives 100% speed. 0 = needs no workers.
    #[serde(default)]
    pub optimal_workers: u32,
    /// Number of parallel buildings. Multiplies output and optimal crew.
    #[serde(default = "d_one")]
    pub units: u32,
    #[serde(default = "d_true")]
    pub enabled: bool,
    /// Tier needed before the node operates (0 = available from the start).
    #[serde(default)]
    pub tier: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeKindDef {
    Source(SourceDef),
    Processor(RecipeDef),
    Sink(SinkDef),
    Market,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceDef {
    pub output: String,
    /// Units per second per building at full crew and 100% efficiency.
    pub rate: f64,
    /// Random yield variance in percent (0 = none).
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

/// A final consumer (the Tower). Consumes input "sets" and produces Progress.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SinkDef {
    /// One set = these quantities of each input.
    pub inputs: Vec<(String, f64)>,
    /// Progress gained per set.
    pub progress: f64,
    /// Lift/scaffold limit: sets per second per building at full crew.
    pub lift_rate: f64,
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
    /// Size of the worker pool.
    #[serde(default = "d_workers")]
    pub workers_total: u32,
    /// Coin per worker per minute (paid for the whole pool, idle or not).
    #[serde(default = "d_one_f")]
    pub worker_upkeep: f64,
    #[serde(default = "d_coin")]
    pub start_coin: f64,
    /// Global discount on all upgrade costs, percent.
    #[serde(default)]
    pub cost_discount: u32,
}

impl Default for GlobalDef {
    fn default() -> Self {
        GlobalDef {
            diminishing: d_50(),
            water_supply: d_one_f(),
            regrowth: d_regrowth(),
            workers_total: d_workers(),
            worker_upkeep: d_one_f(),
            start_coin: d_coin(),
            cost_discount: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketDef {
    #[serde(default)]
    pub goods: Vec<GoodDef>,
    /// 0 = sell all, 1 = sell only above threshold, 2 = keep a reserve.
    #[serde(default)]
    pub policy: u32,
    /// Policy 1: sell only when price >= this percent of base price.
    #[serde(default = "d_80")]
    pub threshold_pct: u32,
    /// Policy 2: units of each good to keep unsold.
    #[serde(default)]
    pub reserve: f64,
    /// Max units of each good the market absorbs per second.
    #[serde(default = "d_sell")]
    pub sell_rate: f64,
}

impl Default for MarketDef {
    fn default() -> Self {
        MarketDef {
            goods: vec![],
            policy: 0,
            threshold_pct: d_80(),
            reserve: 0.0,
            sell_rate: d_sell(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoodDef {
    pub resource: String,
    pub base_price: f64,
    /// Price sensitivity: price = base / (1 + elasticity * pressure).
    #[serde(default = "d_elastic")]
    pub elasticity: f64,
    /// Share of sales pressure that fades per second.
    #[serde(default = "d_recovery")]
    pub recovery: f64,
    /// Random drift strength, percent.
    #[serde(default)]
    pub volatility: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpgradeDef {
    pub id: String,
    pub name: String,
    pub base_cost: f64,
    /// Cost multiplier per level owned: cost = base * growth ^ owned.
    pub growth: f64,
    pub effects: Vec<EffectDef>,
    #[serde(default)]
    pub max_level: Option<u32>,
    #[serde(default)]
    pub tier: u32,
}

/// Effect of one upgrade level on a parameter (by key, in human units).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EffectDef {
    /// value += amount per level
    Add(String, f64),
    /// value *= factor per level
    Mul(String, f64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GateDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub condition: GateCond,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GateCond {
    /// Tower progress reached.
    Progress(f64),
    /// Coin balance reached.
    Coin(f64),
    /// Cumulative operating profit (income - upkeep) reached.
    Profit(f64),
    /// A node's total output reached.
    Produced(String, f64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventsDef {
    #[serde(default)]
    pub enabled: bool,
    /// Expected events per simulated minute.
    #[serde(default = "d_freq")]
    pub frequency: f64,
    /// 0..100: how hard events hit.
    #[serde(default = "d_50")]
    pub severity: u32,
}

impl Default for EventsDef {
    fn default() -> Self {
        EventsDef {
            enabled: false,
            frequency: d_freq(),
            severity: d_50(),
        }
    }
}

/// A scenario: a chain plus parameter overrides that become the defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Built-in chain name or path.
    #[serde(default = "d_chain")]
    pub chain: String,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Parameter key -> value in human units.
    #[serde(default)]
    pub overrides: BTreeMap<String, f64>,
}

/// A saved parameter set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedParams {
    pub scenario: String,
    pub seed: u64,
    pub values: BTreeMap<String, f64>,
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
fn d_80() -> u32 {
    80
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
fn d_workers() -> u32 {
    12
}
fn d_coin() -> f64 {
    100.0
}
fn d_sell() -> f64 {
    2.0
}
fn d_elastic() -> f64 {
    0.02
}
fn d_recovery() -> f64 {
    0.05
}
fn d_freq() -> f64 {
    0.5
}
fn d_chain() -> String {
    "default".into()
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

pub fn parse_scenario(text: &str) -> Result<ScenarioDef, String> {
    ron_options()
        .from_str(text)
        .map_err(|e| format!("scenario: {e}"))
}

pub fn to_ron<T: Serialize>(v: &T) -> Result<String, String> {
    ron::ser::to_string_pretty(v, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())
}
