//! Lifecycle primitives: a `CancellationToken` hierarchy rooted at
//! the Commander.
//!
//! Every long-lived task receives a token. The Commander's token is
//! the root; subsystems call `child_token()` to derive their own;
//! workers derive again. Cancelling a parent cancels every child.

use tokio_util::sync::CancellationToken;

/// Root token plus a name, for tracing.
#[derive(Clone)]
pub struct Lifecycle {
    pub name: &'static str,
    pub token: CancellationToken,
}

impl Lifecycle {
    #[must_use]
    pub fn root() -> Self {
        Self {
            name: "commander",
            token: CancellationToken::new(),
        }
    }

    #[must_use]
    pub fn child(&self, name: &'static str) -> Self {
        Self {
            name,
            token: self.token.child_token(),
        }
    }

    /// Trigger shutdown for this lifecycle and every descendant.
    pub fn shutdown(&self) {
        tracing::info!(target: "lifecycle", name = %self.name, "shutdown signaled");
        self.token.cancel();
    }
}
