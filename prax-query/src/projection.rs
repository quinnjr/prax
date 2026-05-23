//! Scalar-subquery projections for SELECT clauses.
//!
//! Used by relation-aggregate virtual fields (`@count`, `@sum`, …) and
//! by the `select: { _count: { rel: true } }` ad-hoc accessor (phase
//! 5.5). The `sql` field uses the same `{N}` placeholder convention as
//! [`crate::filter::Filter::ScalarSubquery`] — `{N}` resolves to the
//! dialect placeholder for `params[N]`. Placeholders are renumbered
//! into a single positional sequence at SqlBuilder time, so they
//! compose cleanly with WHERE filters and other projections.

use std::borrow::Cow;

use crate::filter::FilterValue;

/// A scalar-subquery projection added to a SELECT clause, emitted as
/// `(<sql>) AS <alias>`. The alias is a codegen-controlled
/// `&'static str` — never user input — so it can be safely interpolated
/// into SQL after identifier quoting.
#[derive(Debug, Clone)]
pub struct ScalarProjection {
    /// SQL fragment with `{N}` placeholders.
    pub sql: Cow<'static, str>,
    /// Parameter values referenced by the `{N}` placeholders.
    pub params: Vec<FilterValue>,
    /// Output column alias.
    pub alias: &'static str,
}

impl ScalarProjection {
    pub fn new(
        sql: impl Into<Cow<'static, str>>,
        params: Vec<FilterValue>,
        alias: &'static str,
    ) -> Self {
        Self {
            sql: sql.into(),
            params,
            alias,
        }
    }

    /// Rewrite `{N}` placeholders to dialect-specific positional form,
    /// offsetting by `offset` (the count of params already emitted by
    /// earlier clauses in the same query).
    ///
    /// All `params` are appended to `out_params` in index order so the
    /// caller's global param list stays consistent.
    pub(crate) fn to_sql(
        &self,
        offset: usize,
        dialect: &dyn crate::dialect::SqlDialect,
        out_params: &mut Vec<FilterValue>,
    ) -> String {
        // Push this projection's params in order; {N} maps to global slot
        // (offset + N + 1) — matching the Filter::ScalarSubquery convention.
        for v in self.params.iter() {
            out_params.push(v.clone());
        }

        let sql = &self.sql;
        let mut out = String::with_capacity(sql.len() + self.params.len() * 4);
        let mut chars = sql.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '{' {
                let mut digits = String::new();
                while let Some(&c) = chars.peek() {
                    if c == '}' {
                        chars.next();
                        break;
                    }
                    digits.push(c);
                    chars.next();
                }
                let n: usize = digits.parse().unwrap_or_else(|_| {
                    panic!(
                        "ScalarProjection: invalid placeholder index `{{{}}}`",
                        digits
                    )
                });
                if n >= self.params.len() {
                    panic!(
                        "ScalarProjection: placeholder {{{}}} out of range (have {} params)",
                        n,
                        self.params.len()
                    );
                }
                out.push_str(&dialect.placeholder(offset + n + 1));
            } else {
                out.push(ch);
            }
        }
        out
    }
}
