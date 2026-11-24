//! The compiled, static shape of a chain.
//!
//! [`compile`] turns a human-readable [`ChainConfig`] into index-based
//! structures (no string lookups during a tick) and registers every live
//! parameter ("lever") in a [`ParamSet`]. Nothing here changes while the sim
//! runs; the changing parts live in [`super::model::State`].

use std::collections::BTreeMap;

use crate::TICKS_PER_SEC;
use crate::config::{ChainConfig, EffectDef, GateCond, NodeKindDef, StockDef};
use crate::fixed::{Fx, fx};

use super::params::{Group, ParamId, ParamSet, Unit};

pub type ResId = usize;
pub type NodeId = usize;
pub type LinkId = usize;

pub const POLICIES: &[&str] = &["sell all", "above thresh", "keep reserve"];

#[derive(Clone, Debug)]
pub struct Resource {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Chain {
    pub name: String,
    pub resources: Vec<Resource>,
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    /// Topological processing order (upstream first; ties by index).
    pub order: Vec<NodeId>,
    /// Longest-path depth of each node from a source (for the diagram).
    pub depth: Vec<usize>,
    pub g: GlobalParams,
    pub market: Option<MarketSpec>,
    pub upgrades: Vec<Upgrade>,
    pub gates: Vec<Gate>,
    pub start_coin: Fx,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Resources this node buffers on its input side.
    pub in_res: Vec<ResId>,
    /// Resources this node buffers on its output side.
    pub out_res: Vec<ResId>,
    /// Optimal crew per building.
    pub optimal_workers: i64,
    pub tier: u32,
    pub p: NodeParams,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Source(SourceSpec),
    Processor(RecipeSpec),
    Sink(SinkSpec),
    Market,
}

impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Kind::Source(_) => "source",
            Kind::Processor(_) => "processor",
            Kind::Sink(_) => "sink",
            Kind::Market => "market",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SourceSpec {
    pub stock: StockSpec,
}

#[derive(Clone, Debug)]
pub enum StockSpec {
    Infinite,
    Depleting {
        initial: Fx,
    },
    Regrowing {
        initial: Fx,
        max: Fx,
        water_need: Fx,
    },
}

/// Recipe quantities, aligned with the node's `in_res` / `out_res`.
#[derive(Clone, Debug)]
pub struct RecipeSpec {
    pub inputs: Vec<Fx>,
    pub outputs: Vec<Fx>,
    pub batch: i64,
}

/// Sink set definition, aligned with `in_res`.
#[derive(Clone, Debug)]
pub struct SinkSpec {
    pub inputs: Vec<Fx>,
    pub progress: Fx,
}

#[derive(Clone, Debug)]
pub struct Link {
    pub from: NodeId,
    pub to: NodeId,
    pub res: ResId,
    /// Index of `res` in `nodes[from].out_res`.
    pub from_slot: usize,
    /// Index of `res` in `nodes[to].in_res`.
    pub to_slot: usize,
    pub p: LinkParams,
}

#[derive(Clone, Debug)]
pub struct NodeParams {
    pub enabled: ParamId,
    pub efficiency: ParamId,
    pub units: Option<ParamId>,
    pub workers: Option<ParamId>,
    pub in_cap: Option<ParamId>,
    pub out_cap: Option<ParamId>,
    /// Sources: units per second.
    pub rate: Option<ParamId>,
    /// Sources: yield variance percent.
    pub variance: Option<ParamId>,
    /// Processors: cycle length in ticks.
    pub cycle: Option<ParamId>,
    /// Sinks: sets per second.
    pub lift: Option<ParamId>,
}

#[derive(Clone, Debug)]
pub struct LinkParams {
    pub carriers: ParamId,
    pub capacity: ParamId,
    pub distance: ParamId,
}

#[derive(Clone, Debug)]
pub struct GlobalParams {
    pub workers_total: ParamId,
    pub upkeep: ParamId,
    pub diminishing: ParamId,
    pub water: ParamId,
    pub regrowth: ParamId,
    pub discount: ParamId,
    pub ev_enabled: ParamId,
    pub ev_frequency: ParamId,
    pub ev_severity: ParamId,
}

#[derive(Clone, Debug)]
pub struct MarketSpec {
    pub node: NodeId,
    /// Aligned with `nodes[node].in_res`.
    pub goods: Vec<Good>,
    pub policy: ParamId,
    pub threshold: ParamId,
    pub reserve: ParamId,
    pub sell_rate: ParamId,
}

#[derive(Clone, Debug)]
pub struct Good {
    pub res: ResId,
    pub base_price: ParamId,
    pub elasticity: ParamId,
    pub recovery: ParamId,
    pub volatility: ParamId,
}

#[derive(Clone, Debug)]
pub enum Effect {
    Add(i64),
    Mul(f64),
}

#[derive(Clone, Debug)]
pub struct Upgrade {
    pub id: String,
    pub name: String,
    pub effects: Vec<(ParamId, Effect)>,
    pub max_level: Option<u32>,
    pub tier: u32,
    pub base_cost: ParamId,
    pub growth: ParamId,
}

#[derive(Clone, Debug)]
pub enum Cond {
    Progress(Fx),
    Coin(Fx),
    Profit(Fx),
    Produced(NodeId, Fx),
}

#[derive(Clone, Debug)]
pub struct Gate {
    pub name: String,
    pub description: String,
    pub cond: Cond,
}

impl Chain {
    pub fn node_index(&self, id: &str) -> Option<NodeId> {
        self.nodes.iter().position(|n| n.id == id)
    }
    pub fn res_name(&self, r: ResId) -> &str {
        &self.resources[r].name
    }
    pub fn link_label(&self, l: LinkId) -> String {
        let link = &self.links[l];
        format!(
            "{}->{}",
            self.nodes[link.from].name, self.nodes[link.to].name
        )
    }
    pub fn upgrade_index(&self, id: &str) -> Option<usize> {
        self.upgrades.iter().position(|u| u.id == id)
    }
    /// Links feeding into `n`.
    pub fn inbound(&self, n: NodeId) -> impl Iterator<Item = (LinkId, &Link)> {
        self.links
            .iter()
            .enumerate()
            .filter(move |(_, l)| l.to == n)
    }
    /// Links leaving `n`.
    pub fn outbound(&self, n: NodeId) -> impl Iterator<Item = (LinkId, &Link)> {
        self.links
            .iter()
            .enumerate()
            .filter(move |(_, l)| l.from == n)
    }
    /// Nodes that make "end-to-end" output: sinks if any, else the market.
    pub fn sinks(&self) -> Vec<NodeId> {
        let s: Vec<_> = (0..self.nodes.len())
            .filter(|&i| matches!(self.nodes[i].kind, Kind::Sink(_)))
            .collect();
        if s.is_empty() {
            self.market
                .as_ref()
                .map(|m| vec![m.node])
                .unwrap_or_default()
        } else {
            s
        }
    }
}

const QTY_RANGE: (i64, i64) = (0, 10_000_000);
const QTY_STEPS: (i64, i64) = (1_000, 10_000);

/// Compile a config into a [`Chain`] plus its parameter registry.
pub fn compile(cfg: &ChainConfig) -> Result<(Chain, ParamSet), String> {
    let mut ps = ParamSet::default();

    // Resources
    let mut res_ix: BTreeMap<&str, ResId> = BTreeMap::new();
    let mut resources = Vec::new();
    for (i, r) in cfg.resources.iter().enumerate() {
        if res_ix.insert(r.id.as_str(), i).is_some() {
            return Err(format!("duplicate resource id '{}'", r.id));
        }
        resources.push(Resource {
            id: r.id.clone(),
            name: r.name.clone(),
        });
    }
    let res = |id: &str| -> Result<ResId, String> {
        res_ix
            .get(id)
            .copied()
            .ok_or_else(|| format!("unknown resource '{id}'"))
    };

    // Globals
    let gd = &cfg.globals;
    let gg = Group::Global;
    let g = GlobalParams {
        workers_total: ps.add(
            "global.workers",
            "Worker pool",
            gg,
            Unit::Count,
            gd.workers_total as i64,
            (0, 500),
            (1, 5),
        ),
        upkeep: ps.add(
            "global.upkeep",
            "Upkeep /worker/min",
            gg,
            Unit::Coin,
            fx(gd.worker_upkeep),
            (0, 1_000_000),
            (100, 1_000),
        ),
        diminishing: ps.add(
            "global.diminishing",
            "Over-crew returns",
            gg,
            Unit::Percent,
            gd.diminishing as i64,
            (0, 100),
            (5, 20),
        ),
        water: ps.add(
            "global.water",
            "Water supply",
            gg,
            Unit::Rate,
            fx(gd.water_supply),
            QTY_RANGE,
            (100, 1_000),
        ),
        regrowth: ps.add(
            "global.regrowth",
            "Regrowth rate",
            gg,
            Unit::Ratio,
            fx(gd.regrowth),
            (0, 1_000),
            (5, 50),
        ),
        discount: ps.add(
            "cost.discount",
            "Bulk discount",
            Group::Costs,
            Unit::Percent,
            gd.cost_discount as i64,
            (0, 90),
            (5, 20),
        ),
        ev_enabled: ps.add(
            "events.enabled",
            "Random events",
            Group::Events,
            Unit::Bool,
            cfg.events.enabled as i64,
            (0, 1),
            (1, 1),
        ),
        ev_frequency: ps.add(
            "events.frequency",
            "Events per minute",
            Group::Events,
            Unit::Ratio,
            fx(cfg.events.frequency),
            (0, 20_000),
            (100, 1_000),
        ),
        ev_severity: ps.add(
            "events.severity",
            "Event severity",
            Group::Events,
            Unit::Percent,
            cfg.events.severity as i64,
            (0, 100),
            (5, 20),
        ),
    };

    // Nodes
    let mut node_ix: BTreeMap<&str, NodeId> = BTreeMap::new();
    let mut nodes = Vec::with_capacity(cfg.nodes.len());
    let mut market_node = None;
    for (ni, nd) in cfg.nodes.iter().enumerate() {
        if node_ix.insert(nd.id.as_str(), ni).is_some() {
            return Err(format!("duplicate node id '{}'", nd.id));
        }
        let key = |s: &str| format!("node.{}.{s}", nd.id);
        let label = |s: &str| format!("{}: {s}", nd.name);
        let grp = Group::Node(ni);

        let enabled = ps.add(
            key("enabled"),
            label("on/off"),
            grp,
            Unit::Bool,
            nd.enabled as i64,
            (0, 1),
            (1, 1),
        );
        let workers = (nd.optimal_workers > 0).then(|| {
            ps.add(
                key("workers"),
                label("workers"),
                grp,
                Unit::Count,
                nd.workers as i64,
                (0, 200),
                (1, 5),
            )
        });
        let efficiency = ps.add(
            key("efficiency"),
            label("efficiency"),
            grp,
            Unit::Percent,
            nd.efficiency as i64,
            (5, 500),
            (5, 25),
        );
        let units = (!matches!(nd.kind, NodeKindDef::Market)).then(|| {
            ps.add(
                key("units"),
                label("buildings"),
                grp,
                Unit::Count,
                nd.units.max(1) as i64,
                (1, 100),
                (1, 5),
            )
        });

        let mut rate = None;
        let mut variance = None;
        let mut cycle = None;
        let mut lift = None;
        let (kind, in_res, out_res) = match &nd.kind {
            NodeKindDef::Source(s) => {
                let stock = match s.stock {
                    StockDef::Infinite => StockSpec::Infinite,
                    StockDef::Depleting { amount } => StockSpec::Depleting {
                        initial: fx(amount),
                    },
                    StockDef::Regrowing {
                        standing,
                        max,
                        water_need,
                    } => StockSpec::Regrowing {
                        initial: fx(standing),
                        max: fx(max),
                        water_need: fx(water_need),
                    },
                };
                rate = Some(ps.add(
                    key("rate"),
                    label("harvest rate"),
                    grp,
                    Unit::Rate,
                    fx(s.rate),
                    QTY_RANGE,
                    (100, 1_000),
                ));
                variance = Some(ps.add(
                    key("variance"),
                    label("yield variance"),
                    grp,
                    Unit::Percent,
                    s.variance as i64,
                    (0, 100),
                    (5, 20),
                ));
                (
                    Kind::Source(SourceSpec { stock }),
                    vec![],
                    vec![res(&s.output)?],
                )
            }
            NodeKindDef::Processor(r) => {
                if r.inputs.is_empty() || r.outputs.is_empty() {
                    return Err(format!("processor '{}' needs inputs and outputs", nd.id));
                }
                let (in_res, inputs) = split(&r.inputs, &res)?;
                let (out_res, outputs) = split(&r.outputs, &res)?;
                cycle = Some(ps.add(
                    key("cycle"),
                    label("cycle time"),
                    grp,
                    Unit::Ticks,
                    (r.cycle_secs * TICKS_PER_SEC as f64).round() as i64,
                    (1, 36_000),
                    (1, 10),
                ));
                (
                    Kind::Processor(RecipeSpec {
                        inputs,
                        outputs,
                        batch: r.batch.max(1) as i64,
                    }),
                    in_res,
                    out_res,
                )
            }
            NodeKindDef::Sink(s) => {
                if s.inputs.is_empty() {
                    return Err(format!("sink '{}' needs inputs", nd.id));
                }
                let (in_res, inputs) = split(&s.inputs, &res)?;
                lift = Some(ps.add(
                    key("lift"),
                    label("lift rate"),
                    grp,
                    Unit::Rate,
                    fx(s.lift_rate),
                    QTY_RANGE,
                    (50, 500),
                ));
                (
                    Kind::Sink(SinkSpec {
                        inputs,
                        progress: fx(s.progress),
                    }),
                    in_res,
                    vec![],
                )
            }
            NodeKindDef::Market => {
                if market_node.replace(ni).is_some() {
                    return Err("only one Market node is supported".into());
                }
                let mut in_res = vec![];
                for gdef in &cfg.market.goods {
                    in_res.push(res(&gdef.resource)?);
                }
                (Kind::Market, in_res, vec![])
            }
        };

        let in_cap = (!in_res.is_empty()).then(|| {
            ps.add(
                key("in_cap"),
                label("input cap"),
                grp,
                Unit::Qty,
                fx(nd.in_cap),
                QTY_RANGE,
                QTY_STEPS,
            )
        });
        let out_cap = (!out_res.is_empty()).then(|| {
            ps.add(
                key("out_cap"),
                label("output cap"),
                grp,
                Unit::Qty,
                fx(nd.out_cap),
                QTY_RANGE,
                QTY_STEPS,
            )
        });

        nodes.push(Node {
            id: nd.id.clone(),
            name: nd.name.clone(),
            kind,
            in_res,
            out_res,
            optimal_workers: nd.optimal_workers as i64,
            tier: nd.tier,
            p: NodeParams {
                enabled,
                efficiency,
                units,
                workers,
                in_cap,
                out_cap,
                rate,
                variance,
                cycle,
                lift,
            },
        });
    }

    // Links
    let mut links = Vec::with_capacity(cfg.links.len());
    for (li, ld) in cfg.links.iter().enumerate() {
        let from = *node_ix
            .get(ld.from.as_str())
            .ok_or_else(|| format!("link {li}: unknown node '{}'", ld.from))?;
        let to = *node_ix
            .get(ld.to.as_str())
            .ok_or_else(|| format!("link {li}: unknown node '{}'", ld.to))?;
        let r = res(&ld.resource)?;
        let from_slot = nodes[from]
            .out_res
            .iter()
            .position(|&x| x == r)
            .ok_or_else(|| format!("link {li}: '{}' does not output '{}'", ld.from, ld.resource))?;
        let to_slot = nodes[to]
            .in_res
            .iter()
            .position(|&x| x == r)
            .ok_or_else(|| format!("link {li}: '{}' does not take '{}'", ld.to, ld.resource))?;
        let key = |s: &str| format!("link.{li}.{s}");
        let label = |s: &str| format!("{}>{}: {s}", nodes[from].name, nodes[to].name);
        let grp = Group::Link(li);
        let p = LinkParams {
            carriers: ps.add(
                key("carriers"),
                label("carriers"),
                grp,
                Unit::Count,
                ld.carriers as i64,
                (0, 200),
                (1, 5),
            ),
            capacity: ps.add(
                key("capacity"),
                label("load/carrier"),
                grp,
                Unit::Qty,
                fx(ld.capacity),
                QTY_RANGE,
                (500, 5_000),
            ),
            distance: ps.add(
                key("distance"),
                label("distance"),
                grp,
                Unit::Ticks,
                (ld.distance_secs * TICKS_PER_SEC as f64).round() as i64,
                (1, 6_000),
                (5, 50),
            ),
        };
        links.push(Link {
            from,
            to,
            res: r,
            from_slot,
            to_slot,
            p,
        });
    }

    // Market
    let market = match market_node {
        None => None,
        Some(node) => {
            let md = &cfg.market;
            let gm = Group::Market;
            let policy = ps.add(
                "market.policy",
                "Sell policy",
                gm,
                Unit::Choice(POLICIES),
                md.policy.min(2) as i64,
                (0, 2),
                (1, 1),
            );
            let threshold = ps.add(
                "market.threshold",
                "Threshold % of base",
                gm,
                Unit::Percent,
                md.threshold_pct as i64,
                (0, 300),
                (5, 20),
            );
            let reserve = ps.add(
                "market.reserve",
                "Reserve kept",
                gm,
                Unit::Qty,
                fx(md.reserve),
                QTY_RANGE,
                QTY_STEPS,
            );
            let sell_rate = ps.add(
                "market.sell_rate",
                "Sell rate per good",
                gm,
                Unit::Rate,
                fx(md.sell_rate),
                QTY_RANGE,
                (100, 1_000),
            );
            let mut goods = vec![];
            for gdef in &md.goods {
                let r = res(&gdef.resource)?;
                let nm = &resources[r].name;
                let key = |s: &str| format!("market.{}.{s}", gdef.resource);
                goods.push(Good {
                    res: r,
                    base_price: ps.add(
                        key("price"),
                        format!("{nm}: base price"),
                        gm,
                        Unit::Coin,
                        fx(gdef.base_price),
                        (0, 1_000_000_000),
                        (100, 1_000),
                    ),
                    elasticity: ps.add(
                        key("elasticity"),
                        format!("{nm}: elasticity"),
                        gm,
                        Unit::Ratio,
                        fx(gdef.elasticity),
                        (0, 1_000),
                        (2, 20),
                    ),
                    recovery: ps.add(
                        key("recovery"),
                        format!("{nm}: recovery"),
                        gm,
                        Unit::Ratio,
                        fx(gdef.recovery),
                        (0, 1_000),
                        (5, 50),
                    ),
                    volatility: ps.add(
                        key("volatility"),
                        format!("{nm}: volatility"),
                        gm,
                        Unit::Percent,
                        gdef.volatility as i64,
                        (0, 100),
                        (5, 20),
                    ),
                });
            }
            Some(MarketSpec {
                node,
                goods,
                policy,
                threshold,
                reserve,
                sell_rate,
            })
        }
    };

    // Upgrades (effects resolved after every parameter exists)
    let mut upgrades = vec![];
    for ud in &cfg.upgrades {
        let key = |s: &str| format!("cost.{}.{s}", ud.id);
        let base_cost = ps.add(
            key("base"),
            format!("{}: base cost", ud.name),
            Group::Costs,
            Unit::Coin,
            fx(ud.base_cost),
            (0, 1_000_000_000),
            (1_000, 10_000),
        );
        let growth = ps.add(
            key("growth"),
            format!("{}: growth", ud.name),
            Group::Costs,
            Unit::Ratio,
            fx(ud.growth),
            (1_000, 5_000),
            (10, 100),
        );
        let mut effects = vec![];
        for e in &ud.effects {
            let (k, eff) = match e {
                EffectDef::Add(k, amount) => {
                    let id = ps
                        .id(k)
                        .ok_or_else(|| format!("upgrade '{}': unknown parameter '{k}'", ud.id))?;
                    (id, Effect::Add(ps.list[id].unit.from_human(*amount)))
                }
                EffectDef::Mul(k, f) => {
                    let id = ps
                        .id(k)
                        .ok_or_else(|| format!("upgrade '{}': unknown parameter '{k}'", ud.id))?;
                    (id, Effect::Mul(*f))
                }
            };
            effects.push((k, eff));
        }
        upgrades.push(Upgrade {
            id: ud.id.clone(),
            name: ud.name.clone(),
            effects,
            max_level: ud.max_level,
            tier: ud.tier,
            base_cost,
            growth,
        });
    }

    // Gates
    let mut gates = vec![];
    for gd in &cfg.gates {
        let cond = match &gd.condition {
            GateCond::Progress(x) => Cond::Progress(fx(*x)),
            GateCond::Coin(x) => Cond::Coin(fx(*x)),
            GateCond::Profit(x) => Cond::Profit(fx(*x)),
            GateCond::Produced(n, x) => Cond::Produced(
                *node_ix
                    .get(n.as_str())
                    .ok_or_else(|| format!("gate '{}': unknown node '{n}'", gd.name))?,
                fx(*x),
            ),
        };
        gates.push(Gate {
            name: gd.name.clone(),
            description: gd.description.clone(),
            cond,
        });
    }

    let order = topo_order(nodes.len(), &links);
    let mut depth = vec![0usize; nodes.len()];
    for &n in &order {
        for l in links.iter().filter(|l| l.from == n) {
            if l.to != n {
                depth[l.to] = depth[l.to].max(depth[n] + 1);
            }
        }
    }
    Ok((
        Chain {
            name: cfg.name.clone(),
            resources,
            nodes,
            links,
            order,
            depth,
            g,
            market,
            upgrades,
            gates,
            start_coin: fx(gd.start_coin),
        },
        ps,
    ))
}

type ResLookup<'a> = dyn Fn(&str) -> Result<ResId, String> + 'a;

fn split(list: &[(String, f64)], res: &ResLookup) -> Result<(Vec<ResId>, Vec<Fx>), String> {
    let mut ids = vec![];
    let mut qty = vec![];
    for (r, q) in list {
        ids.push(res(r)?);
        qty.push(fx(*q));
    }
    Ok((ids, qty))
}

/// Kahn's algorithm, always picking the lowest-index ready node so the order
/// is deterministic. Nodes caught in a cycle are appended in index order.
fn topo_order(n: usize, links: &[Link]) -> Vec<NodeId> {
    let mut indeg = vec![0usize; n];
    for l in links {
        indeg[l.to] += 1;
    }
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while let Some(next) = (0..n).find(|&i| !done[i] && indeg[i] == 0) {
        done[next] = true;
        order.push(next);
        for l in links.iter().filter(|l| l.from == next) {
            indeg[l.to] -= 1;
        }
    }
    order.extend((0..n).filter(|&i| !done[i]));
    order
}
