//! Router construction.
//!
//! Stub routes return `501` with a structured body. As subsystems
//! land, they replace stubs by passing in their own router and
//! merging.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::outcome::not_implemented;
use crate::state::ApiState;

/// Build the base router. Subsystems extend this via `Router::merge`.
#[must_use]
pub fn build(state: ApiState) -> Router {
    Router::new()
        // Health and info
        .route("/health", get(health))
        .route("/services/server/info", get(server_info))
        // Metrics
        .route("/metrics/prometheus", get(metrics_prom))
        .route("/metrics", get(metrics_json))
        // Search
        .route(
            "/services/search/jobs",
            get(|| async { not_implemented("/services/search/jobs") }),
        )
        .route(
            "/services/search/jobs/:sid",
            get(|| async { not_implemented("/services/search/jobs/:sid") }),
        )
        // Indexes
        .route("/services/data/indexes", get(list_indexes))
        // Auth
        .route(
            "/services/authentication/users",
            get(|| async { not_implemented("/services/authentication/users") }),
        )
        .with_state(state)
}

async fn health(State(s): State<ApiState>) -> impl IntoResponse {
    let phase = s.current_phase();
    // DEGRADED admits work and stays in load-balancer rotation — return 200
    // with a structured body so operators can detect the degraded state without
    // the LB pulling the node. STARTED and STOPPING return 503 so the LB
    // stops routing before the node is ready or after it begins draining.
    let status = match phase {
        spank_core::HecPhase::SERVING | spank_core::HecPhase::DEGRADED => StatusCode::OK,
        spank_core::HecPhase::STARTED | spank_core::HecPhase::STOPPING => {
            StatusCode::SERVICE_UNAVAILABLE
        }
    };
    let health_status = match phase {
        spank_core::HecPhase::SERVING => "healthy",
        spank_core::HecPhase::DEGRADED => "degraded",
        _ => "unavailable",
    };
    (
        status,
        Json(json!({
            "status": health_status,
            "phase": format!("{:?}", phase),
            "admits_work": phase.admits_work(),
        })),
    )
}

async fn server_info(State(s): State<ApiState>) -> impl IntoResponse {
    Json(json!({
        "version": s.build.version,
        "bundle": s.build.bundle,
    }))
}

async fn metrics_prom(State(s): State<ApiState>) -> impl IntoResponse {
    (
        [("content-type", "text/plain; version=0.0.4")],
        s.metrics.render(),
    )
}

/// `GET /services/data/indexes`
///
/// Returns the list of configured indexes. Event counts and sizes are zero
/// until the store query layer (Phase 1) is wired; the structure matches
/// the Splunk REST API shape so SDK clients do not error on the response.
async fn list_indexes(State(s): State<ApiState>) -> impl IntoResponse {
    let entries: Vec<_> = s
        .known_indexes
        .iter()
        .map(|name| {
            json!({
                "name": name,
                "datatype": "event",
                "totalEventCount": 0,
                "currentDBSizeMB": 0,
                "maxTotalDataSizeMB": 500000,
                "isInternal": false,
            })
        })
        .collect();
    Json(json!({
        "entry": entries,
        "paging": {
            "total": entries.len(),
            "perPage": entries.len(),
            "offset": 0,
        },
        "messages": [],
    }))
}

async fn metrics_json(State(_s): State<ApiState>) -> impl IntoResponse {
    Json(json!({
        "note": "use /metrics/prometheus for full metrics",
    }))
}
