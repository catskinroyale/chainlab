//! Scenario loading.
//!
//! Built-in chains are embedded in the binary so it runs anywhere. A file on
//! disk with the same name in `data/` (or any path passed explicitly) takes
//! precedence, so you can edit chains without recompiling.

use std::path::Path;

use crate::config::{ChainConfig, parse_chain};

/// Built-in chain definitions: (name, RON text).
pub const BUILTIN: &[(&str, &str)] = &[(
    "default",
    include_str!("../../../../data/chain_default.ron"),
)];

/// Names of all built-in scenarios.
pub fn names() -> Vec<&'static str> {
    BUILTIN.iter().map(|(n, _)| *n).collect()
}

/// Load a scenario by name or path.
///
/// Lookup order: an existing file path, `data/<name>.ron`,
/// `data/chain_<name>.ron`, then the embedded built-ins.
pub fn load(name_or_path: &str) -> Result<ChainConfig, String> {
    let candidates = [
        name_or_path.to_string(),
        format!("data/{name_or_path}.ron"),
        format!("data/chain_{name_or_path}.ron"),
    ];
    for c in &candidates {
        let p = Path::new(c);
        if p.is_file() {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{c}: {e}"))?;
            return parse_chain(&text).map_err(|e| format!("{c}: {e}"));
        }
    }
    let key = name_or_path.to_ascii_lowercase();
    BUILTIN
        .iter()
        .find(|(n, _)| *n == key)
        .map(|(_, text)| parse_chain(text))
        .unwrap_or_else(|| {
            Err(format!(
                "unknown scenario '{name_or_path}' (built-ins: {})",
                names().join(", ")
            ))
        })
}
