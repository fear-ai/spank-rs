//! Per-connection receiver.
//!
//! Reads newline-delimited frames with a hard line cap. Every error
//! path is attributed to a syscall and a peer.

use std::net::SocketAddr;

use bytes::BytesMut;
use spank_core::error::{Result, SpankError};
use spank_core::lifecycle::Lifecycle;
use spank_obs::{error_event, ingest_event, lifecycle_event};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct ConnHandle {
    pub peer: SocketAddr,
    pub conn_id: u64,
}

/// Event surfaced by a per-connection task.
#[derive(Debug)]
pub enum ConnEvent {
    /// Connection accepted.
    Opened { handle: ConnHandle },
    /// One framed line received.
    Line { handle: ConnHandle, line: String },
    /// Connection closed; final.
    Closed { handle: ConnHandle, reason: String },
}

static CONN_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub async fn run_connection(
    mut stream: TcpStream,
    peer: SocketAddr,
    max_line_bytes: usize,
    out: &mpsc::Sender<ConnEvent>,
    lifecycle: Lifecycle,
) -> Result<()> {
    let conn_id = CONN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let handle = ConnHandle { peer, conn_id };
    lifecycle_event!(
        component = "tcp_conn",
        kind = "open",
        peer = %peer,
        conn_id = conn_id
    );
    // Opened event is best-effort; consumer closure is not a reason to abort.
    let _ = out.try_send(ConnEvent::Opened { handle: handle.clone() });

    if let Err(e) = stream.set_nodelay(true) {
        let err = SpankError::io("set_nodelay", peer.to_string(), e);
        metrics::counter!(
            spank_obs::metrics::names::TCP_SYSCALL_ERRORS_TOTAL,
            "syscall" => "set_nodelay"
        )
        .increment(1);
        error_event!(error = %err, recovery = ?err.recovery(), peer = %peer);
        // non-fatal: continue
    }

    let mut buf = BytesMut::with_capacity(8 * 1024);
    let mut reason = String::from("eof");
    loop {
        tokio::select! {
            _ = lifecycle.token.cancelled() => {
                reason = "shutdown".into();
                break;
            }
            res = stream.read_buf(&mut buf) => {
                match res {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        metrics::counter!(spank_obs::metrics::names::TCP_BYTES_IN_TOTAL)
                            .increment(n as u64);
                        // Emit each newline-terminated frame. Discard
                        // partial trailing bytes until next read.
                        loop {
                            let Some(pos) = buf.iter().position(|b| *b == b'\n') else { break };
                            if pos > max_line_bytes {
                                error_event!(
                                    error = "line exceeds max_line_bytes",
                                    recovery = "drop",
                                    peer = %peer,
                                    bytes = pos
                                );
                                buf.advance_to(pos + 1);
                                continue;
                            }
                            let line_bytes = buf.split_to(pos + 1);
                            let mut line = String::from_utf8_lossy(&line_bytes[..pos]).into_owned();
                            // Strip trailing \r if CRLF.
                            if line.ends_with('\r') {
                                line.pop();
                            }
                            ingest_event!(
                                kind = "tcp.line",
                                peer = %peer,
                                conn_id = conn_id,
                                bytes = line.len()
                            );
                            match out.try_send(ConnEvent::Line {
                                handle: handle.clone(),
                                line,
                            }) {
                                Ok(()) => {}
                                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                    metrics::counter!(
                                        spank_obs::metrics::names::TCP_LINES_DROPPED_TOTAL,
                                        "peer" => peer.to_string()
                                    ).increment(1);
                                    error_event!(
                                        error = "consumer channel full; line dropped",
                                        recovery = "backpressure",
                                        peer = %peer,
                                        conn_id = conn_id
                                    );
                                }
                                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                    reason = "consumer_closed".into();
                                    let _ = stream.shutdown().await;
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let err = SpankError::io("read", peer.to_string(), e);
                        metrics::counter!(
                            spank_obs::metrics::names::TCP_SYSCALL_ERRORS_TOTAL,
                            "syscall" => "read"
                        ).increment(1);
                        error_event!(error = %err, recovery = ?err.recovery(), peer = %peer);
                        reason = format!("{}", err);
                        break;
                    }
                }
            }
        }
    }

    lifecycle_event!(
        component = "tcp_conn",
        kind = "close",
        peer = %peer,
        conn_id = conn_id,
        reason = %reason
    );
    let _ = out.try_send(ConnEvent::Closed { handle, reason });
    Ok(())
}

/// Trait for `BytesMut::advance_to` consistency on older versions.
trait BytesMutExt {
    fn advance_to(&mut self, n: usize);
}
impl BytesMutExt for BytesMut {
    fn advance_to(&mut self, n: usize) {
        let _ = self.split_to(n);
    }
}

use tokio::io::AsyncWriteExt;
