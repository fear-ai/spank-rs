//! Unified error taxonomy for Spank.
//!
//! Libraries return [`SpankError`]; the binary's `main` may use `anyhow` to
//! collapse them. Every variant carries enough structure to be matched
//! against and reported on.
//!
//! # Recovery classes
//!
//! Each error implicitly belongs to one of four recovery classes; see
//! [`SpankError::recovery`].
//!
//! - `Retryable` — transient I/O, peer reset. Caller should back off and retry.
//! - `Backpressure` — queue full. Caller should shed load (HEC code 9 / 503).
//! - `FatalComponent` — config invalid, port bind. Component fails to start;
//!   the Commander logs and exits non-zero.
//! - `FatalProcess` — invariant violation, panic. Process aborts; supervisor
//!   restarts.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// The recovery class of an error.
///
/// Drives the central reaction in callers: retry, backpressure, fail
/// component, or fail process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    Retryable,
    Backpressure,
    FatalComponent,
    FatalProcess,
}

/// Unified Spank error type.
///
/// Variants are organized by domain. Construct directly or via the `From`
/// impls (see [`From<io::Error>`](#impl-From<io::Error>-for-SpankError)).
#[derive(Debug, Error)]
pub enum SpankError {
    /// Configuration is invalid or could not be loaded.
    #[error("configuration error: {message}")]
    Config { message: String },

    /// I/O error from a syscall, with attribution.
    ///
    /// `syscall` is the operation that failed (e.g. "bind", "accept",
    /// "read"). `target` is the file path or peer address that the call
    /// was operating on. `source` is the underlying `io::Error`.
    #[error("io error in {syscall} on {target}: {source}")]
    Io {
        syscall: &'static str,
        target: String,
        #[source]
        source: io::Error,
    },

    /// HEC protocol error — produced by the receiver when a request
    /// cannot be served. Carries the wire-format `code` and `text`.
    #[error("hec error: code={code} text={text}")]
    Hec {
        code: u32,
        text: String,
        http_status: u16,
    },

    /// Storage backend error.
    #[error("storage error: {message}")]
    Storage { message: String },

    /// Authentication or authorization error.
    #[error("auth error: {message}")]
    Auth { message: String },

    /// Lifecycle error — startup failure, shutdown timeout, etc.
    #[error("lifecycle error: {message}")]
    Lifecycle { message: String },

    /// Bounded-queue rejection — the producer should shed load.
    #[error("queue full: {queue}")]
    QueueFull { queue: &'static str },

    /// Invariant violation. Caller should not see this in normal operation.
    #[error("internal error: {message}")]
    Internal { message: String },
}

impl SpankError {
    /// Convenience constructor for I/O errors that records the syscall
    /// name and the path involved.
    #[must_use]
    pub fn io(syscall: &'static str, target: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            syscall,
            target: target.into(),
            source,
        }
    }

    /// Convenience constructor for I/O errors with a `PathBuf` target.
    #[must_use]
    pub fn io_path(syscall: &'static str, path: &PathBuf, source: io::Error) -> Self {
        Self::Io {
            syscall,
            target: path.display().to_string(),
            source,
        }
    }

    /// Recovery class for this error.
    ///
    /// The caller uses this to choose between retry, backpressure, fail
    /// component, and fail process. Sites that want a different class
    /// should construct a different variant rather than override the
    /// classification here.
    #[must_use]
    pub fn recovery(&self) -> Recovery {
        match self {
            Self::Config { .. } | Self::Lifecycle { .. } => Recovery::FatalComponent,
            Self::Io { source, .. } => match source.kind() {
                io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::TimedOut
                | io::ErrorKind::Interrupted
                | io::ErrorKind::WouldBlock => Recovery::Retryable,
                _ => Recovery::FatalComponent,
            },
            Self::Hec { .. } | Self::Auth { .. } | Self::Storage { .. } => Recovery::Retryable,
            Self::QueueFull { .. } => Recovery::Backpressure,
            Self::Internal { .. } => Recovery::FatalProcess,
        }
    }
}

/// Project-wide `Result` alias.
pub type Result<T> = std::result::Result<T, SpankError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_recovery_matches_kind() {
        let err = SpankError::io(
            "read",
            "/tmp/x",
            io::Error::new(io::ErrorKind::ConnectionReset, "peer"),
        );
        assert_eq!(err.recovery(), Recovery::Retryable);

        let err = SpankError::io(
            "bind",
            "0.0.0.0:9000",
            io::Error::new(io::ErrorKind::AddrInUse, "in use"),
        );
        assert_eq!(err.recovery(), Recovery::FatalComponent);
    }

    #[test]
    fn queue_full_is_backpressure() {
        let err = SpankError::QueueFull {
            queue: "indexer",
        };
        assert_eq!(err.recovery(), Recovery::Backpressure);
    }
}
