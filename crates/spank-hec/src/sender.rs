//! `Sender` ABC and `FileSender` implementation.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use spank_core::{Result, Rows, SpankError};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};

/// `Sender` trait — egress for `Rows`.
///
/// Concrete impls: `FileSender` (debug, this crate), `Forwarder`
/// (HEC egress, `spank-shipper::HecSender` later), `TcpSender`
/// (`spank-shipper::TcpSender`).
pub trait Sender: Send + Sync + 'static {
    /// Submit rows. Implementations must not block long; use a
    /// bounded channel internally if necessary.
    fn submit(&self, rows: Rows) -> Result<()>;

    /// Flush any buffered rows tagged with `tag` and return when
    /// they have reached durable state. Called by the indexing loop
    /// on Sentinel receipt.
    fn flush(&self, tag: &str) -> Result<()>;
}

/// File-based sender. Writes JSON-lines to a per-tag file under
/// `output_dir`. Useful as the destination during HEC bring-up.
pub struct FileSender {
    output_dir: PathBuf,
    inner: Arc<Mutex<HashMap<String, BufWriter<File>>>>,
}

impl FileSender {
    /// Create a `FileSender` rooted at `output_dir`. The directory is
    /// created if it does not exist.
    ///
    /// # Errors
    /// Returns `SpankError::Io { syscall: "create_dir_all", .. }` if
    /// the directory cannot be created.
    pub fn new(output_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&output_dir)
            .map_err(|e| SpankError::io_path("create_dir_all", &output_dir, e))?;
        Ok(Self {
            output_dir,
            inner: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn writer_for(&self, tag: &str) -> Result<()> {
        let mut map = self.inner.lock();
        if map.contains_key(tag) {
            return Ok(());
        }
        let file_name = sanitize(tag);
        let path = self.output_dir.join(format!("{file_name}.jsonl"));
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| SpankError::io_path("open", &path, e))?;
        map.insert(tag.to_string(), BufWriter::new(f));
        Ok(())
    }
}

impl Sender for FileSender {
    fn submit(&self, rows: Rows) -> Result<()> {
        // Group rows by source so each goes to its own tag-file.
        let mut by_source: HashMap<String, Vec<&spank_core::Record>> = HashMap::new();
        for r in &rows {
            let key = if r.source.is_empty() { "default" } else { r.source.as_str() };
            by_source.entry(key.to_string()).or_default().push(r);
        }
        for (tag, recs) in by_source {
            self.writer_for(&tag)?;
            let mut map = self.inner.lock();
            let w = map.get_mut(&tag).expect("just inserted");
            for r in recs {
                let line = serde_json::to_string(r).map_err(|e| SpankError::Internal {
                    message: format!("serialize record: {e}"),
                })?;
                w.write_all(line.as_bytes())
                    .map_err(|e| SpankError::io("write", tag.clone(), e))?;
                w.write_all(b"\n")
                    .map_err(|e| SpankError::io("write", tag.clone(), e))?;
            }
        }
        Ok(())
    }

    fn flush(&self, tag: &str) -> Result<()> {
        let mut map = self.inner.lock();
        if let Some(w) = map.get_mut(tag) {
            w.flush()
                .map_err(|e| SpankError::io("flush", tag.to_string(), e))?;
            w.get_ref()
                .sync_all()
                .map_err(|e| SpankError::io("fsync", tag.to_string(), e))?;
        }
        Ok(())
    }
}

fn sanitize(tag: &str) -> String {
    tag.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
