//! Runtime for relation loading.
//!
//! Fetches children keyed by parent PK, buckets by FK, and hands back
//! a map the per-model [`crate::traits::ModelRelationLoader::load_relation`]
//! closure uses to stitch results onto the parent slice.
//!
//! The executor is the only place in the pipeline that issues a
//! secondary query on behalf of `.include()`. It deliberately avoids
//! JOINs so the parent hydration path stays the same as a bare
//! `find_many()` — no row-multiplication, no column-name collisions,
//! one network round-trip per included relation.

use std::collections::HashMap;

use crate::error::{QueryError, QueryResult};
use crate::filter::{Filter, FilterValue};
use crate::relations::RelationMeta;
use crate::row::FromRow;
use crate::traits::{Model, ModelWithPk, QueryEngine};

/// Fetch children for a `HasMany` (or `HasOne`) relation and bucket by
/// the FK column value turned into a stable string key.
///
/// Returns an empty map when `parents` is empty — the caller must
/// short-circuit before issuing the SELECT.
pub async fn load_has_many<E, P, C, R>(
    engine: &E,
    parents: &[P],
) -> QueryResult<HashMap<String, Vec<C>>>
where
    E: QueryEngine,
    P: Model + ModelWithPk,
    C: Model + ModelWithPk + FromRow + Send + 'static,
    R: RelationMeta<Owner = P, Target = C>,
{
    let pk_values: Vec<FilterValue> = parents.iter().map(|p| p.pk_value()).collect();
    if pk_values.is_empty() {
        return Ok(HashMap::new());
    }

    let filter = Filter::In(R::FOREIGN_KEY.into(), pk_values);
    let dialect = engine.dialect();
    let (where_sql, params) = filter.to_sql(0, dialect);
    let sql = format!("SELECT * FROM {} WHERE {}", C::TABLE_NAME, where_sql);

    let children: Vec<C> = engine.query_many::<C>(&sql, params).await?;

    let mut out: HashMap<String, Vec<C>> = HashMap::new();
    for child in children {
        let fk = child.get_column_value(R::FOREIGN_KEY).ok_or_else(|| {
            QueryError::internal(format!(
                "relation {}: child model missing FK column {}",
                R::NAME,
                R::FOREIGN_KEY
            ))
        })?;
        let key = filter_value_key(&fk);
        out.entry(key).or_default().push(child);
    }
    Ok(out)
}

/// Map a scalar [`FilterValue`] to a stable string key, re-exported
/// for use inside `#[derive(Model)]`-generated code.
///
/// Codegen emits this call inline in the `ModelRelationLoader` match
/// arms, so it must be publicly reachable from `::prax_query::...`.
/// Runtime callers should prefer the inner [`filter_value_key`]
/// helper.
#[doc(hidden)]
pub fn filter_value_key_public(v: &FilterValue) -> String {
    filter_value_key(v)
}

/// Map a scalar [`FilterValue`] to a stable string key for bucketing
/// children by their parent's PK value.
///
/// Single-column PKs route through the scalar variants. Composite PKs
/// arrive here as [`FilterValue::List`] and currently panic — the
/// relation executor does not support composite keys yet because every
/// derived model has a single-column PK, and supporting multi-column
/// PKs would complicate the FK column lookup on the child side.
pub(crate) fn filter_value_key(v: &FilterValue) -> String {
    match v {
        FilterValue::Int(i) => i.to_string(),
        FilterValue::String(s) => s.clone(),
        FilterValue::Bool(b) => b.to_string(),
        FilterValue::Float(f) => f.to_string(),
        FilterValue::Null => "<null>".into(),
        FilterValue::Json(v) => v.to_string(),
        FilterValue::List(_) => {
            panic!("relation executor does not support composite keys yet (FilterValue::List)")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_value_key_int() {
        assert_eq!(filter_value_key(&FilterValue::Int(42)), "42");
    }

    #[test]
    fn filter_value_key_string() {
        assert_eq!(filter_value_key(&FilterValue::String("abc".into())), "abc");
    }

    #[test]
    fn filter_value_key_bool() {
        assert_eq!(filter_value_key(&FilterValue::Bool(true)), "true");
    }

    #[test]
    fn filter_value_key_null() {
        assert_eq!(filter_value_key(&FilterValue::Null), "<null>");
    }

    #[test]
    #[should_panic(expected = "composite keys")]
    fn filter_value_key_list_panics() {
        let _ = filter_value_key(&FilterValue::List(vec![FilterValue::Int(1)]));
    }
}
