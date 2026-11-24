//! Scenarios: a chain plus lever overrides.
//!
//! Built-in chains and scenarios are embedded in the binary so it runs
//! anywhere. A file on disk takes precedence (`data/<name>.ron`,
//! `data/scenarios/<name>.ron`, `data/chain_<name>.ron` or any path), so you
//! can edit or add scenarios without recompiling.
//!
//! Scenario overrides become the levers' *defaults*, so "changed" markers
//! and "reset" are relative to the scenario, not the raw chain.

use std::path::Path;

use crate::config::{ChainConfig, ScenarioDef, parse_chain, parse_scenario};
use crate::engine::Sim;

/// Built-in chains: (name, RON text).
pub const CHAINS: &[(&str, &str)] = &[(
    "default",
    include_str!("../../../../data/chain_default.ron"),
)];

/// Built-in scenarios: (id, RON text).
pub const SCENARIOS: &[(&str, &str)] = &[
    (
        "balanced",
        include_str!("../../../../data/scenarios/balanced.ron"),
    ),
    (
        "starved_forest",
        include_str!("../../../../data/scenarios/starved_forest.ron"),
    ),
    (
        "expensive_logistics",
        include_str!("../../../../data/scenarios/expensive_logistics.ron"),
    ),
    (
        "volatile_market",
        include_str!("../../../../data/scenarios/volatile_market.ron"),
    ),
];

/// A resolved scenario ready to instantiate.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub id: String,
    pub scenario: ScenarioDef,
    pub chain: ChainConfig,
}

impl Loaded {
    /// Build a sim. `seed` overrides the scenario's seed (default 42).
    pub fn build(&self, seed: Option<u64>) -> Result<Sim, String> {
        let seed = seed.or(self.scenario.seed).unwrap_or(42);
        let mut sim = Sim::new(&self.chain, seed)?;
        let unknown = sim.apply_values(&self.scenario.overrides, true);
        if !unknown.is_empty() {
            return Err(format!(
                "scenario '{}': unknown parameters {}",
                self.id,
                unknown.join(", ")
            ));
        }
        Ok(sim)
    }
}

/// (id, name, description) of every built-in scenario.
pub fn list() -> Vec<(String, String, String)> {
    SCENARIOS
        .iter()
        .filter_map(|(id, text)| {
            parse_scenario(text)
                .ok()
                .map(|s| (id.to_string(), s.name, s.description))
        })
        .collect()
}

pub fn names() -> Vec<&'static str> {
    SCENARIOS.iter().map(|(n, _)| *n).collect()
}

fn read_file(candidates: &[String]) -> Option<Result<(String, String), String>> {
    candidates.iter().find(|c| Path::new(c).is_file()).map(|c| {
        std::fs::read_to_string(c)
            .map(|t| (c.clone(), t))
            .map_err(|e| format!("{c}: {e}"))
    })
}

/// Load a chain by built-in name or path.
pub fn chain(name_or_path: &str) -> Result<ChainConfig, String> {
    let files = [
        name_or_path.to_string(),
        format!("data/chain_{name_or_path}.ron"),
    ];
    if let Some(r) = read_file(&files) {
        let (path, text) = r?;
        return parse_chain(&text).map_err(|e| format!("{path}: {e}"));
    }
    CHAINS
        .iter()
        .find(|(n, _)| *n == name_or_path)
        .map(|(_, t)| parse_chain(t))
        .unwrap_or_else(|| Err(format!("unknown chain '{name_or_path}'")))
}

/// Load a scenario (or a bare chain file) by id, name or path.
pub fn load(name_or_path: &str) -> Result<Loaded, String> {
    let id = name_or_path.to_ascii_lowercase().replace([' ', '-'], "_");
    let files = [
        name_or_path.to_string(),
        format!("data/scenarios/{id}.ron"),
        format!("data/{id}.ron"),
        format!("data/chain_{id}.ron"),
    ];
    let (origin, text) = match read_file(&files) {
        Some(r) => r?,
        None => {
            if let Some((_, t)) = SCENARIOS.iter().find(|(n, _)| *n == id) {
                (id.clone(), t.to_string())
            } else if let Some((_, t)) = CHAINS.iter().find(|(n, _)| *n == id) {
                (id.clone(), t.to_string())
            } else {
                return Err(format!(
                    "unknown scenario '{name_or_path}' (built-ins: {})",
                    names().join(", ")
                ));
            }
        }
    };
    match parse_scenario(&text) {
        Ok(scenario) => {
            let chain = chain(&scenario.chain)?;
            Ok(Loaded {
                id,
                scenario,
                chain,
            })
        }
        Err(se) => match parse_chain(&text) {
            Ok(chain) => Ok(Loaded {
                id,
                scenario: ScenarioDef {
                    name: chain.name.clone(),
                    description: String::new(),
                    chain: origin,
                    seed: None,
                    overrides: Default::default(),
                },
                chain,
            }),
            Err(ce) => Err(format!("{origin}: not a scenario ({se}) or chain ({ce})")),
        },
    }
}
