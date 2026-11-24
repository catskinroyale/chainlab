//! # Logistics
//!
//! Goods don't teleport. Each link has a number of **carriers**, a **load
//! capacity** per carrier and a **distance** (one-way travel time `T`). A
//! carrier's life is a loop:
//!
//! ```text
//! load at source --(T)--> unload at destination --(T)--> back, free again
//! ```
//!
//! So a single carrier moves at most `capacity / 2T` units per tick, and a
//! link's ceiling is
//!
//! ```text
//! max throughput = carriers * capacity / (2 * distance)
//! ```
//!
//! Doubling the distance halves throughput unless you double the carriers.
//! That's the latency/capacity trade-off at the heart of logistics games.
//!
//! The dispatcher is smart but patient:
//! * It only ships what the destination will have room for, counting goods
//!   already on the road. A full destination stops shipments, the source's
//!   output buffer fills, and the source goes **Blocked** (backpressure).
//! * It waits for a full load unless the destination's room or the source's
//!   buffer size is the smaller limit, or the source has stopped producing.
//! * If goods arrive and the destination has filled up meanwhile, the
//!   carrier waits there, still busy, until it can unload.
//!
//! Links are processed in config order, so when several links leave the same
//! buffer the earlier link gets first pick. The default chain uses this to
//! feed the builder first and send only the surplus to market.

use crate::fixed::Fx;

use super::chain::Chain;
use super::model::{Shipment, State, Status};

pub fn tick(chain: &Chain, eff: &[i64], st: &mut State) {
    let now = st.tick;
    let State { nodes, links, .. } = st;
    for (li, link) in chain.links.iter().enumerate() {
        let ls = &mut links[li];
        ls.delivered_tick = 0;
        let carriers = eff[link.p.carriers].max(0) as usize;
        let load_cap = eff[link.p.capacity].max(0);
        let travel = eff[link.p.distance].max(1) as u64;
        let to_cap = chain.nodes[link.to].p.in_cap.map_or(0, |p| eff[p]);
        let from_cap = chain.nodes[link.from].p.out_cap.map_or(0, |p| eff[p]);

        // 1. Unload carriers that have arrived.
        for s in ls.shipments.iter_mut() {
            if s.outbound() && now >= s.arrive {
                let buf = &mut nodes[link.to].inputs[link.to_slot];
                let space = (to_cap - *buf).max(0);
                let d = s.qty.min(space);
                *buf += d;
                s.qty -= d;
                ls.delivered_total += d;
                ls.delivered_tick += d;
                if s.qty == 0 {
                    s.return_at = Some(now + travel);
                }
            }
        }

        // 2. Carriers that made it home are free again.
        ls.shipments.retain(|s| s.return_at.is_none_or(|r| now < r));

        // 3. Dispatch free carriers.
        let mut in_transit: Fx = ls.in_transit();
        let free = carriers.saturating_sub(ls.shipments.len());
        let source_idle = nodes[link.from].status != Status::Working;
        for _ in 0..free {
            let avail = nodes[link.from].outputs[link.from_slot];
            let dest_room = to_cap - nodes[link.to].inputs[link.to_slot] - in_transit;
            let target = load_cap.min(dest_room).min(from_cap);
            if target <= 0 || avail <= 0 {
                break;
            }
            if avail < target && !source_idle {
                break; // wait for a full load
            }
            let load = avail.min(target);
            nodes[link.from].outputs[link.from_slot] -= load;
            in_transit += load;
            ls.shipments.push(Shipment {
                qty: load,
                depart: now,
                arrive: now + travel,
                return_at: None,
            });
        }
    }
}

/// Theoretical link ceiling in fixed-point units per second.
pub fn max_rate_per_sec(carriers: i64, capacity: Fx, distance_ticks: i64) -> Fx {
    let round_trip = (2 * distance_ticks.max(1)) as i128;
    ((carriers.max(0) as i128 * capacity as i128 * crate::TICKS_PER_SEC as i128) / round_trip) as Fx
}
