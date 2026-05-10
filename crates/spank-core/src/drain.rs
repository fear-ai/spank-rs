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
    /// Tag → notifier. Permanent once created; shared across all waiters
    /// for the same tag and released by `notify_waiters`.
    notifiers: HashMap<String, Arc<Notify>>,
    /// Latched set of tags that have already fired. A waiter that arrives
    /// after the signal still completes immediately.
    signaled: HashMap<String, ()>,
}

/// Wait-side handle for source completion.
///
/// Cloning shares state. One `Drain` per indexer; producers and
/// waiters use the same instance.
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
    ///
    /// # Race-free protocol
    ///
    /// Uses `Notified::enable()` to subscribe to the notifier before
    /// checking the latch. `enable()` places the future in the notifier's
    /// wait set immediately — no poll required — so any `notify_waiters()`
    /// call that occurs after `enable()` will wake this future regardless
    /// of when it is first polled. The latch check after `enable()` catches
    /// signals that arrived before `enable()` ran; such signals wrote to
    /// `signaled` before calling `notify_waiters()`, so the check always
    /// observes them.
    pub async fn wait(&self, tag: &str, timeout: Option<Duration>) -> bool {
        // Fast-path: already signaled before we even start.
        {
            let inner = self.inner.lock();
            if inner.signaled.contains_key(tag) {
                return true;
            }
        }

        let n = self.notifier(tag);
        let mut notified = std::pin::pin!(n.notified());
        // Subscribe to the notifier before checking the latch. Any
        // notify_waiters() call after this point will reach this future.
        notified.as_mut().enable();

        // Check whether signal() ran between notifier() and enable().
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
