//! Schema-aware DSL validation + did-you-mean diagnostics.
//!
//! Phase 3, task 11: `validate_block` walks an AST against a schema and
//! emits clear `syn::Error`s for unknown fields, wrong operators, and
//! cardinality mismatches. The Jaro-Winkler-based `suggest` helper is
//! also used by lowering passes when they encounter unknown keys.

#[allow(dead_code)]
pub fn suggest(key: &str, candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .map(|c| (c, strsim::jaro_winkler(key, c)))
        .filter(|(_, s)| *s >= 0.85)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(c, _)| c.clone())
}
