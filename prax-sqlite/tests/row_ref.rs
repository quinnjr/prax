use prax_query::row::RowRef;
use prax_sqlite::row_ref::SqliteRowRef;
use rusqlite::Connection;

#[test]
fn materializes_row_from_rusqlite() {
    let conn = Connection::open_in_memory().unwrap();
    let mut stmt = conn.prepare("SELECT 42 AS n, 'hello' AS s").unwrap();
    let mut rows = stmt.query([]).unwrap();
    let row = rows.next().unwrap().unwrap();
    let r = SqliteRowRef::from_rusqlite(row).unwrap();
    assert_eq!(r.get_i32("n").unwrap(), 42);
    assert_eq!(r.get_str("s").unwrap(), "hello");
}

#[test]
fn materializes_naive_temporal_values() {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    let conn = Connection::open_in_memory().unwrap();
    let mut stmt = conn
        .prepare("SELECT '2026-04-27T15:30:45' AS dt, '2026-04-27' AS d, '15:30:45' AS t")
        .unwrap();
    let mut rows = stmt.query([]).unwrap();
    let row = rows.next().unwrap().unwrap();
    let r = SqliteRowRef::from_rusqlite(row).unwrap();
    assert_eq!(
        r.get_naive_datetime("dt").unwrap(),
        NaiveDateTime::parse_from_str("2026-04-27 15:30:45", "%Y-%m-%d %H:%M:%S").unwrap()
    );
    assert_eq!(
        r.get_naive_date("d").unwrap(),
        NaiveDate::from_ymd_opt(2026, 4, 27).unwrap()
    );
    assert_eq!(
        r.get_naive_time("t").unwrap(),
        NaiveTime::from_hms_opt(15, 30, 45).unwrap()
    );
}

#[test]
fn opt_methods_distinguish_missing_column_from_null() {
    let conn = Connection::open_in_memory().unwrap();
    let mut stmt = conn
        .prepare("SELECT 42 AS present, NULL AS nulled")
        .unwrap();
    let mut rows = stmt.query([]).unwrap();
    let row = rows.next().unwrap().unwrap();
    let r = SqliteRowRef::from_rusqlite(row).unwrap();

    // Present column with a value → Ok(Some(_)).
    assert_eq!(r.get_i32_opt("present").unwrap(), Some(42));

    // Present column whose value is NULL → Ok(None).
    assert_eq!(r.get_i32_opt("nulled").unwrap(), None);

    // Absent column (typo / not in the SELECT list) → Err(ColumnNotFound).
    let err = r.get_i32_opt("missing").unwrap_err();
    assert!(
        matches!(err, prax_query::row::RowError::ColumnNotFound(ref col) if col == "missing"),
        "expected ColumnNotFound for absent column, got {err:?}",
    );
}
