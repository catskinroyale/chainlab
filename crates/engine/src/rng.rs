//! Deterministic pseudo-random numbers.
//!
//! A tiny self-contained xoshiro256** generator seeded through SplitMix64.
//! No external crates, so the sequence can never change under us because a
//! dependency was upgraded.
//!
//! Each subsystem gets its **own stream** derived from the master seed. That
//! way switching random events on or off does not shift the sequence used for
//! yield variance, and comparisons between parameter sets stay fair.
//! Subsystems also draw a fixed number of values per tick whether or not they
//! use them, for the same reason.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// Create a generator for `stream` under master `seed`.
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut sm = seed ^ stream.wrapping_mul(0xD1B5_4A32_D192_ED03);
        let mut next = || {
            sm = sm.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = sm;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Rng {
            s: [next(), next(), next(), next()],
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[-1, 1)`.
    pub fn signed(&mut self) -> f64 {
        self.unit() * 2.0 - 1.0
    }
}

/// One stream per subsystem.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rngs {
    pub yields: Rng,
    pub market: Rng,
    pub events: Rng,
}

impl Rngs {
    pub fn new(seed: u64) -> Self {
        Rngs {
            yields: Rng::new(seed, 1),
            market: Rng::new(seed, 2),
            events: Rng::new(seed, 3),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Rng::new(42, 1);
        let mut b = Rng::new(42, 1);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn streams_differ() {
        let mut a = Rng::new(42, 1);
        let mut b = Rng::new(42, 2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn unit_in_range() {
        let mut r = Rng::new(7, 0);
        for _ in 0..10_000 {
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }
}
