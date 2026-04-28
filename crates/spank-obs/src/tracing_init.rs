//! `tracing` initialization.
//!
//! One entry point — `init_tracing` — chosen so that the binary can
//! never half-initialize. The function is idempotent: subsequent calls
//! return `Ok(())` without changing the subscriber.

use std::path::PathBuf;

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TracingFormat {
    /// Human-readable colored output for development.
    Pretty,
    /// JSON-per-line for log shippers and machine consumption.
    Json,
}

impl Default for TracingFormat {
    fn default() -> Self {
        Self::Pretty
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TracingConfig {
    #[serde(default)]
    pub format: TracingFormat,
    /// `tracing_subscriber::EnvFilter` directive. `RUST_LOG` overrides
    /// this when set.
    #[serde(default = "default_filter")]
    pub filter: String,
    /// If set, also write logs to this file (JSON regardless of
    /// `format`). Rotation is daily.
    #[serde(default)]
    pub file: Option<PathBuf>,
}

fn default_filter() -> String {
    "info,spank=info".to_string()
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            format: TracingFormat::default(),
            filter: default_filter(),
            file: None,
        }
    }
}

static INIT: OnceCell<()> = OnceCell::new();
static FILE_GUARD: OnceCell<tracing_appender::non_blocking::WorkerGuard> = OnceCell::new();

/// Initialize global `tracing`. Idempotent.
///
/// # Errors
/// Returns `Err` if a subscriber is already installed by another path
/// (e.g. tests setting one up first); this is logged and ignored.
pub fn init_tracing(cfg: &TracingConfig) -> std::io::Result<()> {
    if INIT.get().is_some() {
        return Ok(());
    }

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cfg.filter.clone()));

    let stdout_layer = match cfg.format {
        TracingFormat::Pretty => fmt::layer().with_target(true).boxed(),
        TracingFormat::Json => fmt::layer().json().with_target(true).boxed(),
    };

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer);

    if let Some(path) = &cfg.file {
        let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(parent)?;
        let file_appender = tracing_appender::rolling::daily(parent, path.file_name().unwrap());
        let (nb, guard) = tracing_appender::non_blocking(file_appender);
        let _ = FILE_GUARD.set(guard);
        let file_layer = fmt::layer().json().with_writer(nb);
        let _ = subscriber.with(file_layer).try_init();
    } else {
        let _ = subscriber.try_init();
    }

    let _ = INIT.set(());
    Ok(())
}

use tracing_subscriber::Layer;
