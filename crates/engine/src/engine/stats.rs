//! # Statistics
//!
//! Everything in the stats panel is measured over a rolling **window** of the
//! last 60 simulated seconds (600 ticks):
//!
//! * **Throughput** = output produced in the window, as units per minute.
//!   Early in a run (window not yet full) it's scaled up from the ticks seen
//!   so far.
//! * **Utilisation** = the share of window ticks spent in each [`Status`].
//!   Working% is "true" utilisation. Starved% and Blocked% tell you which
//!   side of the node the problem is on.
//! * **Link load** = average share of carriers busy.
//! * **Money**: income, upkeep and purchases per minute.
//!
//! Once per simulated second a sample goes into the **history** rings (last
//! 120 s) that draw the sparklines.
//!
//! Sums are kept incrementally in integers, so they're exact and cheap.

use std::collections::VecDeque;

use crate::TICKS_PER_SEC;
use crate::fixed::Fx;

use super::chain::Chain;
use super::model::{State, Status};

/// Rolling window length in ticks (one simulated minute).
pub const WINDOW: usize = 60 * TICKS_PER_SEC as usize;
/// History length in samples (one per simulated second).
pub const HISTORY: usize = 120;

/// Fixed-capacity ring of integers with a running sum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ring {
    buf: VecDeque<i64>,
    cap: usize,
    sum: i64,
}

impl Ring {
    pub fn new(cap: usize) -> Self {
        Ring {
            buf: VecDeque::with_capacity(cap),
            cap,
            sum: 0,
        }
    }
    pub fn push(&mut self, v: i64) {
        if self.buf.len() == self.cap
            && let Some(old) = self.buf.pop_front()
        {
            self.sum -= old;
        }
        self.buf.push_back(v);
        self.sum += v;
    }
    pub fn sum(&self) -> i64 {
        self.sum
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    pub fn last(&self) -> Option<i64> {
        self.buf.back().copied()
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &i64> + ExactSizeIterator {
        self.buf.iter()
    }
    /// Sum scaled to a full window: "per minute" for per-tick rings.
    pub fn per_window(&self) -> i64 {
        if self.buf.is_empty() {
            0
        } else {
            self.sum * self.cap as i64 / self.buf.len() as i64
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeStats {
    pub out: Ring,
    status: VecDeque<Status>,
    /// Ticks per status inside the window.
    pub window_counts: [u32; 5],
    /// Ticks per status since start.
    pub totals: [u64; 5],
    pub out_total: Fx,
}

impl NodeStats {
    fn new() -> Self {
        NodeStats {
            out: Ring::new(WINDOW),
            status: VecDeque::with_capacity(WINDOW),
            window_counts: [0; 5],
            totals: [0; 5],
            out_total: 0,
        }
    }

    fn push(&mut self, s: Status, out: Fx) {
        if self.status.len() == WINDOW
            && let Some(old) = self.status.pop_front()
        {
            self.window_counts[old.idx()] -= 1;
        }
        self.status.push_back(s);
        self.window_counts[s.idx()] += 1;
        self.totals[s.idx()] += 1;
        self.out.push(out);
        self.out_total += out;
    }

    /// Primary output per simulated minute over the window (fixed point).
    pub fn per_min(&self) -> Fx {
        self.out.per_window()
    }

    /// Share of the window spent in `s`, 0.0..=1.0.
    pub fn share(&self, s: Status) -> f64 {
        let n = self.status.len();
        if n == 0 {
            0.0
        } else {
            self.window_counts[s.idx()] as f64 / n as f64
        }
    }

    /// Statuses over the last `n` ticks, oldest first.
    pub fn recent(&self, n: usize) -> impl Iterator<Item = &Status> {
        self.status.iter().skip(self.status.len().saturating_sub(n))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkStats {
    pub busy: Ring,
    pub carriers: Ring,
    pub delivered: Ring,
}

impl LinkStats {
    /// Average share of carriers busy over the window, 0.0..=1.0.
    pub fn load(&self) -> f64 {
        if self.carriers.sum() == 0 {
            0.0
        } else {
            self.busy.sum() as f64 / self.carriers.sum() as f64
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoodStats {
    pub sold: Ring,
    pub revenue: Ring,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    pub coin: Ring,
    pub net: Ring,
    pub progress: Ring,
    pub nodes: Vec<Ring>,
    pub prices: Vec<Ring>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stats {
    pub nodes: Vec<NodeStats>,
    pub links: Vec<LinkStats>,
    pub goods: Vec<GoodStats>,
    pub income: Ring,
    pub upkeep: Ring,
    pub purchases: Ring,
    pub progress: Ring,
    pub history: History,
}

impl Stats {
    pub fn new(chain: &Chain) -> Self {
        let n_goods = chain.market.as_ref().map_or(0, |m| m.goods.len());
        Stats {
            nodes: (0..chain.nodes.len()).map(|_| NodeStats::new()).collect(),
            links: (0..chain.links.len())
                .map(|_| LinkStats {
                    busy: Ring::new(WINDOW),
                    carriers: Ring::new(WINDOW),
                    delivered: Ring::new(WINDOW),
                })
                .collect(),
            goods: (0..n_goods)
                .map(|_| GoodStats {
                    sold: Ring::new(WINDOW),
                    revenue: Ring::new(WINDOW),
                })
                .collect(),
            income: Ring::new(WINDOW),
            upkeep: Ring::new(WINDOW),
            purchases: Ring::new(WINDOW),
            progress: Ring::new(WINDOW),
            history: History {
                coin: Ring::new(HISTORY),
                net: Ring::new(HISTORY),
                progress: Ring::new(HISTORY),
                nodes: (0..chain.nodes.len()).map(|_| Ring::new(HISTORY)).collect(),
                prices: (0..n_goods).map(|_| Ring::new(HISTORY)).collect(),
            },
        }
    }

    pub fn income_per_min(&self) -> Fx {
        self.income.per_window()
    }
    pub fn upkeep_per_min(&self) -> Fx {
        self.upkeep.per_window()
    }
    pub fn purchases_per_min(&self) -> Fx {
        self.purchases.per_window()
    }
    /// Expenses per minute: upkeep + purchases.
    pub fn expenses_per_min(&self) -> Fx {
        self.upkeep_per_min() + self.purchases_per_min()
    }
    /// Net cash flow per minute (income - upkeep - purchases).
    pub fn net_per_min(&self) -> Fx {
        self.income_per_min() - self.expenses_per_min()
    }
    /// Operating profit per minute (income - upkeep), ignoring investments.
    pub fn operating_per_min(&self) -> Fx {
        self.income_per_min() - self.upkeep_per_min()
    }
    /// Sink progress per minute.
    pub fn progress_per_min(&self) -> Fx {
        self.progress.per_window()
    }
}

/// Record this tick into the rolling windows (and history once a second).
pub fn record(chain: &Chain, eff: &[i64], st: &mut State) {
    let State {
        nodes,
        links,
        stats,
        ledger,
        market,
        tick,
        coin,
        ..
    } = st;
    let mut progress = 0;
    for (i, (ns, s)) in nodes.iter().zip(stats.nodes.iter_mut()).enumerate() {
        s.push(ns.status, ns.out_tick);
        if matches!(chain.nodes[i].kind, super::chain::Kind::Sink(_)) {
            progress += ns.out_tick;
        }
    }
    for (li, (ls, s)) in links.iter().zip(stats.links.iter_mut()).enumerate() {
        s.busy.push(ls.shipments.len() as i64);
        s.carriers.push(eff[chain.links[li].p.carriers].max(0));
        s.delivered.push(ls.delivered_tick);
    }
    for (g, s) in market.iter().zip(stats.goods.iter_mut()) {
        s.sold.push(g.sold_tick);
        s.revenue.push(g.revenue_tick);
    }
    stats.income.push(ledger.income_tick);
    stats.upkeep.push(ledger.upkeep_tick);
    stats.purchases.push(ledger.purchase_tick);
    stats.progress.push(progress);

    if (*tick + 1) % TICKS_PER_SEC == 0 {
        let net = stats.net_per_min();
        let prog = stats.progress_per_min();
        let h = &mut stats.history;
        h.coin.push(*coin);
        h.net.push(net);
        h.progress.push(prog);
        for (r, s) in h.nodes.iter_mut().zip(stats.nodes.iter()) {
            r.push(s.per_min());
        }
        for (r, g) in h.prices.iter_mut().zip(market.iter()) {
            r.push(g.price);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_running_sum() {
        let mut r = Ring::new(3);
        for v in [1, 2, 3, 4] {
            r.push(v);
        }
        assert_eq!(r.sum(), 9);
        assert_eq!(r.len(), 3);
        assert_eq!(r.per_window(), 9);
    }

    #[test]
    fn per_window_scales_partial_rings() {
        let mut r = Ring::new(10);
        r.push(2);
        r.push(2);
        assert_eq!(r.per_window(), 20);
    }
}
