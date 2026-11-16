//! Async handle to a [`rusqlite::Connection`] running on its own thread.
//!
//! `rusqlite::Connection` is blocking and `!Sync`, so every connection is
//! owned by a dedicated background thread. [`AsyncConnection`] is a cheap,
//! cloneable handle that sends closures to that thread and awaits their
//! results. Closures run one at a time, in the order they were sent, which
//! the transaction code relies on (see `engine.rs`).
//!
//! This replaces the `tokio-rusqlite` dependency used before 0.13.1: no
//! `tokio-rusqlite` release supports rusqlite 0.41, the first rusqlite whose
//! bundled SQLite is patched for RUSTSEC-2026-0459.

use std::fmt;
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use tokio::sync::oneshot;

type Job = Box<dyn FnOnce(&mut rusqlite::Connection) + Send + 'static>;

/// Error returned by [`AsyncConnection`] operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum AsyncConnectionError {
    /// The connection's background thread has stopped (the connection was
    /// closed, or a previous closure panicked), so it can't run queries.
    ConnectionClosed,
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
}

impl fmt::Display for AsyncConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionClosed => write!(f, "SQLite connection is closed"),
            Self::Sqlite(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AsyncConnectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ConnectionClosed => None,
            Self::Sqlite(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for AsyncConnectionError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

/// Cloneable async handle to a SQLite connection on a background thread.
///
/// The connection closes when the last handle is dropped.
#[derive(Clone)]
pub struct AsyncConnection {
    sender: mpsc::Sender<Job>,
}

impl fmt::Debug for AsyncConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsyncConnection").finish_non_exhaustive()
    }
}

impl AsyncConnection {
    /// Open (or create) the database file at `path`.
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self, AsyncConnectionError> {
        let path = path.as_ref().to_owned();
        Self::start(move || rusqlite::Connection::open(path)).await
    }

    /// Open a new private in-memory database.
    pub async fn open_in_memory() -> Result<Self, AsyncConnectionError> {
        Self::start(rusqlite::Connection::open_in_memory).await
    }

    /// Open `path` with explicit [`rusqlite::OpenFlags`].
    pub async fn open_with_flags<P: AsRef<Path>>(
        path: P,
        flags: rusqlite::OpenFlags,
    ) -> Result<Self, AsyncConnectionError> {
        let path = path.as_ref().to_owned();
        Self::start(move || rusqlite::Connection::open_with_flags(path, flags)).await
    }

    /// Wrap an already-open blocking connection. It moves to a new
    /// background thread.
    pub fn from_connection(conn: rusqlite::Connection) -> Self {
        let (sender, receiver) = mpsc::channel::<Job>();
        thread::spawn(move || run(conn, receiver));
        Self { sender }
    }

    async fn start<F>(open: F) -> Result<Self, AsyncConnectionError>
    where
        F: FnOnce() -> rusqlite::Result<rusqlite::Connection> + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel::<Job>();
        let (opened_tx, opened_rx) = oneshot::channel::<rusqlite::Result<()>>();
        thread::spawn(move || match open() {
            Ok(conn) => {
                if opened_tx.send(Ok(())).is_ok() {
                    run(conn, receiver);
                }
            }
            Err(e) => {
                let _ = opened_tx.send(Err(e));
            }
        });
        opened_rx
            .await
            .map_err(|_| AsyncConnectionError::ConnectionClosed)??;
        Ok(Self { sender })
    }

    /// Run `f` on the connection's thread and return its result.
    ///
    /// Closures run sequentially in submission order. Returns
    /// [`AsyncConnectionError::ConnectionClosed`] if the background thread
    /// has stopped, including when `f` panics.
    pub async fn call<F, R>(&self, f: F) -> Result<R, AsyncConnectionError>
    where
        F: FnOnce(&mut rusqlite::Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let (result_tx, result_rx) = oneshot::channel::<rusqlite::Result<R>>();
        let job: Job = Box::new(move |conn| {
            let _ = result_tx.send(f(conn));
        });
        self.sender
            .send(job)
            .map_err(|_| AsyncConnectionError::ConnectionClosed)?;
        let result = result_rx
            .await
            .map_err(|_| AsyncConnectionError::ConnectionClosed)?;
        Ok(result?)
    }
}

/// Background thread body: run jobs until every handle is dropped.
fn run(mut conn: rusqlite::Connection, receiver: mpsc::Receiver<Job>) {
    while let Ok(job) = receiver.recv() {
        job(&mut conn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn call_runs_on_the_connection_and_returns_results() {
        let conn = AsyncConnection::open_in_memory().await.unwrap();
        conn.call(|c| c.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)"))
            .await
            .unwrap();
        let id = conn
            .call(|c| {
                c.execute("INSERT INTO t (name) VALUES (?1)", ["a"])?;
                Ok(c.last_insert_rowid())
            })
            .await
            .unwrap();
        assert_eq!(id, 1);
        let name: String = conn
            .call(|c| c.query_row("SELECT name FROM t WHERE id = 1", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(name, "a");
    }

    #[tokio::test]
    async fn clones_share_one_connection_in_order() {
        let conn = AsyncConnection::open_in_memory().await.unwrap();
        let other = conn.clone();
        conn.call(|c| c.execute_batch("CREATE TABLE t (n INTEGER)"))
            .await
            .unwrap();
        let mut pending = Vec::new();
        for n in 0..50_i64 {
            let h = if n % 2 == 0 {
                conn.clone()
            } else {
                other.clone()
            };
            pending.push(async move {
                h.call(move |c| c.execute("INSERT INTO t (n) VALUES (?1)", [n]))
                    .await
            });
        }
        for p in pending {
            p.await.unwrap();
        }
        let rows: Vec<i64> = other
            .call(|c| {
                let mut stmt = c.prepare("SELECT n FROM t ORDER BY rowid")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                rows.collect()
            })
            .await
            .unwrap();
        assert_eq!(rows, (0..50).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn sqlite_errors_are_reported() {
        let conn = AsyncConnection::open_in_memory().await.unwrap();
        let err = conn
            .call(|c| c.execute_batch("NOT VALID SQL"))
            .await
            .unwrap_err();
        assert!(matches!(err, AsyncConnectionError::Sqlite(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_panicking_closure_closes_the_connection() {
        let conn = AsyncConnection::open_in_memory().await.unwrap();
        let err = conn
            .call(|_| -> rusqlite::Result<()> { panic!("boom") })
            .await
            .unwrap_err();
        assert!(matches!(err, AsyncConnectionError::ConnectionClosed));
        let err = conn.call(|_| Ok(())).await.unwrap_err();
        assert!(matches!(err, AsyncConnectionError::ConnectionClosed));
    }

    #[tokio::test]
    async fn open_reports_errors() {
        let err = AsyncConnection::open_with_flags(
            "/nonexistent-dir/prax/never.db",
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AsyncConnectionError::Sqlite(_)), "{err:?}");
    }

    #[tokio::test]
    async fn from_connection_wraps_an_open_connection() {
        let raw = rusqlite::Connection::open_in_memory().unwrap();
        raw.execute_batch("CREATE TABLE t (n INTEGER); INSERT INTO t VALUES (7)")
            .unwrap();
        let conn = AsyncConnection::from_connection(raw);
        let n: i64 = conn
            .call(|c| c.query_row("SELECT n FROM t", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 7);
    }

    #[test]
    fn handle_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AsyncConnection>();
    }
}
