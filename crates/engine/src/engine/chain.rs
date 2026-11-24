//! The compiled, static shape of a chain.
//!
//! [`compile`] turns a human-readable [`ChainConfig`] into index-based
//! structures (no string lookups during a tick) and registers every live
//! parameter in a [`ParamSet`]. Nothing here changes while the sim runs; the
//! changing parts live in [`super::model::State`].

use std::collections::BTreeMap;

use crate::TICKS_PER_SEC;
use crate::config::{ChainConfig, NodeKindDef, StockDef};
use crate::fixed::{Fx, fx};

use super::params::{Group, ParamId, ParamSet, Unit};

pub type ResId = usize;
pub type NodeId = usize;
pub type LinkId = usize;

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
    pub g: GlobalParams,
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
    pub optimal_workers: i64,
    pub p: NodeParams,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Source(SourceSpec),
    Processor(RecipeSpec),
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
    pub workers: Option<ParamId>,
    pub in_cap: Option<ParamId>,
    pub out_cap: Option<ParamId>,
    /// Sources: units per second.
    pub rate: Option<ParamId>,
    /// Sources: yield variance percent.
    pub variance: Option<ParamId>,
    /// Processors: cycle length in ticks.
    pub cycle: Option<ParamId>,
}

#[derive(Clone, Debug)]
pub struct LinkParams {
    pub carriers: ParamId,
    pub capacity: ParamId,
    pub distance: ParamId,
}

#[derive(Clone, Debug)]
pub struct GlobalParams {
    pub diminishing: ParamId,
    pub water: ParamId,
    pub regrowth: ParamId,
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
}

const QTY_RANGE: (i64, i64) = (0, 10_000_000);
const QTY_STEPS: (i64, i64) = (1_000, 10_000);

/// Compile a config into a [`Chain`] plus its parameter registry.
pub fn compile(cfg: &ChainConfig) -> Result<(Chain, ParamSet), String> {
    let mut ps = ParamSet::default();

    // Resources
    let mut res_ix: BTreeMap<&str, ResId> = BTreeMap::new();
    let resources: Vec<Resource> = cfg
        .resources
        .iter()
        .enumerate()
        .map(|(i, r)| {
            res_ix.insert(r.id.as_str(), i);
            Resource {
                id: r.id.clone(),
                name: r.name.clone(),
            }
        })
        .collect();
    let res = |id: &str| -> Result<ResId, String> {
        res_ix
            .get(id)
            .copied()
            .ok_or_else(|| format!("unknown resource '{id}'"))
    };

    // Globals first so their ids are stable and easy to find.
    let g = GlobalParams {
        diminishing: ps.add(
            "global.diminishing",
            "Over-crew returns",
            Group::Global,
            Unit::Percent,
            cfg.globals.diminishing as i64,
            (0, 100),
            (5, 20),
        ),
        water: ps.add(
            "global.water",
            "Water supply",
            Group::Global,
            Unit::Rate,
            fx(cfg.globals.water_supply),
            QTY_RANGE,
            (100, 1_000),
        ),
        regrowth: ps.add(
            "global.regrowth",
            "Regrowth rate",
            Group::Global,
            Unit::Ratio,
            fx(cfg.globals.regrowth),
            (0, 1_000),
            (5, 50),
        ),
    };

    // Nodes
    let mut node_ix: BTreeMap<&str, NodeId> = BTreeMap::new();
    let mut nodes = Vec::with_capacity(cfg.nodes.len());
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
                (0, 100),
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

        let (kind, in_res, out_res, rate, variance, cycle) = match &nd.kind {
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
                let rate = ps.add(
                    key("rate"),
                    label("harvest rate"),
                    grp,
                    Unit::Rate,
                    fx(s.rate),
                    QTY_RANGE,
                    (100, 1_000),
                );
                let variance = ps.add(
                    key("variance"),
                    label("yield variance"),
                    grp,
                    Unit::Percent,
                    s.variance as i64,
                    (0, 100),
                    (5, 20),
                );
                (
                    Kind::Source(SourceSpec { stock }),
                    vec![],
                    vec![res(&s.output)?],
                    Some(rate),
                    Some(variance),
                    None,
                )
            }
            NodeKindDef::Processor(r) => {
                if r.inputs.is_empty() || r.outputs.is_empty() {
                    return Err(format!("processor '{}' needs inputs and outputs", nd.id));
                }
                let mut in_res = vec![];
                let mut inputs = vec![];
                for (rid, q) in &r.inputs {
                    in_res.push(res(rid)?);
                    inputs.push(fx(*q));
                }
                let mut out_res = vec![];
                let mut outputs = vec![];
                for (rid, q) in &r.outputs {
                    out_res.push(res(rid)?);
                    outputs.push(fx(*q));
                }
                let cycle = ps.add(
                    key("cycle"),
                    label("cycle time"),
                    grp,
                    Unit::Ticks,
                    (r.cycle_secs * TICKS_PER_SEC as f64).round() as i64,
                    (1, 36_000),
                    (1, 10),
                );
                (
                    Kind::Processor(RecipeSpec {
                        inputs,
                        outputs,
                        batch: r.batch.max(1) as i64,
                    }),
                    in_res,
                    out_res,
                    None,
                    None,
                    Some(cycle),
                )
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
            p: NodeParams {
                enabled,
                efficiency,
                workers,
                in_cap,
                out_cap,
                rate,
                variance,
                cycle,
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
        let label = |s: &str| format!("{}->{}: {s}", nodes[from].name, nodes[to].name);
        let grp = Group::Link(li);
        let p = LinkParams {
            carriers: ps.add(
                key("carriers"),
                label("carriers"),
                grp,
                Unit::Count,
                ld.carriers as i64,
                (0, 100),
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

    let order = topo_order(nodes.len(), &links);
    Ok((
        Chain {
            name: cfg.name.clone(),
            resources,
            nodes,
            links,
            order,
            g,
        },
        ps,
    ))
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
