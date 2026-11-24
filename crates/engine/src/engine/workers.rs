//! # Workers and diminishing returns
//!
//! A node has an *optimal* crew size. Below it, speed scales linearly
//! (half the crew = half the speed). Above it, each extra worker adds less
//! than the last: the first extra adds `d` of a worker, the next `d^2`, and so
//! on, where `d` is the global "over-crew returns" lever. With `d = 50%` a
//! crew of 2 on an optimal-2 node runs at 100%, 3 at 125%, 4 at 137.5%, and
//! it never reaches 200%. This is the classic idle-game pattern that makes
//! spreading workers out better than piling them on one node.
//!
//! The result is a *crew factor* in fixed point, where `ONE` = full speed.

use crate::fixed::{Fx, fx};

/// Crew factor for `workers` on a node whose optimal crew is `optimal`.
/// `diminishing_pct` is the over-crew return in percent (0..=100).
/// Nodes with `optimal <= 0` don't need workers and always run at full speed.
pub fn crew_factor(workers: i64, optimal: i64, diminishing_pct: i64) -> Fx {
    if optimal <= 0 {
        return crate::fixed::ONE;
    }
    if workers <= 0 {
        return 0;
    }
    let base = workers.min(optimal) as f64;
    let extra = (workers - optimal).max(0);
    let d = (diminishing_pct as f64 / 100.0).clamp(0.0, 1.0);
    // Geometric series d + d^2 + ... + d^extra
    let bonus = if extra == 0 {
        0.0
    } else if (d - 1.0).abs() < f64::EPSILON {
        extra as f64
    } else {
        d * (1.0 - d.powi(extra as i32)) / (1.0 - d)
    };
    fx((base + bonus) / optimal as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::ONE;

    #[test]
    fn linear_below_optimal() {
        assert_eq!(crew_factor(0, 4, 50), 0);
        assert_eq!(crew_factor(2, 4, 50), ONE / 2);
        assert_eq!(crew_factor(4, 4, 50), ONE);
    }

    #[test]
    fn diminishing_above_optimal() {
        assert_eq!(crew_factor(3, 2, 50), 1250);
        assert_eq!(crew_factor(4, 2, 50), 1375);
        assert_eq!(crew_factor(3, 2, 0), ONE);
        assert_eq!(crew_factor(3, 2, 100), 1500);
    }

    #[test]
    fn crewless_nodes_full_speed() {
        assert_eq!(crew_factor(0, 0, 50), ONE);
    }
}
