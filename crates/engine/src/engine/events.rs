//! # Random events
//!
//! Shocks that test how robust your chain is. When enabled, each tick has a
//! `frequency / 600` chance of starting one (so `frequency` is the expected
//! number per simulated minute). **Severity** (0-100%) scales how hard
//! they hit:
//!
//! * **Drought**: water supply cut by severity% for 30-60 s. The forest
//!   regrows slower; if you were harvesting near the limit, it collapses.
//! * **Price crash**: one good's price drift drops by up to 70%, then
//!   recovers on its own. Test the "above threshold" sell policy here.
//! * **Breakdown**: a random processor stops for 15-45 s. Buffers decide
//!   whether the rest of the chain notices.
//! * **Windfall**: a one-off Coin bonus.
//!
//! The event stream has its own random sequence, so toggling events doesn't
//! change yields or prices.

use crate::TICKS_PER_SEC;
use crate::fixed::{Fx, fx};

use super::chain::{Chain, Kind};
use super::market::crash_drift;
use super::model::{ActiveEvent, EventKind, LogKind, State};

/// Expire finished events and maybe roll a new one. Returns true if the
/// set of parameter-affecting events changed.
pub fn tick(chain: &Chain, eff: &[i64], st: &mut State) -> bool {
    let now = st.tick;
    let mut dirty = false;
    let mut ended = vec![];
    st.events.retain(|e| {
        let keep = e.end > now;
        if !keep {
            ended.push(e.kind);
        }
        keep
    });
    for k in ended {
        dirty |= matches!(k, EventKind::Drought | EventKind::Breakdown);
        st.log(LogKind::Event, format!("{} over", k.name()));
    }

    let r = &mut st.rng.events;
    let (roll, which, pick, dur) = (r.unit(), r.unit(), r.unit(), r.unit());
    if eff[chain.g.ev_enabled] != 0 {
        let p = crate::fixed::to_f64(eff[chain.g.ev_frequency]) / (60.0 * TICKS_PER_SEC as f64);
        if roll < p {
            let kind = EventKind::ALL[((which * 4.0) as usize).min(3)];
            dirty |= spawn(chain, st, kind, eff[chain.g.ev_severity], pick, dur);
        }
    }
    dirty
}

/// Start an event. `pick` chooses the target, `dur` the duration (both 0..1).
/// Returns true if effective parameters need refreshing.
pub fn spawn(
    chain: &Chain,
    st: &mut State,
    kind: EventKind,
    severity: i64,
    pick: f64,
    dur: f64,
) -> bool {
    let now = st.tick;
    let secs = |lo: f64, hi: f64| ((lo + (hi - lo) * dur) * TICKS_PER_SEC as f64) as u64;
    let sev = severity.clamp(0, 100);
    match kind {
        EventKind::Drought => {
            st.events.push(ActiveEvent {
                kind,
                target: None,
                severity: sev,
                start: now,
                end: now + secs(30.0, 60.0),
            });
            st.log(LogKind::Event, format!("DROUGHT: water -{sev}%"));
            true
        }
        EventKind::PriceCrash => {
            let n = st.market.len();
            if n == 0 {
                return false;
            }
            let k = ((pick * n as f64) as usize).min(n - 1);
            st.market[k].drift = crash_drift(st.market[k].drift, sev);
            st.events.push(ActiveEvent {
                kind,
                target: Some(k),
                severity: sev,
                start: now,
                end: now + secs(20.0, 40.0),
            });
            let name = chain
                .market
                .as_ref()
                .map_or("?", |m| chain.res_name(m.goods[k].res));
            st.log(LogKind::Event, format!("PRICE CRASH: {name} (sev {sev}%)"));
            false
        }
        EventKind::Breakdown => {
            let procs: Vec<usize> = (0..chain.nodes.len())
                .filter(|&i| {
                    matches!(chain.nodes[i].kind, Kind::Processor(_))
                        && chain.nodes[i].tier <= st.tier
                })
                .collect();
            if procs.is_empty() {
                return false;
            }
            let n = procs[((pick * procs.len() as f64) as usize).min(procs.len() - 1)];
            // Severity stretches the outage.
            let len = secs(15.0, 45.0) * (50 + sev as u64) / 100;
            st.events.push(ActiveEvent {
                kind,
                target: Some(n),
                severity: sev,
                start: now,
                end: now + len.max(TICKS_PER_SEC),
            });
            st.log(
                LogKind::Event,
                format!("BREAKDOWN: {}", chain.nodes[n].name),
            );
            true
        }
        EventKind::Windfall => {
            let amount: Fx = fx(10.0 + sev as f64 * 1.5) * (1 + st.tier as i64);
            st.coin += amount;
            st.ledger.income_tick += amount;
            st.ledger.income_total += amount;
            st.log(
                LogKind::Event,
                format!("WINDFALL: +{:.0}c", crate::fixed::to_f64(amount)),
            );
            false
        }
    }
}
