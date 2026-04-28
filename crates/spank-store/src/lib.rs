//! `spank-store` — generic storage layer.
//!
//! Defines the `BucketWriter`, `BucketReader`, and `PartitionManager`
//! traits and a `SqliteBackend` reference implementation tuned for
//! bulk insert (BEGIN IMMEDIATE, prepared INSERT, WAL, NORMAL,
//! MEMORY temp_store).
//!
//! `DuckDb` and `Postgres` adapters are scaffolded behind feature
//! flags to keep default builds free of those toolchains.

pub mod backend;
pub mod sqlite;
pub mod traits;

pub use backend::Backend;
pub use sqlite::SqliteBackend;
pub use traits::{BucketReader, BucketWriter, PartitionManager};
