//! SQLite backend tuned for bulk ingest.
//!
//! Tunings applied at open time per Pyst §15:
//! - `journal_mode = WAL`
//! - `synchronous = NORMAL`
//! - `temp_store = MEMORY`
//! - `mmap_size = 256MB`
//! - `cache_size = -64000` (64 MiB)
//!
//! Bulk insert pattern:
//! - Single transaction (`BEGIN IMMEDIATE`).
//! - Prepared statement reused across rows.
//! - `commit` flushes; the WAL fsync is governed by the journal mode.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OpenFlags};
use spank_core::{Record, Result, Rows, SpankError};

use crate::traits::{BucketReader, BucketWriter, PartitionManager};

const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    time_event_ns INTEGER NOT NULL,
    time_index_ns INTEGER NOT NULL,
    raw TEXT NOT NULL,
    source TEXT NOT NULL,
    sourcetype TEXT NOT NULL,
    host TEXT NOT NULL,
    idx TEXT NOT NULL,
    fields_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_time_event ON events(time_event_ns);
";

/// SQLite-rooted partition manager.
pub struct SqliteBackend {
    pub root: PathBuf,
}

impl SqliteBackend {
    /// Open a backend rooted at `root`. The directory is created.
    ///
    /// # Errors
    /// `SpankError::Io` if the directory cannot be created.
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)
            .map_err(|e| SpankError::io_path("create_dir_all", &root.to_path_buf(), e))?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    fn bucket_path(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.sqlite"))
    }
}

fn tune(conn: &Connection) -> Result<()> {
    let pragmas = [
        "PRAGMA journal_mode = WAL",
        "PRAGMA synchronous = NORMAL",
        "PRAGMA temp_store = MEMORY",
        "PRAGMA mmap_size = 268435456",
        "PRAGMA cache_size = -64000",
    ];
    for p in pragmas {
        conn.execute_batch(p)
            .map_err(|e| SpankError::Storage {
                message: format!("pragma {p}: {e}"),
            })?;
    }
    Ok(())
}

impl PartitionManager for SqliteBackend {
    fn create_hot(&self, name: &str) -> Result<Box<dyn BucketWriter>> {
        let path = self.bucket_path(name);
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| SpankError::Storage {
            message: format!("open {}: {e}", path.display()),
        })?;
        tune(&conn)?;
        conn.execute_batch(SCHEMA).map_err(|e| SpankError::Storage {
            message: format!("schema: {e}"),
        })?;
        Ok(Box::new(SqliteWriter {
            conn: Mutex::new(Some(conn)),
            in_txn: false,
        }))
    }

    fn open_reader(&self, name: &str) -> Result<Box<dyn BucketReader>> {
        let path = self.bucket_path(name);
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(
            |e| SpankError::Storage {
                message: format!("open_ro {}: {e}", path.display()),
            },
        )?;
        Ok(Box::new(SqliteReader { conn: Mutex::new(conn) }))
    }

    fn list(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let rd = std::fs::read_dir(&self.root)
            .map_err(|e| SpankError::io_path("read_dir", &self.root, e))?;
        for ent in rd {
            let ent = ent.map_err(|e| SpankError::io_path("read_dir_entry", &self.root, e))?;
            let p = ent.path();
            if p.extension().and_then(|s| s.to_str()) == Some("sqlite") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.push(stem.to_string());
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

pub struct SqliteWriter {
    conn: Mutex<Option<Connection>>,
    in_txn: bool,
}

impl BucketWriter for SqliteWriter {
    fn append(&mut self, rows: &Rows) -> Result<usize> {
        let mut guard = self.conn.lock().expect("poisoned");
        let conn = guard.as_mut().ok_or_else(|| SpankError::Storage {
            message: "writer closed".into(),
        })?;
        if !self.in_txn {
            conn.execute_batch("BEGIN IMMEDIATE")
                .map_err(|e| SpankError::Storage {
                    message: format!("begin: {e}"),
                })?;
            self.in_txn = true;
        }
        let mut stmt = conn
            .prepare_cached(
                "INSERT INTO events
                 (time_event_ns, time_index_ns, raw, source, sourcetype, host, idx, fields_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|e| SpankError::Storage {
                message: format!("prepare: {e}"),
            })?;
        let mut n = 0usize;
        for r in rows {
            let fields = serde_json::to_string(&r.fields).map_err(|e| SpankError::Storage {
                message: format!("fields json: {e}"),
            })?;
            stmt.execute(params![
                r.time_event_ns,
                r.time_index_ns,
                r.raw,
                r.source,
                r.sourcetype,
                r.host,
                r.index,
                fields
            ])
            .map_err(|e| SpankError::Storage {
                message: format!("insert: {e}"),
            })?;
            n += 1;
        }
        metrics::counter!(spank_obs::metrics::names::STORE_INSERTS_TOTAL).increment(n as u64);
        Ok(n)
    }

    fn commit(&mut self) -> Result<()> {
        if !self.in_txn {
            return Ok(());
        }
        let mut guard = self.conn.lock().expect("poisoned");
        let conn = guard.as_mut().ok_or_else(|| SpankError::Storage {
            message: "writer closed".into(),
        })?;
        conn.execute_batch("COMMIT")
            .map_err(|e| SpankError::Storage {
                message: format!("commit: {e}"),
            })?;
        self.in_txn = false;
        Ok(())
    }

    fn close(mut self: Box<Self>) -> Result<()> {
        if self.in_txn {
            self.commit()?;
        }
        let mut guard = self.conn.lock().expect("poisoned");
        guard.take(); // drop connection
        Ok(())
    }
}

pub struct SqliteReader {
    conn: Mutex<Connection>,
}

impl BucketReader for SqliteReader {
    fn count(&self) -> Result<u64> {
        let conn = self.conn.lock().expect("poisoned");
        let n: i64 = conn
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .map_err(|e| SpankError::Storage {
                message: format!("count: {e}"),
            })?;
        Ok(n as u64)
    }

    fn scan_time_range(&self, from_ns: i64, to_ns: i64) -> Result<Rows> {
        let conn = self.conn.lock().expect("poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT time_event_ns, time_index_ns, raw, source, sourcetype, host, idx, fields_json
                 FROM events WHERE time_event_ns >= ?1 AND time_event_ns < ?2
                 ORDER BY time_event_ns",
            )
            .map_err(|e| SpankError::Storage { message: format!("prepare scan: {e}") })?;
        let rows_iter = stmt
            .query_map(params![from_ns, to_ns], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(|e| SpankError::Storage { message: format!("query: {e}") })?;

        let mut out = Rows::new();
        for row in rows_iter {
            let (te, ti, raw, source, sourcetype, host, idx, fields_json) =
                row.map_err(|e| SpankError::Storage { message: format!("row: {e}") })?;
            let fields: std::collections::BTreeMap<String, String> =
                serde_json::from_str(&fields_json).unwrap_or_default();
            out.push(Record {
                time_event_ns: te,
                time_index_ns: ti,
                raw,
                source,
                sourcetype,
                host,
                index: idx,
                fields,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SqliteBackend::open(dir.path()).unwrap();
        let mut w = backend.create_hot("b1").unwrap();
        let rows = vec![Record::builder("hello")
            .time_event_ns(100)
            .time_index_ns(101)
            .build()];
        w.append(&rows).unwrap();
        w.commit().unwrap();
        w.close().unwrap();
        let r = backend.open_reader("b1").unwrap();
        assert_eq!(r.count().unwrap(), 1);
        let scan = r.scan_time_range(0, 200).unwrap();
        assert_eq!(scan.len(), 1);
        assert_eq!(scan[0].raw, "hello");
    }
}
