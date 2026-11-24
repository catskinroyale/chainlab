//! Live parameters ("levers").
//!
//! Every tunable number in the simulation is a [`Param`] in one flat
//! [`ParamSet`]. Each remembers its default so the UI can show what changed
//! and reset it. Values are integers; [`Unit`] says how to read them
//! (fixed-point quantity, ticks, percent...).
//!
//! Parameters are addressed by a stable string key such as
//! `node.kiln.cycle` or `link.0.carriers`, which is what save files use.

use std::collections::BTreeMap;

use crate::TICKS_PER_SEC;
use crate::fixed::{Fx, fx, to_f64};

pub type ParamId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// 0 or 1.
    Bool,
    /// Plain integer.
    Count,
    /// Integer percent.
    Percent,
    /// Fixed-point quantity.
    Qty,
    /// Fixed-point quantity per second.
    Rate,
    /// Fixed-point dimensionless ratio.
    Ratio,
    /// Integer ticks, shown as seconds.
    Ticks,
}

impl Unit {
    pub fn to_human(self, v: i64) -> f64 {
        match self {
            Unit::Bool | Unit::Count | Unit::Percent => v as f64,
            Unit::Qty | Unit::Rate | Unit::Ratio => to_f64(v),
            Unit::Ticks => v as f64 / TICKS_PER_SEC as f64,
        }
    }

    pub fn from_human(self, x: f64) -> i64 {
        match self {
            Unit::Bool | Unit::Count | Unit::Percent => x.round() as i64,
            Unit::Qty | Unit::Rate | Unit::Ratio => fx(x),
            Unit::Ticks => (x * TICKS_PER_SEC as f64).round() as i64,
        }
    }

    pub fn format(self, v: i64) -> String {
        match self {
            Unit::Bool => (if v != 0 { "on" } else { "off" }).to_string(),
            Unit::Count => v.to_string(),
            Unit::Percent => format!("{v}%"),
            Unit::Qty => format!("{:.1}", to_f64(v)),
            Unit::Rate => format!("{:.2}/s", to_f64(v)),
            Unit::Ratio => format!("{:.3}", to_f64(v)),
            Unit::Ticks => format!("{:.1}s", self.to_human(v)),
        }
    }
}

/// Which lever panel group a parameter belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Node(usize),
    Link(usize),
    Global,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub key: String,
    pub label: String,
    pub group: Group,
    pub unit: Unit,
    pub value: i64,
    pub default: i64,
    pub min: i64,
    pub max: i64,
    pub fine: i64,
    pub coarse: i64,
}

impl Param {
    pub fn changed(&self) -> bool {
        self.value != self.default
    }
    pub fn set(&mut self, v: i64) {
        self.value = v.clamp(self.min, self.max);
    }
    pub fn display(&self) -> String {
        self.unit.format(self.value)
    }
    pub fn display_default(&self) -> String {
        self.unit.format(self.default)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamSet {
    pub list: Vec<Param>,
    index: BTreeMap<String, ParamId>,
}

impl ParamSet {
    /// Register a parameter. `default` is in raw integer units.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        key: impl Into<String>,
        label: impl Into<String>,
        group: Group,
        unit: Unit,
        default: i64,
        (min, max): (i64, i64),
        (fine, coarse): (i64, i64),
    ) -> ParamId {
        let key = key.into();
        let id = self.list.len();
        let default = default.clamp(min, max);
        self.index.insert(key.clone(), id);
        self.list.push(Param {
            key,
            label: label.into(),
            group,
            unit,
            value: default,
            default,
            min,
            max,
            fine,
            coarse,
        });
        id
    }

    pub fn id(&self, key: &str) -> Option<ParamId> {
        self.index.get(key).copied()
    }

    pub fn get(&self, id: ParamId) -> i64 {
        self.list[id].value
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Set a value in human units by key.
    pub fn set_human(&mut self, key: &str, x: f64) -> Result<(), String> {
        let id = self
            .id(key)
            .ok_or_else(|| format!("unknown parameter '{key}'"))?;
        let p = &mut self.list[id];
        let v = p.unit.from_human(x);
        p.set(v);
        Ok(())
    }

    /// Make the current values the new defaults (used by scenario presets).
    pub fn bake_defaults(&mut self) {
        for p in &mut self.list {
            p.default = p.value;
        }
    }

    /// Snapshot of current values.
    pub fn values(&self) -> Vec<Fx> {
        self.list.iter().map(|p| p.value).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_change_and_human_units() {
        let mut ps = ParamSet::default();
        let id = ps.add(
            "x.cycle",
            "cycle",
            Group::Global,
            Unit::Ticks,
            40,
            (1, 100),
            (1, 10),
        );
        assert_eq!(ps.list[id].display(), "4.0s");
        ps.list[id].set(500);
        assert_eq!(ps.get(id), 100);
        assert!(ps.list[id].changed());
        ps.set_human("x.cycle", 2.5).unwrap();
        assert_eq!(ps.get(id), 25);
        assert!(ps.set_human("nope", 1.0).is_err());
    }
}
