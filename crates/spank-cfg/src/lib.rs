//! `spank-cfg` — layered configuration via `figment`.
//!
//! Layer precedence (lowest to highest):
//! 1. Compiled defaults from `serde::Deserialize` impls.
//! 2. TOML file at `--config` or `$SP_CONFIG` (optional).
//! 3. Environment variables prefixed `SP_` (e.g. `SP_API__BIND` = `api.bind`).
//! 4. Programmatic overrides.
//!
//! Hot reload is not supported; SIGHUP is graceful restart per
//! `docs/Sparst.md` §5.1.

use std::path::{Path, PathBuf};

use figment::providers::{Env, Format, Toml};
use figment::Figment;
use serde::{Deserialize, Serialize};
use spank_obs::TracingConfig;

/// Top-level Spank configuration. The discriminator on bundle is
/// implicit — the bundle preset is applied by the binary before the
/// config is built.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SpankConfig {
    #[serde(default)]
    pub tracing: TracingConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub hec: HecConfig,
    #[serde(default)]
    pub tcp: TcpConfig,
    #[serde(default)]
    pub files: FilesConfig,
    #[serde(default)]
    pub shipper: ShipperConfig,
    #[serde(default)]
    pub store: StoreConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    pub bind: String,
    pub workers: Option<usize>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8089".to_string(),
            workers: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HecConfig {
    /// If `bind` is set, the HEC receiver listens on it. Otherwise
    /// HEC routes are served by the API listener.
    pub bind: Option<String>,
    pub max_content_length: usize,
    pub queue_depth: usize,
    pub output_dir: PathBuf,
    pub tokens: Vec<HecToken>,
}

impl Default for HecConfig {
    fn default() -> Self {
        Self {
            bind: None,
            max_content_length: 1024 * 1024,
            queue_depth: 1024,
            output_dir: PathBuf::from("./data/hec"),
            tokens: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HecToken {
    pub id: String,
    pub value: String,
    #[serde(default)]
    pub allowed_indexes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TcpConfig {
    pub bind: Option<String>,
    pub max_line_bytes: usize,
    pub output_dir: PathBuf,
}

impl Default for TcpConfig {
    fn default() -> Self {
        Self {
            bind: None,
            max_line_bytes: 64 * 1024,
            output_dir: PathBuf::from("./data/tcp"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FilesConfig {
    pub sources: Vec<FileSource>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileSource {
    pub path: PathBuf,
    #[serde(default)]
    pub mode: FileMode,
    #[serde(default = "default_workers")]
    pub workers: usize,
    #[serde(default = "default_channel_depth")]
    pub channel_depth: usize,
    #[serde(default)]
    pub order: ReadOrder,
}

fn default_workers() -> usize {
    1
}
fn default_channel_depth() -> usize {
    1024
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileMode {
    /// Read once and emit `Sentinel::end`.
    #[default]
    OneShot,
    /// Tail mode: follow growth, react to rotation.
    Tail,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReadOrder {
    #[default]
    Lexicographic,
    Mtime,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ShipperConfig {
    pub destinations: Vec<ShipperDest>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShipperDest {
    pub name: String,
    pub kind: ShipperKind,
    pub addr: String,
    #[serde(default = "default_backoff_ms")]
    pub backoff_initial_ms: u64,
    #[serde(default = "default_backoff_max_ms")]
    pub backoff_max_ms: u64,
}

fn default_backoff_ms() -> u64 {
    100
}
fn default_backoff_max_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShipperKind {
    Tcp,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoreConfig {
    pub backend: StoreBackend,
    pub path: PathBuf,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            backend: StoreBackend::Sqlite,
            path: PathBuf::from("./data/store"),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StoreBackend {
    Sqlite,
    /// DuckDB backend. Requires the `duckdb` feature on `spank-store`.
    DuckDb,
    /// Postgres backend. Requires the `postgres` feature on `spank-store`.
    Postgres,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Worker thread count for the tokio multi-thread runtime.
    /// `None` defaults to the number of logical cores.
    pub worker_threads: Option<usize>,
    /// Shutdown drain budget in seconds.
    pub shutdown_seconds: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            worker_threads: None,
            shutdown_seconds: 10,
        }
    }
}

/// Errors during configuration build.
#[derive(Debug, thiserror::Error)]
pub enum CfgError {
    #[error("figment error: {0}")]
    Figment(#[from] figment::Error),
    #[error("io error reading config: {0}")]
    Io(#[from] std::io::Error),
    #[error("validation error: {0}")]
    Validation(String),
}

/// Build the layered configuration.
///
/// `path` is the optional TOML file. Environment variables prefixed
/// `SP_` (with `__` as nested separator, e.g. `SP_API__BIND`) override
/// the file. Programmatic overrides apply on top.
///
/// # Errors
/// `CfgError::Figment` for parse/merge failures, `CfgError::Validation`
/// for cross-field invariants.
pub fn load(path: Option<&Path>) -> Result<SpankConfig, CfgError> {
    let mut fig = Figment::from(figment::providers::Serialized::defaults(SpankConfig::default()));
    if let Some(p) = path {
        fig = fig.merge(Toml::file(p));
    }
    fig = fig.merge(Env::prefixed("SP_").split("__"));
    let cfg: SpankConfig = fig.extract()?;
    validate(&cfg)?;
    Ok(cfg)
}

/// Render the effective config as TOML for `--show-config`.
pub fn render_toml(cfg: &SpankConfig) -> Result<String, CfgError> {
    toml::to_string_pretty(cfg).map_err(|e| CfgError::Validation(e.to_string()))
}

fn validate(cfg: &SpankConfig) -> Result<(), CfgError> {
    if cfg.hec.queue_depth == 0 {
        return Err(CfgError::Validation(
            "hec.queue_depth must be > 0 (bounded queues required)".into(),
        ));
    }
    if cfg.hec.max_content_length == 0 {
        return Err(CfgError::Validation(
            "hec.max_content_length must be > 0".into(),
        ));
    }
    for t in &cfg.hec.tokens {
        if t.value.is_empty() {
            return Err(CfgError::Validation(format!(
                "hec.tokens[id={}].value must not be empty",
                t.id
            )));
        }
    }
    if let Some(bind) = &cfg.hec.bind {
        bind.parse::<std::net::SocketAddr>().map_err(|e| {
            CfgError::Validation(format!("hec.bind is not a valid socket address: {e}"))
        })?;
    }
    cfg.api
        .bind
        .parse::<std::net::SocketAddr>()
        .map_err(|e| CfgError::Validation(format!("api.bind is not a valid socket address: {e}")))?;
    if let Some(bind) = &cfg.tcp.bind {
        bind.parse::<std::net::SocketAddr>().map_err(|e| {
            CfgError::Validation(format!("tcp.bind is not a valid socket address: {e}"))
        })?;
    }
    for dest in &cfg.shipper.destinations {
        dest.addr.parse::<std::net::SocketAddr>().map_err(|e| {
            CfgError::Validation(format!(
                "shipper.destinations[name={}].addr is not a valid socket address: {e}",
                dest.name
            ))
        })?;
    }
    if cfg.runtime.shutdown_seconds == 0 {
        return Err(CfgError::Validation(
            "runtime.shutdown_seconds must be > 0".into(),
        ));
    }
    if let Some(n) = cfg.runtime.worker_threads {
        if n == 0 {
            return Err(CfgError::Validation(
                "runtime.worker_threads must be > 0 if set".into(),
            ));
        }
    }
    for (i, src) in cfg.files.sources.iter().enumerate() {
        if src.workers == 0 {
            return Err(CfgError::Validation(format!(
                "files.sources[{i}].workers must be >= 1"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn defaults_and_validation() {
        // Defaults load cleanly.
        let cfg = load(None).unwrap();
        assert_eq!(cfg.api.bind, "127.0.0.1:8089");
        assert_eq!(cfg.hec.queue_depth, 1024);

        // Zero queue_depth rejected.
        std::env::set_var("SP_HEC__QUEUE_DEPTH", "0");
        let res = load(None);
        std::env::remove_var("SP_HEC__QUEUE_DEPTH");
        assert!(matches!(res, Err(CfgError::Validation(_))), "queue_depth=0 should fail");

        // Zero max_content_length rejected.
        std::env::set_var("SP_HEC__MAX_CONTENT_LENGTH", "0");
        let res = load(None);
        std::env::remove_var("SP_HEC__MAX_CONTENT_LENGTH");
        assert!(matches!(res, Err(CfgError::Validation(_))), "max_content_length=0 should fail");

        // Zero shutdown_seconds rejected.
        std::env::set_var("SP_RUNTIME__SHUTDOWN_SECONDS", "0");
        let res = load(None);
        std::env::remove_var("SP_RUNTIME__SHUTDOWN_SECONDS");
        assert!(matches!(res, Err(CfgError::Validation(_))), "shutdown_seconds=0 should fail");

        // Zero worker_threads rejected.
        std::env::set_var("SP_RUNTIME__WORKER_THREADS", "0");
        let res = load(None);
        std::env::remove_var("SP_RUNTIME__WORKER_THREADS");
        assert!(matches!(res, Err(CfgError::Validation(_))), "worker_threads=0 should fail");

        // Invalid api.bind rejected.
        std::env::set_var("SP_API__BIND", "not-an-address");
        let res = load(None);
        std::env::remove_var("SP_API__BIND");
        assert!(matches!(res, Err(CfgError::Validation(_))), "bad api.bind should fail");
    }
}
