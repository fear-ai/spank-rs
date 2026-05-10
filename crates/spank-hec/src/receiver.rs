//! HEC routes and the receiver state.
//!
//! Routes:
//! - `POST /services/collector/event` — JSON envelope events.
//! - `POST /services/collector/raw` — raw line-delimited body.
//! - `GET  /services/collector/health` — phase reporting.
//! - `POST /services/collector/ack` — stub (501 today; ACK is wired
//!   when the indexer lands).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use spank_core::error::SpankError;
use spank_core::lifecycle::Lifecycle;
use spank_core::{Drain, HecPhase, Rows, Sentinel};
use spank_obs::{audit_event, error_event, ingest_event};
use tokio::sync::mpsc;

use crate::authenticator::{Authenticator, HecCredential};
use crate::outcome::RequestOutcome;
use crate::processor::{decode_body, parse_event_body, parse_raw_body};
use crate::sender::Sender;

/// State carried by HEC handlers.
pub struct HecState {
    pub auth: Arc<dyn Authenticator>,
    pub queue: mpsc::Sender<QueueItem>,
    pub max_content_length: usize,
    pub phase: Arc<arc_swap::ArcSwap<HecPhase>>,
    pub drain: Drain,
}

/// What rides the indexer queue: rows or a sentinel.
#[derive(Debug)]
pub enum QueueItem {
    Rows { tag: String, rows: Rows },
    Sentinel(Sentinel),
}

/// Build HEC routes and merge them into a router.
#[must_use]
pub fn routes(state: Arc<HecState>) -> Router {
    Router::new()
        .route("/services/collector/event", post(post_event))
        .route("/services/collector/raw", post(post_raw))
        .route("/services/collector/health", get(get_health))
        .route(
            "/services/collector/ack",
            post(|| async {
                (
                    StatusCode::NOT_IMPLEMENTED,
                    Json(json!({ "text": "ack not yet implemented", "code": 14 })),
                )
            }),
        )
        .with_state(state)
}

fn extract_credential(headers: &HeaderMap) -> Option<HecCredential> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| {
            // "Splunk <token>" or "Bearer <token>"
            let mut parts = h.splitn(2, ' ');
            let scheme = parts.next()?;
            let token = parts.next()?;
            if scheme.eq_ignore_ascii_case("Splunk") || scheme.eq_ignore_ascii_case("Bearer") {
                Some(HecCredential {
                    token_value: token.to_string(),
                })
            } else {
                None
            }
        })
}

async fn post_event(
    State(s): State<Arc<HecState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    handle(&s, &headers, body, BodyKind::Event).await
}

async fn post_raw(
    State(s): State<Arc<HecState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    handle(&s, &headers, body, BodyKind::Raw).await
}

async fn get_health(State(s): State<Arc<HecState>>) -> impl IntoResponse {
    let phase = **s.phase.load();
    // DEGRADED returns 200 — the node still admits work and must stay in
    // rotation. Splunk's own degraded indexers respond 200 while reporting
    // reduced capacity. STARTED and STOPPING return 503.
    let (status, text, code) = match phase {
        HecPhase::SERVING => (StatusCode::OK, "HEC is available", 0),
        HecPhase::DEGRADED => (StatusCode::OK, "HEC is degraded", 0),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "HEC is unavailable", 9),
    };
    (
        status,
        Json(json!({
            "text": text,
            "phase": format!("{:?}", phase),
            "code": code,
        })),
    )
}

#[derive(Copy, Clone)]
enum BodyKind {
    Event,
    Raw,
}

async fn handle(
    s: &Arc<HecState>,
    headers: &HeaderMap,
    body: Bytes,
    kind: BodyKind,
) -> (StatusCode, Json<RequestOutcome>) {
    metrics::counter!(spank_obs::metrics::names::HEC_REQUESTS_TOTAL).increment(1);
    metrics::counter!(spank_obs::metrics::names::HEC_BYTES_IN_TOTAL).increment(body.len() as u64);

    // Phase admission.
    let phase = **s.phase.load();
    if !phase.admits_work() {
        let o = RequestOutcome::server_busy();
        return respond(o);
    }

    // Length cap.
    if body.len() > s.max_content_length {
        let o = RequestOutcome::invalid_data("max_content_length exceeded");
        return respond(o);
    }

    // Auth.
    let cred = match extract_credential(headers) {
        Some(c) => c,
        None => {
            let o = RequestOutcome::no_authorization();
            return respond(o);
        }
    };
    let principal = match s.auth.authenticate(&cred) {
        Ok(p) => p,
        Err(SpankError::Auth { .. }) => {
            audit_event!(decision = "deny", reason = "invalid_token");
            let o = RequestOutcome::invalid_token();
            return respond(o);
        }
        Err(e) => {
            error_event!(error = %e, recovery = "fatal_component");
            let o = RequestOutcome::invalid_token();
            return respond(o);
        }
    };
    audit_event!(decision = "allow", principal = %principal.name);

    // Decode body.
    let ce = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = match decode_body(body, ce.as_deref()) {
        Ok(b) => b,
        Err(o) => return respond(o),
    };

    // Parse.
    let rows = match kind {
        BodyKind::Event => match parse_event_body(&body) {
            Ok(r) => r,
            Err(o) => return respond(o),
        },
        BodyKind::Raw => parse_raw_body(&body, &principal.name),
    };

    if rows.is_empty() {
        let o = RequestOutcome::no_data();
        return respond(o);
    }

    // Tag — channel header if present and non-empty, else token id.
    // Empty string after trimming is treated as absent: a header with no
    // content is not a useful channel identifier and must not be used as
    // a routing key. See docs/HECst.md §3.3.
    let tag = headers
        .get("x-splunk-request-channel")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or(principal.name);

    // Submit nonblocking; queue full -> 503/code 9.
    let count = rows.len();
    if let Err(_e) = s.queue.try_send(QueueItem::Rows { tag: tag.clone(), rows }) {
        metrics::counter!(spank_obs::metrics::names::QUEUE_FULL_TOTAL).increment(1);
        let o = RequestOutcome::server_busy();
        return respond(o);
    }

    ingest_event!(
        kind = "hec.request",
        tag = %tag,
        rows = count,
        outcome_code = 0u32
    );
    let o = RequestOutcome::ok();
    respond(o)
}

fn respond(o: RequestOutcome) -> (StatusCode, Json<RequestOutcome>) {
    metrics::counter!(spank_obs::metrics::names::HEC_OUTCOME_CODE,
        "code" => o.code.to_string()
    )
    .increment(1);
    (
        StatusCode::from_u16(o.http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(o),
    )
}

/// Spawn the indexer-side consumer that drains the queue and writes
/// to a `Sender`. Returns immediately after spawning; the task runs
/// until `lifecycle` cancels.
pub fn spawn_consumer(
    mut rx: mpsc::Receiver<QueueItem>,
    sender: Arc<dyn Sender>,
    drain: Drain,
    lifecycle: Lifecycle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = lifecycle.token.cancelled() => break,
                msg = rx.recv() => {
                    let Some(msg) = msg else { break };
                    match msg {
                        QueueItem::Rows { tag, rows } => {
                            let count = rows.len();
                            metrics::gauge!(spank_obs::metrics::names::QUEUE_DEPTH_CURRENT)
                                .set(rx.len() as f64);
                            if let Err(e) = sender.submit(rows) {
                                error_event!(
                                    error = %e,
                                    recovery = ?e.recovery(),
                                    component = "hec.consumer",
                                    tag = %tag,
                                );
                            } else {
                                ingest_event!(
                                    kind = "indexer.write",
                                    tag = %tag,
                                    rows = count
                                );
                            }
                        }
                        QueueItem::Sentinel(s) => {
                            if let Err(e) = sender.flush(&s.tag) {
                                error_event!(error = %e, recovery = ?e.recovery());
                            }
                            drain.signal(&s.tag);
                            ingest_event!(kind = "sentinel.processed", tag = %s.tag);
                        }
                    }
                }
            }
        }
    })
}
