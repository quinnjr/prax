//! Schema-aware DSL validation + did-you-mean diagnostics.
//!
//! Most of the validation happens inline during lowering (the
//! `where_input` / `include_input` / `select_input` / `order_by_input`
//! lowering passes each call `model.get_field(...)` and emit a
//! `syn::Error` on miss). This module centralizes the suggester
//! helper used by every lowering pass, plus a top-level
//! `validate_top_keys` helper for op-level diagnostics
//! (`unknown_top_key`, `select` xor `include`, etc.).

#![allow(dead_code)]

use proc_macro2::Span;

/// Jaro-Winkler-based "did you mean" suggester.
///
/// Returns the highest-scoring candidate above the 0.85 similarity
/// threshold, or `None` if no candidate is close enough. The threshold
/// matches the spec §5 value.
pub fn suggest(key: &str, candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .map(|c| (c, strsim::jaro_winkler(key, c)))
        .filter(|(_, s)| *s >= 0.85)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(c, _)| c.clone())
}

/// Helper for op-level diagnostics on the top-level keys of an
/// operation input (`where`, `include`, `select`, `order_by`, ...).
/// Emits a `syn::Error` pointing at `key_span` with a suggestion if
/// any candidate is close enough.
pub fn unknown_top_key_error(
    key: &str,
    key_span: Span,
    allowed: &[&'static str],
    op_name: &str,
) -> syn::Error {
    let owned: Vec<String> = allowed.iter().map(|s| (*s).to_string()).collect();
    let suggestion = suggest(key, &owned);
    let msg = match suggestion {
        Some(c) => format!(
            "unknown top-level key `{key}` for `{op_name}!`. did you mean `{c}`? \
             Allowed keys: {allowed:?}"
        ),
        None => {
            format!("unknown top-level key `{key}` for `{op_name}!`. Allowed keys: {allowed:?}")
        }
    };
    syn::Error::new(key_span, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_returns_typo_neighbor() {
        let candidates = vec!["email".to_string(), "name".to_string(), "id".to_string()];
        assert_eq!(suggest("emial", &candidates).as_deref(), Some("email"));
    }

    #[test]
    fn suggest_returns_none_when_too_far() {
        let candidates = vec!["email".to_string()];
        assert!(suggest("xyz", &candidates).is_none());
    }

    #[test]
    fn suggest_picks_highest_when_multiple_close() {
        let candidates = vec!["email".to_string(), "emails".to_string()];
        // `emial` is closer to `email` than `emails`.
        assert_eq!(suggest("emial", &candidates).as_deref(), Some("email"));
    }

    #[test]
    fn unknown_top_key_error_includes_suggestion_when_close() {
        let err = unknown_top_key_error(
            "wher",
            Span::call_site(),
            &["where", "include", "select"],
            "find_many",
        );
        let msg = err.to_string();
        assert!(msg.contains("did you mean"), "got: {msg}");
        assert!(msg.contains("where"), "got: {msg}");
    }

    #[test]
    fn unknown_top_key_error_omits_suggestion_when_far() {
        let err = unknown_top_key_error(
            "xyzzy",
            Span::call_site(),
            &["where", "include", "select"],
            "find_many",
        );
        let msg = err.to_string();
        assert!(!msg.contains("did you mean"), "got: {msg}");
    }
}
