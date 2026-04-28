//! Backend selector.
//!
//! Today only `Sqlite` is real; `DuckDb` and `Postgres` are scaffolded
//! and return `Storage` errors when constructed without their feature
//! flag.

use spank_cfg::StoreBackend;
use spank_core::{Result, SpankError};

use crate::sqlite::SqliteBackend;
use crate::traits::PartitionManager;

pub enum Backend {
    Sqlite(SqliteBackend),
}

impl Backend {
    pub fn open(kind: StoreBackend, root: &std::path::Path) -> Result<Self> {
        match kind {
            StoreBackend::Sqlite => Ok(Self::Sqlite(SqliteBackend::open(root)?)),
        }
    }

    #[must_use]
    pub fn as_partition_manager(&self) -> &dyn PartitionManager {
        match self {
            Self::Sqlite(b) => b,
        }
    }
}

/// Stub for DuckDB. Returns Storage error until the `duckdb` feature
/// is implemented.
pub fn open_duckdb(_root: &std::path::Path) -> Result<()> {
    Err(SpankError::Storage {
        message: "duckdb backend not yet implemented (feature gated)".into(),
    })
}

/// Stub for Postgres. Returns Storage error until the `postgres`
/// feature is implemented.
pub fn open_postgres(_dsn: &str) -> Result<()> {
    Err(SpankError::Storage {
        message: "postgres backend not yet implemented (feature gated)".into(),
    })
}
