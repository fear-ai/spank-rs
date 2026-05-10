//! Backend selector.
//!
//! `Sqlite` is the only fully-implemented backend. `DuckDb` and
//! `Postgres` are feature-gated variants; building without their
//! feature flag compiles the enum variant but routes to a stub that
//! returns a `Storage` error at runtime. When the real implementation
//! lands, the stub arms are replaced — the enum shape and the
//! `PartitionManager` dispatch do not change.
//!
//! Feature flags: `duckdb`, `postgres` on this crate.

use spank_cfg::StoreBackend;
use spank_core::{Result, SpankError};

use crate::sqlite::SqliteBackend;
use crate::traits::PartitionManager;

pub enum Backend {
    Sqlite(SqliteBackend),
    #[cfg(feature = "duckdb")]
    DuckDb(DuckDbBackend),
    #[cfg(not(feature = "duckdb"))]
    DuckDb,
    #[cfg(feature = "postgres")]
    Postgres(PostgresBackend),
    #[cfg(not(feature = "postgres"))]
    Postgres,
}

impl Backend {
    /// Open the backend indicated by `kind` rooted at `root` (or `dsn`
    /// for Postgres). Returns `SpankError::Storage` for backends whose
    /// feature flag is not enabled.
    pub fn open(kind: StoreBackend, root: &std::path::Path) -> Result<Self> {
        match kind {
            StoreBackend::Sqlite => Ok(Self::Sqlite(SqliteBackend::open(root)?)),
            StoreBackend::DuckDb => {
                #[cfg(feature = "duckdb")]
                { Ok(Self::DuckDb(DuckDbBackend::open(root)?)) }
                #[cfg(not(feature = "duckdb"))]
                { Err(SpankError::Storage {
                    message: "duckdb backend not enabled; rebuild with feature `duckdb`".into(),
                }) }
            }
            StoreBackend::Postgres => {
                #[cfg(feature = "postgres")]
                { Ok(Self::Postgres(PostgresBackend::open(root)?)) }
                #[cfg(not(feature = "postgres"))]
                { Err(SpankError::Storage {
                    message: "postgres backend not enabled; rebuild with feature `postgres`".into(),
                }) }
            }
        }
    }

    #[must_use]
    pub fn as_partition_manager(&self) -> &dyn PartitionManager {
        match self {
            Self::Sqlite(b) => b,
            #[cfg(feature = "duckdb")]
            Self::DuckDb(b) => b,
            #[cfg(not(feature = "duckdb"))]
            Self::DuckDb => unreachable!("DuckDb variant constructed only with feature flag"),
            #[cfg(feature = "postgres")]
            Self::Postgres(b) => b,
            #[cfg(not(feature = "postgres"))]
            Self::Postgres => unreachable!("Postgres variant constructed only with feature flag"),
        }
    }
}

// ---------------------------------------------------------------------------
// Stub types — present when the feature flag is absent so the code compiles.
// Replaced by real implementations when the flag is enabled.
// ---------------------------------------------------------------------------

/// DuckDB backend stub. Present when the `duckdb` feature is disabled.
/// The real implementation replaces this struct and its `PartitionManager`
/// impl when the feature is enabled.
#[cfg(not(feature = "duckdb"))]
pub struct DuckDbBackend;

/// Postgres backend stub. Present when the `postgres` feature is disabled.
#[cfg(not(feature = "postgres"))]
pub struct PostgresBackend;
