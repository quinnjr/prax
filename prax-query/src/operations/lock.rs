//! Shared row-lock validation and rendering for the find operations.

use crate::advanced::RowLock;
use crate::dialect::{RowLockSql, SqlDialect};
use crate::error::{QueryError, QueryResult};

/// Check that `lock` (if any) can be expressed by `dialect` and combined
/// with the rest of the query.
pub(crate) fn validate(
    lock: Option<&RowLock>,
    dialect: &dyn SqlDialect,
    has_distinct: bool,
) -> QueryResult<()> {
    let Some(lock) = lock else {
        return Ok(());
    };
    if has_distinct {
        return Err(QueryError::invalid_input(
            "lock",
            "row locks cannot be combined with distinct",
        ));
    }
    dialect.row_lock_sql(lock).map(|_| ())
}

/// Render `lock` for `dialect`. Yields the empty rendering when there is no
/// lock or the dialect rejects it, so `build_sql` stays infallible: a lock
/// the dialect rejects is silently left out of `build_sql`'s output, and
/// only `exec` (through [`validate`]) reports it.
pub(crate) fn render(lock: Option<&RowLock>, dialect: &dyn SqlDialect) -> RowLockSql {
    lock.and_then(|l| dialect.row_lock_sql(l).ok())
        .unwrap_or_default()
}
