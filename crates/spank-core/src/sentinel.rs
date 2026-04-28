//! Sentinel — end/checkpoint marker enqueued at source termination.
//!
//! See `docs/Sparst.md` §8.4 for the design.

use serde::{Deserialize, Serialize};

/// Kind of sentinel marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SentinelKind {
    /// Source finished; flush buffers and signal `Drain`.
    End,
    /// Mid-stream checkpoint; flush buffers and signal `Drain`, but
    /// the source continues. Reserved — not produced or consumed yet.
    Checkpoint,
}

/// End/checkpoint marker carried through ingest channels.
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

    #[must_use]
    pub fn checkpoint(tag: impl Into<String>) -> Self {
        Self {
            kind: SentinelKind::Checkpoint,
            tag: tag.into(),
        }
    }
}
