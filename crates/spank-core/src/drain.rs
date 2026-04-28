//! Drain — wait-side handle for source completion.
//!
//! A `Drain` exposes `wait(tag)` for callers that need to block until a
//! named source has been fully ingested and committed. The indexing
//! loop calls `signal(tag)` when it processes the matching `Sentinel`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;

#[derive(Default)]
struct Inner {
    /// Tag → notifier. The notifier is permanent; multiple waiters
    /// share the same `Notify` and all are released by `notify_waiters`.
    /// `signaled` carries the latched fact that the tag has fired so
    /// late waiters return immediately.
    notifiers: HashMap<String, Arc<Notify>>,
    signaled: HashMap<String, ()>,
}

/// Wait-side handle for source completion.
///
/// Cloning this handle shares state. There is one `Drain` per
/// indexer; producers and waiters use the same instance.
#[derive(Clone, Default)]
pub struct Drain {
    inner: Arc<Mutex<Inner>>,
}

impl Drain {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn notifier(&self, tag: &str) -> Arc<Notify> {
        let mut inner = self.inner.lock();
        if let Some(n) = inner.notifiers.get(tag) {
            return n.clone();
        }
        let n = Arc::new(Notify::new());
        inner.notifiers.insert(tag.to_string(), n.clone());
        n
    }

    /// Block until `tag` is signaled or `timeout` elapses.
    ///
    /// Returns `true` if the tag fired, `false` on timeout.
    pub async fn wait(&self, tag: &str, timeout: Option<Duration>) -> bool {
        // Fast-path: already signaled.
        {
            let inner = self.inner.lock();
            if inner.signaled.contains_key(tag) {
                return true;
            }
        }
        let n = self.notifier(tag);
        let notified = n.notified();
        // Re-check after subscribing to avoid lost wake-ups.
        {
            let inner = self.inner.lock();
            if inner.signaled.contains_key(tag) {
                return true;
            }
        }
        match timeout {
            Some(d) => tokio::time::timeout(d, notified).await.is_ok(),
            None => {
                notified.await;
                true
            }
        }
    }

    /// Mark `tag` as signaled and wake all waiters. Called by the
    /// indexing loop on Sentinel receipt.
    pub fn signal(&self, tag: &str) {
        let n = {
            let mut inner = self.inner.lock();
            inner.signaled.insert(tag.to_string(), ());
            inner.notifiers.get(tag).cloned()
        };
        if let Some(n) = n {
            n.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wait_then_signal_releases() {
        let d = Drain::new();
        let d2 = d.clone();
        let h = tokio::spawn(async move { d2.wait("a", None).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        d.signal("a");
        assert!(h.await.unwrap());
    }

    #[tokio::test]
    async fn signal_then_wait_returns_immediately() {
        let d = Drain::new();
        d.signal("a");
        assert!(d.wait("a", Some(Duration::from_millis(10))).await);
    }

    #[tokio::test]
    async fn timeout_returns_false() {
        let d = Drain::new();
        assert!(!d.wait("nope", Some(Duration::from_millis(20))).await);
    }
}
