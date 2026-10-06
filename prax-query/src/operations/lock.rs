//! Shared row-lock validation and rendering for the find operations.

use crate::advanced::RowLock;
use crate::dialect::{RowLockSql, SqlDialect};
use crate::error::QueryResult;

/// Check that `lock` (if any) can be expressed by `dialect`.
///
/// Combinations with other clauses (for example `distinct`) are checked by
/// the operation that owns the clause.
pub(crate) fn validate(lock: Option<&RowLock>, dialect: &dyn SqlDialect) -> QueryResult<()> {
    match lock {
        Some(lock) => dialect.row_lock_sql(lock).map(|_| ()),
        None => Ok(()),
    }
}

/// Render `lock` for `dialect`. Yields the empty rendering when there is no
/// lock or the dialect rejects it, so `build_sql` stays infallible: a lock
/// the dialect rejects is silently left out of `build_sql`'s output. Use
/// `try_build_sql` (or `exec`, which calls it) to surface the error.
pub(crate) fn render(lock: Option<&RowLock>, dialect: &dyn SqlDialect) -> RowLockSql {
    lock.and_then(|l| dialect.row_lock_sql(l).ok())
        .unwrap_or_default()
}

/// Expands to the `lock` / `for_update` / `for_share` methods, so their
/// rustdoc exists once. Invoke inside an `impl` block bounded by
/// `SupportsRowLocks` on a type with a `lock: Option<RowLock>` field.
macro_rules! row_lock_methods {
    () => {
        /// Lock the selected rows (`SELECT ... FOR UPDATE` and friends).
        ///
        /// Locks are held until the surrounding transaction ends. Outside a
        /// transaction they are released as soon as the statement finishes,
        /// so they protect nothing; run locking queries inside
        /// `PraxClient::transaction`. With `include`, only the root query is
        /// locked; eagerly loaded relations are read without locks.
        ///
        /// Unsupported combinations (for example `SKIP LOCKED` with a shared
        /// lock on MSSQL) fail at `exec` and in `try_build_sql`; `build_sql`
        /// leaves a rejected lock out of its output.
        pub fn lock(mut self, lock: impl Into<$crate::advanced::RowLock>) -> Self {
            self.lock = Some(lock.into());
            self
        }

        /// Shorthand for `.lock(RowLock::for_update())`.
        pub fn for_update(self) -> Self {
            self.lock($crate::advanced::RowLock::for_update())
        }

        /// Shorthand for `.lock(RowLock::for_share())`.
        pub fn for_share(self) -> Self {
            self.lock($crate::advanced::RowLock::for_share())
        }
    };
}
pub(crate) use row_lock_methods;
