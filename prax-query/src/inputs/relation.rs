//! Relation-aware filter wrappers.
//!
//! `ListRelationFilter<W>` and `SingleRelationFilter<W>` carry the
//! Prisma operator shape (`some`/`every`/`none` for to-many;
//! `is`/`is_not` for to-one). They lower to [`Filter::ScalarSubquery`]
//! fragments via a per-relation [`RelationMeta`] adapter that codegen
//! emits (phase 2) but tests / hand-built users can supply directly.

use crate::filter::{Filter, FilterValue};
use crate::inputs::traits::WhereInput;

/// Static metadata for one parent→child relation.
///
/// Phase 2 codegen emits one impl per relation declared in the schema.
/// Hand-rolled callers can implement this trait themselves.
pub trait RelationMeta {
    /// Parent SQL table name.
    const PARENT_TABLE: &'static str;
    /// Parent primary-key column name.
    const PARENT_PK: &'static str;
    /// Child SQL table name.
    const CHILD_TABLE: &'static str;
    /// Child foreign-key column name pointing back at the parent.
    const CHILD_FK: &'static str;
}

/// Filter operators for a to-many relation.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(bound = "W: serde::Serialize + for<'de2> serde::Deserialize<'de2>")]
pub struct ListRelationFilter<W> {
    /// At least one child matches `W`.
    pub some: Option<W>,
    /// Every existing child matches `W`.
    pub every: Option<W>,
    /// No child matches `W`.
    pub none: Option<W>,
}

/// Filter operators for a to-one relation.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(bound = "W: serde::Serialize + for<'de2> serde::Deserialize<'de2>")]
pub struct SingleRelationFilter<W> {
    /// The related row matches `W`.
    pub is: Option<W>,
    /// The related row does NOT match `W` (or doesn't exist).
    pub is_not: Option<W>,
}

/// Lowering helper: produces `Filter::ScalarSubquery` from a relation
/// filter + `RelationMeta`.
///
/// Implemented for any `W: WhereInput` so neither the codegen nor the
/// macro layer needs to manually thread metadata.
pub trait LowerRelationFilter {
    /// Lower this relation filter to a runtime [`Filter`] using the
    /// supplied metadata.
    fn lower<M: RelationMeta>(self) -> Filter;
}

/// Walk a `Filter` tree and produce inline SQL with `{N}` placeholders.
///
/// Phase 1 supports the operators that the scalar filters emit.
/// `ScalarSubquery` nesting is not expected at phase 1 and panics.
fn render_inline_filter(inner: Filter) -> (String, Vec<FilterValue>) {
    let mut sql = String::new();
    let mut params = Vec::<FilterValue>::new();
    write_filter(&inner, &mut sql, &mut params);
    (sql, params)
}

fn write_filter(f: &Filter, sql: &mut String, params: &mut Vec<FilterValue>) {
    match f {
        Filter::None => sql.push_str("TRUE"),
        Filter::Equals(c, v) => {
            if matches!(v, FilterValue::Null) {
                sql.push_str(&format!("{} IS NULL", c));
            } else {
                let idx = params.len();
                params.push(v.clone());
                sql.push_str(&format!("{} = {{{}}}", c, idx));
            }
        }
        Filter::NotEquals(c, v) => {
            if matches!(v, FilterValue::Null) {
                sql.push_str(&format!("{} IS NOT NULL", c));
            } else {
                let idx = params.len();
                params.push(v.clone());
                sql.push_str(&format!("{} <> {{{}}}", c, idx));
            }
        }
        Filter::Lt(c, v) => {
            let i = params.len();
            params.push(v.clone());
            sql.push_str(&format!("{} < {{{}}}", c, i));
        }
        Filter::Lte(c, v) => {
            let i = params.len();
            params.push(v.clone());
            sql.push_str(&format!("{} <= {{{}}}", c, i));
        }
        Filter::Gt(c, v) => {
            let i = params.len();
            params.push(v.clone());
            sql.push_str(&format!("{} > {{{}}}", c, i));
        }
        Filter::Gte(c, v) => {
            let i = params.len();
            params.push(v.clone());
            sql.push_str(&format!("{} >= {{{}}}", c, i));
        }
        Filter::IsNull(c) => sql.push_str(&format!("{} IS NULL", c)),
        Filter::IsNotNull(c) => sql.push_str(&format!("{} IS NOT NULL", c)),
        Filter::Contains(c, FilterValue::String(s)) => {
            let i = params.len();
            params.push(FilterValue::String(format!("%{}%", s)));
            sql.push_str(&format!("{} LIKE {{{}}}", c, i));
        }
        Filter::StartsWith(c, FilterValue::String(s)) => {
            let i = params.len();
            params.push(FilterValue::String(format!("{}%", s)));
            sql.push_str(&format!("{} LIKE {{{}}}", c, i));
        }
        Filter::EndsWith(c, FilterValue::String(s)) => {
            let i = params.len();
            params.push(FilterValue::String(format!("%{}", s)));
            sql.push_str(&format!("{} LIKE {{{}}}", c, i));
        }
        Filter::Contains(_, _) | Filter::StartsWith(_, _) | Filter::EndsWith(_, _) => {
            panic!("phase 1 inline lowering supports only String LIKE values");
        }
        Filter::In(c, values) => {
            if values.is_empty() {
                sql.push_str("FALSE");
                return;
            }
            sql.push_str(&format!("{} IN (", c));
            for (n, v) in values.iter().enumerate() {
                if n > 0 {
                    sql.push_str(", ");
                }
                let i = params.len();
                params.push(v.clone());
                sql.push_str(&format!("{{{}}}", i));
            }
            sql.push(')');
        }
        Filter::NotIn(c, values) => {
            if values.is_empty() {
                sql.push_str("TRUE");
                return;
            }
            sql.push_str(&format!("{} NOT IN (", c));
            for (n, v) in values.iter().enumerate() {
                if n > 0 {
                    sql.push_str(", ");
                }
                let i = params.len();
                params.push(v.clone());
                sql.push_str(&format!("{{{}}}", i));
            }
            sql.push(')');
        }
        Filter::And(parts) => {
            if parts.is_empty() {
                sql.push_str("TRUE");
                return;
            }
            sql.push('(');
            for (n, p) in parts.iter().enumerate() {
                if n > 0 {
                    sql.push_str(" AND ");
                }
                write_filter(p, sql, params);
            }
            sql.push(')');
        }
        Filter::Or(parts) => {
            if parts.is_empty() {
                sql.push_str("FALSE");
                return;
            }
            sql.push('(');
            for (n, p) in parts.iter().enumerate() {
                if n > 0 {
                    sql.push_str(" OR ");
                }
                write_filter(p, sql, params);
            }
            sql.push(')');
        }
        Filter::Not(inner) => {
            sql.push_str("NOT (");
            write_filter(inner, sql, params);
            sql.push(')');
        }
        Filter::ScalarSubquery { .. } => {
            panic!("phase 1 does not support nesting ScalarSubquery inside relation filters");
        }
    }
}

impl<W: WhereInput> LowerRelationFilter for ListRelationFilter<W> {
    fn lower<M: RelationMeta>(self) -> Filter {
        let mut clauses: Vec<Filter> = Vec::new();

        if let Some(w) = self.some {
            let (body, params) = render_inline_filter(w.into_ir());
            let sql = format!(
                "EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND {})",
                M::CHILD_TABLE,
                M::CHILD_TABLE,
                M::CHILD_FK,
                M::PARENT_TABLE,
                M::PARENT_PK,
                body,
            );
            clauses.push(Filter::ScalarSubquery {
                sql: sql.into(),
                params,
            });
        }

        if let Some(w) = self.every {
            let (body, params) = render_inline_filter(w.into_ir());
            let sql = format!(
                "NOT EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND NOT ({}))",
                M::CHILD_TABLE,
                M::CHILD_TABLE,
                M::CHILD_FK,
                M::PARENT_TABLE,
                M::PARENT_PK,
                body,
            );
            clauses.push(Filter::ScalarSubquery {
                sql: sql.into(),
                params,
            });
        }

        if let Some(w) = self.none {
            let (body, params) = render_inline_filter(w.into_ir());
            let sql = format!(
                "NOT EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND {})",
                M::CHILD_TABLE,
                M::CHILD_TABLE,
                M::CHILD_FK,
                M::PARENT_TABLE,
                M::PARENT_PK,
                body,
            );
            clauses.push(Filter::ScalarSubquery {
                sql: sql.into(),
                params,
            });
        }

        match clauses.len() {
            0 => Filter::None,
            1 => clauses.into_iter().next().unwrap(),
            _ => Filter::and(clauses),
        }
    }
}

impl<W: WhereInput> LowerRelationFilter for SingleRelationFilter<W> {
    fn lower<M: RelationMeta>(self) -> Filter {
        let mut clauses: Vec<Filter> = Vec::new();

        if let Some(w) = self.is {
            let (body, params) = render_inline_filter(w.into_ir());
            let sql = format!(
                "EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND {})",
                M::CHILD_TABLE,
                M::CHILD_TABLE,
                M::CHILD_FK,
                M::PARENT_TABLE,
                M::PARENT_PK,
                body,
            );
            clauses.push(Filter::ScalarSubquery {
                sql: sql.into(),
                params,
            });
        }

        if let Some(w) = self.is_not {
            let (body, params) = render_inline_filter(w.into_ir());
            let sql = format!(
                "NOT EXISTS (SELECT 1 FROM {} WHERE {}.{} = {}.{} AND {})",
                M::CHILD_TABLE,
                M::CHILD_TABLE,
                M::CHILD_FK,
                M::PARENT_TABLE,
                M::PARENT_PK,
                body,
            );
            clauses.push(Filter::ScalarSubquery {
                sql: sql.into(),
                params,
            });
        }

        match clauses.len() {
            0 => Filter::None,
            1 => clauses.into_iter().next().unwrap(),
            _ => Filter::and(clauses),
        }
    }
}
