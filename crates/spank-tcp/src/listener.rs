//! TCP listener.
//!
//! Accepts connections, hands each to a per-connection task. On
//! `accept` errors the listener backs off briefly so a transient
//! `EMFILE` does not become a tight loop.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use spank_core::error::{Result, SpankError};
use spank_core::lifecycle::Lifecycle;
use spank_obs::{error_event, lifecycle_event};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::receiver::{run_connection, ConnEvent};

/// Serve TCP on `addr`, sending events to `out`.
///
/// Each accepted connection is given a derived `Lifecycle` so that
/// shutting down the parent cancels all in-flight reads.
///
/// # Errors
/// `SpankError::Io { syscall: "bind", .. }` if bind fails. Accept
/// failures are logged and retried with backoff.
pub async fn serve(
    addr: SocketAddr,
    max_line_bytes: usize,
    out: mpsc::Sender<ConnEvent>,
    lifecycle: Lifecycle,
) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| SpankError::io("bind", addr.to_string(), e))?;
    lifecycle_event!(
        component = "tcp_listener",
        kind = "ready",
        bind = %addr
    );

    let out = Arc::new(out);
    let mut backoff_ms: u64 = 10;
    loop {
        tokio::select! {
            _ = lifecycle.token.cancelled() => break,
            accept = listener.accept() => {
                match accept {
                    Ok((stream, peer)) => {
                        backoff_ms = 10;
                        metrics::gauge!(spank_obs::metrics::names::TCP_CONNECTIONS_CURRENT).increment(1.0);
                        let out = out.clone();
                        let conn_lc = lifecycle.child("tcp_conn");
                        tokio::spawn(async move {
                            let _ = run_connection(stream, peer, max_line_bytes, &out, conn_lc).await;
                            metrics::gauge!(spank_obs::metrics::names::TCP_CONNECTIONS_CURRENT).decrement(1.0);
                        });
                    }
                    Err(e) => {
                        let err = SpankError::io("accept", addr.to_string(), e);
                        metrics::counter!(
                            spank_obs::metrics::names::TCP_SYSCALL_ERRORS_TOTAL,
                            "syscall" => "accept"
                        ).increment(1);
                        error_event!(
                            error = %err,
                            recovery = ?err.recovery(),
                            component = "tcp_listener",
                            backoff_ms = backoff_ms
                        );
                        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                        backoff_ms = (backoff_ms * 2).min(1000);
                    }
                }
            }
        }
    }
    lifecycle_event!(component = "tcp_listener", kind = "stopped");
    Ok(())
}
