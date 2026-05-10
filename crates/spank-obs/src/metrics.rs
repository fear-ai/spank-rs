//! Metrics installation and well-known names.
//!
//! All counters and histograms used across Spank are documented here.
//! Keep this list aligned with `docs/Observability.md`.
//!
//! Naming convention: `spank.<subsystem>.<noun>_<unit>`.
//! - Counters end in a noun: `requests_total`, `bytes_in_total`.
//! - Histograms end in a unit: `request_duration_seconds`.
//! - Gauges end in `_current`: `queue_depth_current`.

use std::net::SocketAddr;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

#[derive(Clone)]
pub struct MetricsHandle {
    pub prometheus: PrometheusHandle,
}

impl std::fmt::Debug for MetricsHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsHandle").finish_non_exhaustive()
    }
}

impl MetricsHandle {
    /// Render the Prometheus exposition for the `/metrics/prometheus`
    /// endpoint.
    #[must_use]
    pub fn render(&self) -> String {
        self.prometheus.render()
    }
}

/// Install the Prometheus exporter and return a render handle. Does
/// not start an HTTP listener; the API server scrapes via `render`.
///
/// # Errors
/// Returns the underlying installer error if the recorder is already
/// installed by another path.
pub fn install_prometheus() -> Result<MetricsHandle, String> {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .map_err(|e| e.to_string())?;
    Ok(MetricsHandle { prometheus: handle })
}

/// Install Prometheus and bind a dedicated metrics HTTP listener.
///
/// Useful when the metrics endpoint should run on a separate port
/// from the API server. Call once.
///
/// # Errors
/// Returns the installer error.
pub fn install_prometheus_with_listener(addr: SocketAddr) -> Result<MetricsHandle, String> {
    let handle = PrometheusBuilder::new()
        .with_http_listener(addr)
        .install_recorder()
        .map_err(|e| e.to_string())?;
    Ok(MetricsHandle { prometheus: handle })
}

/// Well-known metric names. Use these constants rather than literal
/// strings at call sites.
pub mod names {
    pub const HEC_REQUESTS_TOTAL: &str = "spank.hec.requests_total";
    pub const HEC_BYTES_IN_TOTAL: &str = "spank.hec.bytes_in_total";
    pub const HEC_OUTCOME_CODE: &str = "spank.hec.outcome_code_total";

    pub const QUEUE_DEPTH_CURRENT: &str = "spank.queue.depth_current";
    pub const QUEUE_FULL_TOTAL: &str = "spank.queue.full_total";

    pub const FILE_BYTES_READ_TOTAL: &str = "spank.file.bytes_read_total";
    pub const FILE_LINES_READ_TOTAL: &str = "spank.file.lines_read_total";

    pub const TCP_BYTES_IN_TOTAL: &str = "spank.tcp.bytes_in_total";
    pub const TCP_BYTES_OUT_TOTAL: &str = "spank.tcp.bytes_out_total";
    pub const TCP_CONNECTIONS_CURRENT: &str = "spank.tcp.connections_current";
    pub const TCP_SYSCALL_ERRORS_TOTAL: &str = "spank.tcp.syscall_errors_total";
    pub const TCP_LINES_DROPPED_TOTAL: &str = "spank.tcp.lines_dropped_total";

    pub const STORE_INSERTS_TOTAL: &str = "spank.store.inserts_total";
    pub const STORE_INSERT_DURATION: &str = "spank.store.insert_duration_seconds";

    pub const PANICS_TOTAL: &str = "spank.process.panics_total";
}
