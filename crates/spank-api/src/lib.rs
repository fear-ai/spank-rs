//! `spank-api` — the HTTP API surface.
//!
//! Hosts the multi-thread tokio runtime, the axum router, and the
//! Splunk-aligned route surface. Routes that are not yet implemented
//! return `501 Not Implemented` with a structured outcome body so
//! shippers and clients see a real shape today.

pub mod outcome;
pub mod router;
pub mod server;
pub mod state;

pub use server::serve;
pub use state::ApiState;
