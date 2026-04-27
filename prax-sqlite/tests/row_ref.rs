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
