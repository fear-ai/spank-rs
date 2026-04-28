//! `spank-obs` — observability infrastructure.
//!
//! - [`init_tracing`] is the single entry point for `tracing` setup.
//! - [`metrics`] installs the Prometheus exporter and exposes counters
//!   and histograms used by every subsystem.
//! - The macros in this crate (`ingest_event!`, `lifecycle_event!`,
//!   `error_event!`) enforce a uniform field set per event category.
//!
//! All log sites in Spank should use one of these macros rather than
//! `tracing::info!` directly. The macros do not change semantics; they
//! ensure the structured fields a downstream consumer expects are
//! always present.

pub mod metrics;
pub mod tracing_init;

pub use metrics::{install_prometheus, MetricsHandle};
pub use tracing_init::{init_tracing, TracingConfig, TracingFormat};

/// Emit a structured ingest event.
///
/// Required: `kind` (string), `tag` (string — Sentinel correlation
/// when applicable, otherwise the per-source identifier).
///
/// # Example
/// ```no_run
/// # use spank_obs::ingest_event;
/// ingest_event!(kind = "hec.request", tag = "channel-x", outcome_code = 0u32);
/// ```
#[macro_export]
macro_rules! ingest_event {
    ($($field:tt)*) => {
        ::tracing::info!(target: "spank.ingest", category = "ingest", $($field)*)
    };
}

/// Emit a structured lifecycle event (start, ready, shutdown, exit).
#[macro_export]
macro_rules! lifecycle_event {
    ($($field:tt)*) => {
        ::tracing::info!(target: "spank.lifecycle", category = "lifecycle", $($field)*)
    };
}

/// Emit a structured error event with a recovery class.
///
/// Required: `recovery` (one of "retry"/"backpressure"/"fatal_component"/"fatal_process"),
/// `error` (Display'd error).
#[macro_export]
macro_rules! error_event {
    ($($field:tt)*) => {
        ::tracing::error!(target: "spank.error", category = "error", $($field)*)
    };
}

/// Emit a structured audit event (auth decisions, principal mutations).
#[macro_export]
macro_rules! audit_event {
    ($($field:tt)*) => {
        ::tracing::info!(target: "spank.audit", category = "audit", $($field)*)
    };
}
