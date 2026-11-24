//! # Bottleneck detection and the efficiency score
//!
//! ## Bottleneck
//!
//! A chain can only go as fast as its slowest stage. Every stage has a
//! theoretical **capacity** (what it would do if never starved or blocked)
//! and an **actual** rate over the last minute. Their ratio is the stage's
//! *load*:
//!
//! ```text
//! load = actual per minute / capacity per minute
//! ```
//!
//! The bottleneck is the stage with the highest load **on the path that
//! limits the end of the chain**:
//!
//! 1. Start at the sink (the Tower). It consumes *sets* of inputs, so find
//!    the input with the fewest complete sets in its buffer and on the road.
//!    That branch is the one holding the Tower back. The other branch's
//!    surplus piles up or goes to market.
//! 2. Candidates are the sink itself plus every node and link upstream of
//!    that scarce input.
//! 3. The highest-load candidate wins. Ties (within 3 points) go to the
//!    stage nearest the sink: if the Tower's lift is maxed, faster kilns
//!    won't help.
//! 4. A node that is Off or has no crew while work is waiting has load 1.0.
//!    Idle is the tightest constraint of all.
//!
//! A link's load is the goods it keeps on the road vs the most it could
//! (carts x load / 2): half-empty carts count half, and carts waiting to
//! unload at a full destination don't count (that's the receiver's fault).
//!
//! If nothing is above 85% load the chain has slack everywhere and no single
//! bottleneck is reported. Supporting evidence is the classic status
//! pattern: feeders **Blocked** (backed up behind it) and consumers
//! **Starved** (waiting on it).
//!
//! ## Efficiency score (0-100)
//!
//! ```text
//! score = 100 * (0.4 * utilisation + 0.3 * workforce + 0.3 * margin)
//! ```
//!
//! * **utilisation**: mean Working share of active production nodes
//!   (market excluded: an idle market just means no surplus).
//! * **workforce**: share of the worker pool that's on a node *and* that
//!   node is working (idle, unassigned, or stuck workers all count against).
//! * **margin**: operating profit / income over the window, clamped to 0..1.
//!
//! A perfectly balanced, fully staffed chain with low upkeep scores near 100.

use crate::fixed::{Fx, to_f64};

use super::chain::{Chain, Kind, LinkId, NodeId, ResId};
use super::logistics::max_rate_per_sec;
use super::model::{State, Status};
use super::node_capacity_per_min;

/// Minimum load to call something a bottleneck.
pub const THRESHOLD: f64 = 0.85;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limiter {
    Node(NodeId),
    Link(LinkId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bottleneck {
    pub at: Limiter,
    /// Load 0..1 (actual / capacity).
    pub score: f64,
    /// Distance from the sink (0 = the sink), for tie-breaks.
    pub depth: usize,
    pub reason: String,
}

fn link_capacity_per_min(chain: &Chain, eff: &[i64], l: LinkId) -> Fx {
    let p = &chain.links[l].p;
    max_rate_per_sec(eff[p.carriers], eff[p.capacity], eff[p.distance]) * 60
}

fn node_load(chain: &Chain, eff: &[i64], st: &State, n: NodeId) -> f64 {
    let s = &st.stats.nodes[n];
    if s.share(Status::Off) + s.share(Status::Unstaffed) > 0.5 && eff[chain.nodes[n].p.enabled] == 0
        || s.share(Status::Unstaffed) > 0.5
    {
        return 1.0;
    }
    let cap = node_capacity_per_min(chain, eff, n);
    if cap <= 0 {
        return 1.0;
    }
    (to_f64(s.per_min()) / to_f64(cap)).clamp(0.0, 1.0)
}

/// The sink input that is scarcest: fewest complete sets in its buffer plus
/// on the road. (Delivery *rates* can't tell: a full buffer throttles the
/// plentiful input down to the consumption rate too.)
fn scarce_input(chain: &Chain, st: &State, sink: NodeId) -> Option<ResId> {
    let Kind::Sink(spec) = &chain.nodes[sink].kind else {
        return None;
    };
    let node = &chain.nodes[sink];
    let mut best: Option<(f64, ResId)> = None;
    for (slot, &r) in node.in_res.iter().enumerate() {
        let transit: i64 = chain
            .inbound(sink)
            .filter(|(_, l)| l.to_slot == slot)
            .map(|(li, _)| st.links[li].in_transit())
            .sum();
        let have = st.nodes[sink].inputs[slot] + transit;
        let sets = to_f64(have) / to_f64(spec.inputs[slot]).max(1e-9);
        if best.is_none_or(|(b, _)| sets < b) {
            best = Some((sets, r));
        }
    }
    best.map(|(_, r)| r)
}

/// Every candidate on the limiting path with its load, best first.
pub fn candidates(chain: &Chain, eff: &[i64], st: &State) -> Vec<Bottleneck> {
    let n = chain.nodes.len();
    let active = |i: NodeId| chain.nodes[i].tier <= st.tier;
    // depth from the sink along the limiting path; usize::MAX = not on it
    let mut node_depth = vec![usize::MAX; n];
    let mut link_depth = vec![usize::MAX; chain.links.len()];
    let sinks: Vec<NodeId> = chain.sinks().into_iter().filter(|&s| active(s)).collect();
    if sinks.is_empty() {
        for (i, d) in node_depth.iter_mut().enumerate() {
            if active(i) {
                *d = 0;
            }
        }
        link_depth.iter_mut().for_each(|d| *d = 0);
    }
    for &sink in &sinks {
        node_depth[sink] = 0;
        let scarce = scarce_input(chain, st, sink);
        let mut frontier: Vec<NodeId> = vec![];
        for (li, l) in chain.inbound(sink) {
            if scarce.is_none_or(|r| r == l.res) && active(l.from) {
                link_depth[li] = 1;
                if node_depth[l.from] > 2 {
                    node_depth[l.from] = 2;
                    frontier.push(l.from);
                }
            }
        }
        while let Some(x) = frontier.pop() {
            let d = node_depth[x];
            for (li, l) in chain.inbound(x) {
                if !active(l.from) {
                    continue;
                }
                link_depth[li] = link_depth[li].min(d + 1);
                if node_depth[l.from] > d + 2 {
                    node_depth[l.from] = d + 2;
                    frontier.push(l.from);
                }
            }
        }
    }

    let s = &st.stats.nodes;
    let mut out = vec![];
    for i in 0..n {
        if node_depth[i] == usize::MAX {
            continue;
        }
        let load = node_load(chain, eff, st, i);
        let ups: Vec<f64> = chain
            .inbound(i)
            .filter(|(_, l)| active(l.from))
            .map(|(_, l)| s[l.from].share(Status::Blocked))
            .collect();
        let downs: Vec<f64> = chain
            .outbound(i)
            .filter(|(_, l)| active(l.to) && !matches!(chain.nodes[l.to].kind, Kind::Market))
            .map(|(_, l)| s[l.to].share(Status::Starved))
            .collect();
        let mode = if eff[chain.nodes[i].p.enabled] == 0 {
            "is OFF"
        } else if s[i].share(Status::Unstaffed) > 0.5 {
            "has NO CREW"
        } else {
            "at"
        };
        let mut reason = format!(
            "{} {mode} {:.0}% of capacity ({:.1}/{:.1} per min)",
            chain.nodes[i].name,
            load * 100.0,
            to_f64(s[i].per_min()),
            to_f64(node_capacity_per_min(chain, eff, i))
        );
        if !ups.is_empty() {
            reason += &format!(" | feeders blocked {:.0}%", mean(&ups) * 100.0);
        }
        if !downs.is_empty() {
            reason += &format!(" | consumers starved {:.0}%", mean(&downs) * 100.0);
        }
        out.push(Bottleneck {
            at: Limiter::Node(i),
            score: load,
            depth: node_depth[i],
            reason,
        });
    }
    for (li, l) in chain.links.iter().enumerate() {
        if link_depth[li] == usize::MAX {
            continue;
        }
        let cap = link_capacity_per_min(chain, eff, li);
        let actual = st.stats.links[li].delivered.per_window();
        let load = if eff[l.p.carriers] <= 0 {
            1.0
        } else {
            st.stats.links[li].load()
        };
        out.push(Bottleneck {
            at: Limiter::Link(li),
            score: load,
            depth: link_depth[li],
            reason: format!(
                "Haul {} at {:.0}% of capacity ({:.1}/{:.1} per min) | sender blocked {:.0}% | receiver starved {:.0}%",
                chain.link_label(li),
                load * 100.0,
                to_f64(actual),
                to_f64(cap),
                s[l.from].share(Status::Blocked) * 100.0,
                s[l.to].share(Status::Starved) * 100.0
            ),
        });
    }
    // Highest load first; within 3 points, nearest the sink first.
    out.sort_by(|a, b| {
        let (qa, qb) = (
            (a.score * 100.0 / 3.0).round(),
            (b.score * 100.0 / 3.0).round(),
        );
        qb.total_cmp(&qa).then(a.depth.cmp(&b.depth))
    });
    out
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// The current bottleneck, if one stands out.
pub fn bottleneck(chain: &Chain, eff: &[i64], st: &State) -> Option<Bottleneck> {
    // Need some history before judging.
    if st.stats.nodes.first().is_none_or(|n| n.out.len() < 300) {
        return None;
    }
    candidates(chain, eff, st)
        .into_iter()
        .next()
        .filter(|b| b.score >= THRESHOLD)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Efficiency {
    pub score: u32,
    pub utilisation: f64,
    pub workforce: f64,
    pub margin: f64,
}

pub fn efficiency(chain: &Chain, eff: &[i64], st: &State) -> Efficiency {
    let s = &st.stats.nodes;
    let prod: Vec<NodeId> = (0..chain.nodes.len())
        .filter(|&n| {
            !matches!(chain.nodes[n].kind, Kind::Market)
                && chain.nodes[n].tier <= st.tier
                && eff[chain.nodes[n].p.enabled] != 0
        })
        .collect();
    let utilisation = if prod.is_empty() {
        0.0
    } else {
        prod.iter()
            .map(|&n| s[n].share(Status::Working))
            .sum::<f64>()
            / prod.len() as f64
    };
    let pool = eff[chain.g.workers_total];
    let workforce = if pool <= 0 {
        1.0
    } else {
        let used: f64 = chain
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(n, node)| {
                node.p
                    .workers
                    .map(|w| eff[w] as f64 * s[n].share(Status::Working))
            })
            .sum();
        (used / pool as f64).clamp(0.0, 1.0)
    };
    let income = to_f64(st.stats.income_per_min());
    let margin = if income <= 0.0 {
        0.0
    } else {
        (to_f64(st.stats.operating_per_min()) / income).clamp(0.0, 1.0)
    };
    let score = (100.0 * (0.4 * utilisation + 0.3 * workforce + 0.3 * margin)).round() as u32;
    Efficiency {
        score,
        utilisation,
        workforce,
        margin,
    }
}
