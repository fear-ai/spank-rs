//! Wire-format response helpers shared by the API and HEC routes.
//!
//! The shape mirrors Splunk HEC: `{ "text": ..., "code": ... }` for
//! HEC, and a small JSON envelope for non-HEC routes.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct StubOutcome {
    pub text: &'static str,
    pub route: &'static str,
}

pub fn not_implemented(route: &'static str) -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(StubOutcome {
            text: "not implemented",
            route,
        }),
    )
}
