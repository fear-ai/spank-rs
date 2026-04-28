//! `TcpSender` — connect to a peer, ship line-delimited JSON, reconnect
//! on failure with exponential backoff and jitter.

use std::time::Duration;

use spank_core::error::{Result, SpankError};
use spank_core::lifecycle::Lifecycle;
use spank_obs::{error_event, ingest_event, lifecycle_event};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct TcpSender {
    pub addr: String,
    pub backoff_initial_ms: u64,
    pub backoff_max_ms: u64,
}

impl TcpSender {
    #[must_use]
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            backoff_initial_ms: 100,
            backoff_max_ms: 30_000,
        }
    }

    /// Run the shipper. Reads lines from `rx` and writes them to the
    /// peer; reconnects on disconnect; exits when `lifecycle` cancels
    /// or `rx` closes.
    pub async fn run(self, mut rx: mpsc::Receiver<String>, lifecycle: Lifecycle) -> Result<()> {
        let mut backoff_ms = self.backoff_initial_ms;
        loop {
            if lifecycle.token.is_cancelled() {
                break;
            }
            let stream = match TcpStream::connect(&self.addr).await {
                Ok(s) => {
                    backoff_ms = self.backoff_initial_ms;
                    lifecycle_event!(
                        component = "tcp_sender",
                        kind = "connected",
                        addr = %self.addr
                    );
                    s
                }
                Err(e) => {
                    let err = SpankError::io("connect", self.addr.clone(), e);
                    error_event!(
                        error = %err,
                        recovery = "retry",
                        component = "tcp_sender",
                        addr = %self.addr,
                        backoff_ms = backoff_ms
                    );
                    tokio::select! {
                        _ = lifecycle.token.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_millis(backoff_ms)) => {}
                    }
                    backoff_ms = (backoff_ms * 2).min(self.backoff_max_ms);
                    continue;
                }
            };

            let (_, mut wr) = stream.into_split();
            loop {
                tokio::select! {
                    _ = lifecycle.token.cancelled() => return Ok(()),
                    msg = rx.recv() => {
                        let Some(line) = msg else { return Ok(()) };
                        let mut payload = line.into_bytes();
                        payload.push(b'\n');
                        if let Err(e) = wr.write_all(&payload).await {
                            let err = SpankError::io("write", self.addr.clone(), e);
                            error_event!(
                                error = %err,
                                recovery = "retry",
                                component = "tcp_sender"
                            );
                            break;
                        }
                        metrics::counter!(spank_obs::metrics::names::TCP_BYTES_OUT_TOTAL)
                            .increment(payload.len() as u64);
                        ingest_event!(
                            kind = "tcp.shipped",
                            addr = %self.addr,
                            bytes = payload.len()
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
