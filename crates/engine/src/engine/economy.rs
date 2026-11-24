//! # Escalating cost, upkeep and purchases
//!
//! Every upgrade gets more expensive the more of it you own:
//!
//! ```text
//! cost(n) = base * growth ^ n * (1 - discount%)
//! ```
//!
//! With growth 1.15 the 10th level costs about 4x the first and the 20th
//! about 16x. Exponential costs against (at best) linear gains give every
//! idle/tycoon game its rhythm: each upgrade's **payback time** keeps
//! growing until something else (a new tier, a different upgrade) becomes
//! the better buy. The *growth* lever shows how hard that wall hits, and the
//! *bulk discount* lever how much a cost reduction pushes it back.
//!
//! **Upkeep** is the running cost: every worker in the pool costs Coin per
//! minute whether they're assigned or idle, so hiring is only worth it if
//! the extra crew produces more than it costs.

use crate::fixed::{Fx, fx, to_f64};

use super::chain::Chain;
use super::model::State;
use super::stats::WINDOW;

/// Cost of the next level when `owned` are already owned.
pub fn cost(base: Fx, growth: Fx, owned: u32, discount_pct: i64) -> Fx {
    let d = 1.0 - discount_pct.clamp(0, 100) as f64 / 100.0;
    fx(to_f64(base) * to_f64(growth).powi(owned as i32) * d)
}

/// Costs of the next `n` levels starting from `owned`.
pub fn cost_curve(base: Fx, growth: Fx, owned: u32, n: u32, discount_pct: i64) -> Vec<Fx> {
    (0..n)
        .map(|k| cost(base, growth, owned + k, discount_pct))
        .collect()
}

/// Why an upgrade can't be bought right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuyBlock {
    Locked(u32),
    MaxLevel,
    TooExpensive(Fx),
}

/// Current price of upgrade `u`.
pub fn upgrade_cost(chain: &Chain, eff: &[i64], st: &State, u: usize) -> Fx {
    let up = &chain.upgrades[u];
    cost(
        eff[up.base_cost],
        eff[up.growth],
        st.upgrades[u],
        eff[chain.g.discount],
    )
}

/// Check whether upgrade `u` can be bought; returns its cost if so.
pub fn check_buy(chain: &Chain, eff: &[i64], st: &State, u: usize) -> Result<Fx, BuyBlock> {
    let up = &chain.upgrades[u];
    if up.tier > st.tier {
        return Err(BuyBlock::Locked(up.tier));
    }
    if up.max_level.is_some_and(|m| st.upgrades[u] >= m) {
        return Err(BuyBlock::MaxLevel);
    }
    let c = upgrade_cost(chain, eff, st, u);
    if st.coin < c {
        return Err(BuyBlock::TooExpensive(c - st.coin));
    }
    Ok(c)
}

/// Pay worker upkeep for one tick. Exact: sub-milli remainders carry over.
pub fn upkeep(chain: &Chain, eff: &[i64], st: &mut State) {
    let pool = eff[chain.g.workers_total].max(0);
    let per_min = eff[chain.g.upkeep].max(0);
    let l = &mut st.ledger;
    l.upkeep_acc += pool * per_min;
    let pay = l.upkeep_acc / WINDOW as i64;
    l.upkeep_acc %= WINDOW as i64;
    l.upkeep_tick = pay;
    l.upkeep_total += pay;
    st.coin -= pay;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escalating_cost_formula() {
        assert_eq!(cost(fx(100.0), fx(1.15), 0, 0), fx(100.0));
        assert_eq!(cost(fx(100.0), fx(1.15), 1, 0), fx(115.0));
        assert_eq!(cost(fx(100.0), fx(1.15), 2, 0), fx(132.25));
        assert_eq!(cost(fx(100.0), fx(1.15), 2, 20), fx(105.8));
        assert_eq!(cost(fx(10.0), fx(2.0), 10, 0), fx(10_240.0));
    }

    #[test]
    fn cost_curve_is_monotonic() {
        let c = cost_curve(fx(50.0), fx(1.2), 3, 10, 0);
        assert_eq!(c.len(), 10);
        assert!(c.windows(2).all(|w| w[1] > w[0]));
        assert_eq!(c[0], cost(fx(50.0), fx(1.2), 3, 0));
    }
}
