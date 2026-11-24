//! # Statistics
//!
//! Everything you see in the stats panel is measured over a rolling
//! **window** of the last 60 simulated seconds (600 ticks):
//!
//! * **Throughput** = primary output produced in the window, scaled to
//!   units per minute. Early in a run (window not yet full) it's scaled up
//!   from the ticks seen so far.
//! * **Utilisation** = the share of window ticks spent in each [`Status`].
//!   Working% is "true" utilisation; Starved% and Blocked% tell you which
//!   side of the node the problem is on.
//!
//! Sums are kept incrementally in integers, so they're exact and cheap.

use std::collections::VecDeque;

use crate::TICKS_PER_SEC;
use crate::fixed::Fx;

use super::model::{State, Status};

/// Rolling window length in ticks.
pub const WINDOW: usize = 60 * TICKS_PER_SEC as usize;

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
    pub fn iter(&self) -> impl Iterator<Item = &i64> {
        self.buf.iter()
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
        if self.out.is_empty() {
            return 0;
        }
        self.out.sum() * WINDOW as i64 / self.out.len() as i64
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stats {
    pub nodes: Vec<NodeStats>,
}

impl Stats {
    pub fn new(n_nodes: usize) -> Self {
        Stats {
            nodes: (0..n_nodes).map(|_| NodeStats::new()).collect(),
        }
    }
}

/// Record this tick's node statuses and output.
pub fn record(st: &mut State) {
    let State { nodes, stats, .. } = st;
    for (ns, s) in nodes.iter().zip(stats.nodes.iter_mut()) {
        s.push(ns.status, ns.out_tick);
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
    }
}
