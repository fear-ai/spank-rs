//! Storage contract.
//!
//! The trio `BucketWriter`/`BucketReader`/`PartitionManager` is the
//! Spast §4.4 contract. Backends pass or they are not backends; the
//! conformance tests in `tests/conformance/` are the assertion.

use spank_core::{Result, Rows};

/// Append-only writer for a single bucket.
pub trait BucketWriter: Send + Sync {
    /// Insert rows. The implementation is free to batch internally;
    /// callers pass whatever chunk size is convenient.
    fn append(&mut self, rows: &Rows) -> Result<usize>;

    /// Commit any pending work. Called at checkpoint and before close.
    fn commit(&mut self) -> Result<()>;

    /// Close the bucket. After this, no further `append` is allowed.
    fn close(self: Box<Self>) -> Result<()>;
}

/// Random-read access for query.
pub trait BucketReader: Send + Sync {
    /// Total row count (cheap; index lookup).
    fn count(&self) -> Result<u64>;

    /// Stream rows matching a simple time-range filter. Backends are
    /// expected to push the predicate to native SQL.
    fn scan_time_range(&self, from_ns: i64, to_ns: i64) -> Result<Rows>;
}

/// Partition lifecycle (HOT → WARM → COLD).
pub trait PartitionManager: Send + Sync {
    /// Create a new HOT bucket and return a writer.
    fn create_hot(&self, name: &str) -> Result<Box<dyn BucketWriter>>;

    /// Open an existing bucket for reading.
    fn open_reader(&self, name: &str) -> Result<Box<dyn BucketReader>>;

    /// List bucket names known to this manager.
    fn list(&self) -> Result<Vec<String>>;
}
