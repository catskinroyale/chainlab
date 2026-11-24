//! # Market
//!
//! Surplus goods are sold for Coin. Each good has its own price that moves
//! for three reasons:
//!
//! 1. **Demand pressure.** Every unit sold adds to a "pressure" number, and
//!    the price falls as pressure rises:
//!
//!    ```text
//!    price = base_price * drift / (1 + elasticity * pressure)
//!    ```
//!
//!    Dumping 50 units at once with elasticity 0.02 halves the price.
//! 2. **Recovery.** Pressure fades by `recovery` (a share) every second, so
//!    prices climb back if you stop selling. Selling steadily beats selling
//!    in bursts.
//! 3. **Drift.** A random walk (strength = `volatility`) that is gently
//!    pulled back towards 1.0. Price crashes knock drift down and it recovers
//!    the same way.
//!
//! The **sell policy** decides how much of the market's stock is offered:
//! *sell all*, *above threshold* (only while price >= threshold% of base:
//! the market shows as Blocked/HOLDING otherwise) or *keep reserve*
//! (never sell the last X units). The market can absorb at most `sell_rate`
//! units of each good per second.

use crate::TICKS_PER_SEC;
use crate::fixed::{Fx, ONE, fx, mul, to_f64};

use super::chain::Chain;
use super::model::{State, Status};

/// Price from base, drift and pressure (all fixed point).
pub fn price(base: Fx, drift: Fx, elasticity: Fx, pressure: Fx) -> Fx {
    fx(to_f64(base) * to_f64(drift) / (1.0 + to_f64(elasticity) * to_f64(pressure)))
}

/// Pressure after one tick of recovery.
pub fn recover(pressure: Fx, recovery_per_sec: Fx) -> Fx {
    if pressure <= 0 {
        return 0;
    }
    let drop = fx(to_f64(pressure) * to_f64(recovery_per_sec) / TICKS_PER_SEC as f64).max(1);
    (pressure - drop).max(0)
}

pub fn tick(chain: &Chain, eff: &[i64], st: &mut State) {
    let Some(m) = &chain.market else {
        return;
    };
    let node = &chain.nodes[m.node];
    let enabled = eff[node.p.enabled] != 0;
    let policy = eff[m.policy];
    let threshold = eff[m.threshold];
    let reserve = eff[m.reserve];
    let per_tick = eff[m.sell_rate] / TICKS_PER_SEC as i64;

    let (mut sold_any, mut holding) = (false, false);
    let mut sold_qty = 0;
    for (k, good) in m.goods.iter().enumerate() {
        // Always draw so the stream stays aligned whatever happens.
        let u = st.rng.market.signed();
        let base = eff[good.base_price];
        let gs = &mut st.market[k];
        gs.sold_tick = 0;
        gs.revenue_tick = 0;

        // Drift: mean-reverting random walk, clamped to a sane band.
        let vol = eff[good.volatility] as f64 / 100.0;
        let d = to_f64(gs.drift);
        let d = d + (1.0 - d) * 0.03 / TICKS_PER_SEC as f64 + vol * 0.03 * u;
        gs.drift = fx(d.clamp(0.2, 3.0));

        gs.pressure = recover(gs.pressure, eff[good.recovery]);
        gs.price = price(base, gs.drift, eff[good.elasticity], gs.pressure);

        let stock = st.nodes[m.node].inputs[k];
        let allowed = match policy {
            1 => {
                if gs.price as i128 * 100 >= base as i128 * threshold as i128 {
                    stock
                } else {
                    0
                }
            }
            2 => (stock - reserve).max(0),
            _ => stock,
        };
        if stock > 0 && allowed == 0 {
            holding = true;
        }
        let q = allowed.min(per_tick);
        if enabled && q > 0 {
            let revenue = mul(q, gs.price);
            st.nodes[m.node].inputs[k] -= q;
            gs.sold_tick = q;
            gs.revenue_tick = revenue;
            gs.sold_total += q;
            gs.revenue_total += revenue;
            gs.pressure += q;
            st.coin += revenue;
            st.ledger.income_tick += revenue;
            st.ledger.income_total += revenue;
            sold_qty += q;
            sold_any = true;
        }
    }
    let ns = &mut st.nodes[m.node];
    ns.out_tick = sold_qty;
    ns.status = if !enabled {
        Status::Off
    } else if sold_any {
        Status::Working
    } else if holding {
        Status::Blocked
    } else {
        Status::Starved
    };
}

/// Drift multiplier applied by a price crash of `severity`%.
pub fn crash_drift(drift: Fx, severity: i64) -> Fx {
    let keep = 1.0 - 0.7 * (severity.clamp(0, 100) as f64 / 100.0);
    fx(to_f64(drift) * keep).max(ONE / 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_falls_with_pressure() {
        let base = fx(10.0);
        assert_eq!(price(base, ONE, fx(0.02), 0), fx(10.0));
        assert_eq!(price(base, ONE, fx(0.02), fx(50.0)), fx(5.0));
        assert!(price(base, ONE, fx(0.02), fx(100.0)) < fx(5.0));
    }

    #[test]
    fn pressure_recovers_to_zero() {
        let mut p = fx(50.0);
        for _ in 0..10_000 {
            p = recover(p, fx(0.05));
        }
        assert_eq!(p, 0);
    }

    #[test]
    fn crash_cuts_drift() {
        assert_eq!(crash_drift(ONE, 100), fx(0.3));
        assert_eq!(crash_drift(ONE, 0), ONE);
    }
}
