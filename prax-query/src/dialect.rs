//! Abstraction over SQL dialect differences.
//!
//! Different databases vary in placeholder syntax (`$N`, `?`, `?N`, `@PN`),
//! result-returning clauses (`RETURNING`, `OUTPUT INSERTED`), identifier
//! quoting, upsert syntax, and transaction control keywords. Operations in
//! `prax-query` compose SQL through a `&dyn SqlDialect`, obtained from their
//! bound `QueryEngine` via `engine.dialect()`, so a single `build_sql`
//! emission path serves every backend.

/// Sealed supertrait so only this crate can implement `SqlDialect`.
/// Prevents downstream crates from adding their own `SqlDialect`
/// impls; we reserve the right to add new required methods to the
/// trait without a SemVer break.
mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Postgres {}
    impl Sealed for super::Sqlite {}
    impl Sealed for super::Mysql {}
    impl Sealed for super::Mssql {}
    impl Sealed for super::NotSql {}
}

/// Cross-dialect SQL emission helpers.
///
/// Implementations describe a single database backend's syntax choices.
/// Engines return `&dyn SqlDialect` from `QueryEngine::dialect()`.
pub trait SqlDialect: Send + Sync + sealed::Sealed {
    /// Emit the 1-indexed parameter placeholder for position `i`.
    fn placeholder(&self, i: usize) -> String;

    /// Emit the clause (leading space included) that requests the given
    /// columns be returned after an INSERT/UPDATE/DELETE. Postgres/SQLite/MySQL
    /// emit `RETURNING cols`; MSSQL emits `OUTPUT INSERTED.cols`.
    fn returning_clause(&self, cols: &str) -> String;

    /// Quote a table/column identifier for safe interpolation.
    fn quote_ident(&self, ident: &str) -> String;

    /// Whether the dialect supports `SELECT DISTINCT ON (cols)` (Postgres-only
    /// among our backends today).
    fn supports_distinct_on(&self) -> bool {
        false
    }

    /// Whether an INSERT statement can use the dialect's returning clause to
    /// retrieve inserted rows in-place.
    fn insert_has_returning(&self) -> bool {
        true
    }

    /// Emit the ON CONFLICT / ON DUPLICATE KEY clause (leading space
    /// included) that converts an INSERT into an upsert.
    fn upsert_clause(&self, conflict_cols: &[&str], update_set: &str) -> String;

    /// SQL keyword that begins a transaction. Defaults to `BEGIN`.
    fn begin_sql(&self) -> &'static str {
        "BEGIN"
    }

    /// SQL keyword that commits a transaction. Defaults to `COMMIT`.
    fn commit_sql(&self) -> &'static str {
        "COMMIT"
    }

    /// SQL keyword that rolls back a transaction. Defaults to `ROLLBACK`.
    fn rollback_sql(&self) -> &'static str {
        "ROLLBACK"
    }
}

/// PostgreSQL dialect: `$N` placeholders, `RETURNING`, `"ident"` quoting,
/// `ON CONFLICT (cols) DO UPDATE SET ...` upserts, `DISTINCT ON` support.
pub struct Postgres;

/// SQLite dialect: `?N` placeholders, `RETURNING`, `"ident"` quoting,
/// `ON CONFLICT (cols) DO UPDATE SET ...` upserts.
pub struct Sqlite;

/// MySQL dialect: `?` placeholders (positionless), no `RETURNING`
/// support (that's a MariaDB 10.5+ extension, not MySQL 8.0),
/// backtick-quoted identifiers, `ON DUPLICATE KEY UPDATE ...` upserts.
///
/// Because MySQL can't emit the inserted/updated row in-line, the
/// `MysqlEngine` compensates at the driver layer: inserts look up
/// `LAST_INSERT_ID()` and SELECT back, updates re-run the WHERE as a
/// SELECT. See `prax_mysql::MysqlEngine::execute_insert` /
/// `execute_update` for details.
pub struct Mysql;

/// Microsoft SQL Server dialect: `@PN` placeholders, `OUTPUT INSERTED.*`,
/// bracket-quoted identifiers, `BEGIN/COMMIT/ROLLBACK TRANSACTION`. Upserts
/// require MERGE, which the engine post-processes; the upsert clause emits
/// empty.
pub struct Mssql;

/// Inert dialect for engines that do not emit SQL (document stores such as
/// MongoDB). Every helper returns an empty or identity value. Calling these
/// methods is a bug — no SQL string built from this dialect would be valid
/// against any real database. The driver's own non-SQL operation path should
/// never reach these helpers.
pub struct NotSql;

impl SqlDialect for Postgres {
    fn placeholder(&self, i: usize) -> String {
        format!("${}", i)
    }
    fn returning_clause(&self, cols: &str) -> String {
        format!(" RETURNING {}", cols)
    }
    fn quote_ident(&self, i: &str) -> String {
        format!("\"{}\"", i.replace('"', "\"\""))
    }
    fn supports_distinct_on(&self) -> bool {
        true
    }
    fn upsert_clause(&self, c: &[&str], s: &str) -> String {
        format!(" ON CONFLICT ({}) DO UPDATE SET {}", c.join(", "), s)
    }
}

impl SqlDialect for Sqlite {
    fn placeholder(&self, i: usize) -> String {
        format!("?{}", i)
    }
    fn returning_clause(&self, cols: &str) -> String {
        format!(" RETURNING {}", cols)
    }
    fn quote_ident(&self, i: &str) -> String {
        format!("\"{}\"", i.replace('"', "\"\""))
    }
    fn upsert_clause(&self, c: &[&str], s: &str) -> String {
        format!(" ON CONFLICT ({}) DO UPDATE SET {}", c.join(", "), s)
    }
}

impl SqlDialect for Mysql {
    fn placeholder(&self, _i: usize) -> String {
        "?".into()
    }
    fn returning_clause(&self, _cols: &str) -> String {
        // MySQL 8.0 does NOT support `INSERT ... RETURNING` / `UPDATE ...
        // RETURNING` / `DELETE ... RETURNING`. That syntax only works on
        // MariaDB 10.5+. Emitting it here produces a 1064 syntax error
        // on every insert/update through a typed client.
        //
        // The `MysqlEngine`'s `execute_insert` / `execute_update`
        // implementations compensate by running the DML first, then
        // issuing a follow-up SELECT keyed on `LAST_INSERT_ID()` (for
        // inserts) or re-running the filter (for updates). Returning
        // an empty clause keeps the rest of the build_sql machinery
        // working without driver-specific branches.
        String::new()
    }
    fn insert_has_returning(&self) -> bool {
        false
    }
    fn quote_ident(&self, i: &str) -> String {
        format!("`{}`", i.replace('`', "``"))
    }
    fn upsert_clause(&self, _c: &[&str], s: &str) -> String {
        format!(" ON DUPLICATE KEY UPDATE {}", s)
    }
}

impl SqlDialect for Mssql {
    fn placeholder(&self, i: usize) -> String {
        format!("@P{}", i)
    }
    fn returning_clause(&self, cols: &str) -> String {
        if cols == "*" {
            // OUTPUT INSERTED.* is the only syntactic shortcut T-SQL accepts;
            // bare OUTPUT INSERTED.cols_with_commas would need per-column
            // prefixing, which this branch short-circuits.
            return " OUTPUT INSERTED.*".into();
        }
        let prefixed: Vec<String> = cols
            .split(',')
            .map(|c| format!("INSERTED.{}", c.trim()))
            .collect();
        format!(" OUTPUT {}", prefixed.join(", "))
    }
    fn quote_ident(&self, i: &str) -> String {
        format!("[{}]", i.replace(']', "]]"))
    }
    fn upsert_clause(&self, _c: &[&str], _s: &str) -> String {
        String::new()
    }
    fn begin_sql(&self) -> &'static str {
        "BEGIN TRANSACTION"
    }
    fn commit_sql(&self) -> &'static str {
        "COMMIT TRANSACTION"
    }
    fn rollback_sql(&self) -> &'static str {
        "ROLLBACK TRANSACTION"
    }
}

impl SqlDialect for NotSql {
    fn placeholder(&self, _i: usize) -> String {
        unimplemented!(
            "NotSql dialect does not emit SQL; engines that return NotSql from \
             QueryEngine::dialect() must not route requests through the SQL \
             operation builders (FindManyOperation, CreateOperation, etc.). \
             Use a SQL-capable dialect (Postgres/Mysql/Sqlite/Mssql) or build \
             queries natively (e.g. BSON for MongoDB)."
        )
    }
    fn returning_clause(&self, _cols: &str) -> String {
        unimplemented!("NotSql::returning_clause — see NotSql::placeholder for details")
    }
    fn quote_ident(&self, _ident: &str) -> String {
        unimplemented!("NotSql::quote_ident — see NotSql::placeholder for details")
    }
    fn upsert_clause(&self, _c: &[&str], _s: &str) -> String {
        unimplemented!("NotSql::upsert_clause — see NotSql::placeholder for details")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_per_dialect() {
        assert_eq!(Postgres.placeholder(3), "$3");
        assert_eq!(Sqlite.placeholder(3), "?3");
        assert_eq!(Mysql.placeholder(3), "?");
        assert_eq!(Mssql.placeholder(3), "@P3");
    }

    #[test]
    fn returning_mssql_is_output_inserted() {
        assert_eq!(Mssql.returning_clause("*"), " OUTPUT INSERTED.*");
        assert_eq!(Mssql.returning_clause("id"), " OUTPUT INSERTED.id");
        assert_eq!(
            Mssql.returning_clause("id, email"),
            " OUTPUT INSERTED.id, INSERTED.email"
        );
        assert_eq!(
            Mssql.returning_clause("id,email,name"),
            " OUTPUT INSERTED.id, INSERTED.email, INSERTED.name"
        );
    }

    #[test]
    fn upsert_mysql_is_on_duplicate_key() {
        assert_eq!(
            Mysql.upsert_clause(&[], "x = 1"),
            " ON DUPLICATE KEY UPDATE x = 1"
        );
    }

    #[test]
    fn upsert_postgres_is_on_conflict() {
        assert_eq!(
            Postgres.upsert_clause(&["email"], "name = EXCLUDED.name"),
            " ON CONFLICT (email) DO UPDATE SET name = EXCLUDED.name"
        );
    }

    #[test]
    fn quote_ident_backends_escape_the_embedded_quote() {
        assert_eq!(
            Postgres.quote_ident(r#"col"with"quote"#),
            r#""col""with""quote""#
        );
        assert_eq!(
            Sqlite.quote_ident(r#"col"with"quote"#),
            r#""col""with""quote""#
        );
        assert_eq!(Mysql.quote_ident("co`l"), "`co``l`");
        assert_eq!(Mssql.quote_ident("col]ident"), "[col]]ident]");
    }

    #[test]
    #[should_panic(expected = "NotSql dialect does not emit SQL")]
    fn not_sql_placeholder_panics() {
        let _ = NotSql.placeholder(1);
    }

    #[test]
    #[should_panic]
    fn not_sql_quote_ident_panics() {
        let _ = NotSql.quote_ident("col");
    }

    #[test]
    #[should_panic]
    fn not_sql_returning_clause_panics() {
        let _ = NotSql.returning_clause("*");
    }

    #[test]
    #[should_panic]
    fn not_sql_upsert_clause_panics() {
        let _ = NotSql.upsert_clause(&[], "x = 1");
    }

    #[test]
    fn mssql_transaction_keywords_are_distinct() {
        assert_eq!(Mssql.begin_sql(), "BEGIN TRANSACTION");
        assert_eq!(Mssql.commit_sql(), "COMMIT TRANSACTION");
        assert_eq!(Mssql.rollback_sql(), "ROLLBACK TRANSACTION");
    }

    #[test]
    fn distinct_on_support() {
        assert!(Postgres.supports_distinct_on());
        assert!(!Sqlite.supports_distinct_on());
        assert!(!Mysql.supports_distinct_on());
        assert!(!Mssql.supports_distinct_on());
        assert!(!NotSql.supports_distinct_on());
    }

    #[test]
    fn sealed_pattern_prevents_external_impl() {
        // The sealed supertrait means only types that impl sealed::Sealed
        // can impl SqlDialect. Downstream crates can't access
        // `sealed::Sealed` so they can't add new dialects. This test
        // merely documents the intent; the enforcement is the compiler
        // refusing to accept `impl SqlDialect for MyDialect` outside this
        // crate.
        use crate::dialect::{Postgres, SqlDialect};
        let _p: &dyn SqlDialect = &Postgres;
    }
}
