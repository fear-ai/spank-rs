//! Sentinel — end marker enqueued at source termination.
//!
//! See `docs/Sparst.md` §8.4 for the design.
//!
//! `SentinelKind::Checkpoint` was removed (Plan.md SENT-CHKPT1). Re-add
//! when a mid-stream checkpoint use case (e.g. periodic WAL flush
//! confirmation) is concretely designed.

use serde::{Deserialize, Serialize};

/// Kind of sentinel marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SentinelKind {
    /// Source finished; flush buffers and signal `Drain`.
    End,
}

/// End marker carried through ingest channels.
///
/// On receipt, the indexing loop flushes buffers tagged with this
/// `tag` and signals the matching [`crate::Drain`] entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sentinel {
    pub kind: SentinelKind,
    /// Source identifier — channel id, file path, run id.
    pub tag: String,
}

impl Sentinel {
    #[must_use]
    pub fn end(tag: impl Into<String>) -> Self {
        Self {
            kind: SentinelKind::End,
            tag: tag.into(),
        }
    }
}
