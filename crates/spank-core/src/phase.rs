//! HEC server lifecycle phase.
//!
//! See `docs/Sparst.md` §11.6. Members are UPPER_SNAKE; `STARTED` and
//! `STOPPING` are transitional, `SERVING` and `DEGRADED` are steady.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[allow(non_camel_case_types)]
pub enum HecPhase {
    /// Process started, subsystems still initializing.
    STARTED,
    /// Accepting requests; healthy.
    SERVING,
    /// Accepting requests but downstream is stressed (queue full,
    /// backend error). `/health` reports degraded; receivers may
    /// shed load with HEC code 9.
    DEGRADED,
    /// Stop signal received; draining and shutting down.
    STOPPING,
}

impl HecPhase {
    /// Whether this phase admits new work.
    #[must_use]
    pub fn admits_work(self) -> bool {
        matches!(self, Self::SERVING | Self::DEGRADED)
    }

    /// Whether the transition `self -> next` is allowed.
    ///
    /// This list is the source of truth for the state machine and is
    /// the basis for the diagram in `docs/uml/hec-readiness.puml`.
    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        use HecPhase::{DEGRADED, SERVING, STARTED, STOPPING};
        matches!(
            (self, next),
            (STARTED, SERVING)
                | (STARTED, DEGRADED)
                | (SERVING, DEGRADED)
                | (DEGRADED, SERVING)
                | (SERVING, STOPPING)
                | (DEGRADED, STOPPING)
                | (STARTED, STOPPING)
        )
    }
}
