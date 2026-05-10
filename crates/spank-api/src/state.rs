//! Shared API state.
//!
//! `ApiState` is constructed by the binary and cloned into each
//! handler via axum's `State` extractor. Hold `Arc`-friendly things
//! only. The HEC subsystem registers its own routes onto the router
//! with its own state extension, so this struct does not need to
//! know about HEC's internals.

use std::sync::Arc;

use arc_swap::ArcSwap;
use serde::Serialize;
use spank_core::HecPhase;
use spank_obs::MetricsHandle;

#[derive(Clone)]
pub struct ApiState {
    pub phase: Arc<ArcSwap<HecPhase>>,
    pub metrics: Arc<MetricsHandle>,
    pub build: BuildInfo,
    /// Deduplicated list of known index names, derived from
    /// `HecConfig::tokens[*].allowed_indexes` at startup.
    /// Used by `GET /services/data/indexes`.
    pub known_indexes: Arc<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BuildInfo {
    pub version: &'static str,
    pub bundle: &'static str,
}

impl ApiState {
    #[must_use]
    pub fn new(
        metrics: Arc<MetricsHandle>,
        bundle: &'static str,
        known_indexes: Vec<String>,
    ) -> Self {
        Self {
            phase: Arc::new(ArcSwap::from_pointee(HecPhase::STARTED)),
            metrics,
            build: BuildInfo {
                version: env!("CARGO_PKG_VERSION"),
                bundle,
            },
            known_indexes: Arc::new(known_indexes),
        }
    }

    #[must_use]
    pub fn current_phase(&self) -> HecPhase {
        **self.phase.load()
    }

    pub fn set_phase(&self, p: HecPhase) {
        self.phase.store(Arc::new(p));
    }
}
