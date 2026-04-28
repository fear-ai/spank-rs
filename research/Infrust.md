# Infrust — Rust Implementation Infrastructure

`Focus: research` — the Rust infrastructure counterpart to `spank-py/Infra.md`: logging, observability, configuration, process lifecycle, runtime tuning, packaging, CLI shape, health endpoints, build tooling, and test infrastructure. The audience is a developer working on operational or cross-cutting concerns in the Rust port. Does not receive work items (`Plan.md`) or subsystem contracts (`docs/`).

## Scope

Infrust is the Rust counterpart to `Infra.md`. It covers the cross-cutting runtime infrastructure of a hypothetical Rust implementation of Spank: logging and observability, configuration, process lifecycle and signals, runtime tuning, packaging and distribution, CLI shape, health and metrics endpoints, build-time tooling, and test-harness infrastructure. Each section names what the Rust ecosystem has converged on, what the alternatives are, and how the choice contrasts with `Infra.md`'s Python decisions.

The reference projects and their source crates are inventoried in `Pyst.md` Appendix A. Vector at `/Users/walter/Work/Spank/sOSS/vector` is the closest in-segment Rust prior art and is cited concretely.

## Table of Contents

1. Logging Stack
2. Metrics
3. Distributed Tracing
4. Error Taxonomy
5. Configuration
6. Process Lifecycle and Signals
7. Async Runtime Configuration
8. Packaging and Distribution
9. CLI Conventions
10. Health, Readiness, and Observability Endpoints
11. Process and Threading Model
12. Test Harness Infrastructure
13. Build-Time Tooling
14. Side-by-Side with Infra.md

---

## 1. Logging Stack

The Rust ecosystem has two facade libraries (`log` and `tracing`) and a long tail of subscribers. New code uses `tracing`; `log` survives mostly because old dependencies still emit through it, and `tracing` provides a `tracing-log` adapter that captures `log` records.

### 1.1 tracing as the prevailing choice

`tracing` (`tokio-rs/tracing`) emits structured events with key-value fields and supports spans (parent-child contextual scopes that survive across `.await` points). Events carry a level (`TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR`), a target (typically the module path), and an arbitrary set of typed fields:

```rust
tracing::info!(channel = %ch, ack_id = id, "ack issued");
tracing::warn!(error = %e, "failed to parse event");
```

Spans add context that propagates implicitly:

```rust
let span = tracing::info_span!("handle_request", request_id = %req_id);
let _enter = span.enter();
// any tracing::* call inside this scope inherits request_id as a field
```

Every notable Rust server project today uses tracing — Vector, Quickwit, Parseable, axum (via `tower-http::trace`), tonic. The `slog` crate is the older alternative; small holdouts remain (`influxdb_iox` migrated away). New code should not adopt slog.

`defmt` is the embedded analog (no_std, deferred formatting); not relevant here.

### 1.2 tracing-subscriber

A subscriber consumes events and routes them to outputs. `tracing-subscriber` is the canonical implementation; it composes layers:

- `fmt::Layer` — formats events to text or JSON.
- `EnvFilter` — `RUST_LOG`-style filter (`info,spank_storage=debug`).
- `tracing_subscriber::registry()` — the layer host.

A typical setup:

```rust
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

tracing_subscriber::registry()
    .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
    .with(fmt::layer().json().with_current_span(true).with_span_list(false))
    .init();
```

For ELK/Datadog ingest, `tracing-bunyan-formatter` produces a Bunyan-compatible JSON shape. For OpenTelemetry, `tracing-opentelemetry` adds a layer that emits spans as OTel spans (see §3).

### 1.3 JSON output schema

A JSON line per event with a stable schema:

```json
{
  "timestamp": "2026-04-24T18:12:33.142Z",
  "level": "INFO",
  "target": "spank_hec::receiver",
  "message": "ack issued",
  "channel": "abc-123",
  "ack_id": 42,
  "span": {"name": "handle_request", "request_id": "..."}
}
```

`fmt::Layer::json()` emits roughly this shape. Vector's `src/internal_telemetry/log.rs` configures `tracing_subscriber::fmt` with a JSON formatter and writes to stderr — the same shape.

### 1.4 Performance

`tracing::trace!` calls have compile-time level filtering via the `tracing/release_max_level_info` cargo feature: the macro expands to nothing for filtered levels. Production binaries set this feature; debug builds leave it unset for full verbosity at runtime.

Every `tracing::info!` call with a disabled subscriber costs roughly 5–10 ns; with a JSON formatter at INFO level, roughly 1–5 μs depending on field count and stderr contention. For an ingest path running at 100k events/sec, a per-event log call is unaffordable; per-batch logging at warn-and-above is the right discipline.

### 1.5 Sampling and rate limiting

`tracing-subscriber` does not natively rate-limit. Three approaches:

- **Compile-time level filter** (`release_max_level_info`).
- **Custom `Layer`** that drops events above a threshold per second per target.
- **`tracing-error`** captures error context at the error site without per-call logging, then logs once at the boundary.

### 1.6 Stderr versus files

12-factor style is to write to stderr only and let the supervisor (systemd, container runtime) capture and route. journald sees structured fields if `tracing-journald` is the subscriber instead of `fmt::Layer`. File output is available (`tracing-appender` for non-blocking file writes with rotation) but adds a moving part — log file truncation is the operator's `logrotate` problem otherwise.

Vector logs to stderr by default; enterprise deployments redirect via systemd `StandardOutput=journal`.

### 1.7 Mandate

`tracing` plus `tracing-subscriber` with `EnvFilter` and `fmt::layer().json()` to stderr. Compile-time `release_max_level_info`. No file sink in the binary; the OS supervisor handles persistence.

### 1.8 Alternatives

- **`log` + `env_logger`.** Simplest possible. Emits unstructured text. Adequate for a CLI tool; insufficient for a server that needs span context and structured fields.
- **`slog`.** Predates tracing's structured fields. Async-emit mode reduces hot-path cost. Smaller ecosystem now; not chosen for new work.
- **`tracing-bunyan-formatter`.** Preferred when the downstream is Bunyan-aware (Splunk, ELK with a Bunyan parser). Drop-in replacement for `fmt::layer().json()`.
- **`tracing-journald`.** Native journald protocol — preserves structured fields without re-parsing JSON. Linux-only. Use under systemd unconditionally.
- **`defmt`.** Embedded only.

## 2. Metrics

Rust has two metrics ecosystems: the `metrics` crate facade family and the older `prometheus` crate registry style.

### 2.1 metrics crate

`metrics` (`metrics-rs/metrics`) is a facade similar to `log`: emit metrics via macros (`counter!`, `gauge!`, `histogram!`), pluggable recorder backend.

```rust
metrics::counter!("hec.events_received", "channel" => channel.to_string()).increment(1);
metrics::histogram!("hec.process_ms").record(elapsed_ms);
```

`metrics-exporter-prometheus` exposes `/metrics` in Prometheus text format. `metrics-exporter-tcp`, `metrics-exporter-statsd`, etc., for other sinks.

### 2.2 prometheus crate

`prometheus` (`tikv/rust-prometheus`) is the older direct API: register metrics in a `Registry`, increment via the metric handles. Verbose; predates the `metrics` facade. Still used by tikv, by some internal tooling.

### 2.3 OpenTelemetry metrics

`opentelemetry` SDK exposes metrics via OTLP. `opentelemetry-prometheus` bridges to Prometheus pull. Useful when traces and metrics are unified through OTel; overkill if Prometheus pull alone suffices.

### 2.4 Mandate

`metrics` facade with `metrics-exporter-prometheus` exposing `/metrics`. Cardinality discipline: labels are bounded and from a known set; no per-request unique labels.

### 2.5 Alternatives

- **`prometheus` crate direct.** Simpler when there are <10 metrics; awkward beyond.
- **OpenTelemetry only.** Right when the OTel collector sits in front of every signal; heavier setup.
- **No metrics, just logs.** Acceptable for a CLI; insufficient for a server.

## 3. Distributed Tracing

`tracing-opentelemetry` adds an OTel exporter layer to a `tracing-subscriber` registry: every tracing span becomes an OTel span. Configuration:

```rust
let tracer = opentelemetry_otlp::new_pipeline()
    .tracing()
    .with_exporter(opentelemetry_otlp::new_exporter().tonic())
    .install_batch(opentelemetry_sdk::runtime::Tokio)?;

tracing_subscriber::registry()
    .with(tracing_opentelemetry::layer().with_tracer(tracer))
    .with(fmt::layer().json())
    .with(EnvFilter::from_default_env())
    .init();
```

W3C `traceparent` propagation: `tower-http::trace::TraceLayer` injects/extracts on incoming and outgoing HTTP. `tracing-actix-web` is the actix equivalent.

Sampling: `opentelemetry_sdk::trace::Sampler::TraceIdRatioBased(0.01)` for 1% sampling. Per-span sampling decisions are honored by exporters.

### 3.1 Mandate

`tracing-opentelemetry` layer optional, gated by config (`telemetry.otlp.endpoint`). When enabled, `tower-http::TraceLayer` extracts and propagates W3C traceparent.

### 3.2 Alternatives

- **No distributed tracing.** Adequate for laptop and small VPS. Telemetry off by default.
- **Jaeger/Zipkin direct.** Predates OTel. Not chosen for new work.

## 4. Error Taxonomy

Stast §4 prescribes the error-handling rules; Infrust covers the runtime infrastructure: where errors flow, how panics are handled, how backtraces are captured.

### 4.1 Boundaries

- Library crates return `Result<T, ThisCrateError>` where `ThisCrateError` is a `thiserror`-derived enum.
- The application (binary crate) returns `anyhow::Result<()>` from `main`. Internal application functions return `anyhow::Result<T>`.
- HTTP handlers return `Result<Response, AppError>` where `AppError` implements `IntoResponse` and maps internal classes to HTTP status codes plus a fixed body schema. `?` propagates from any `From`-implementing source error.

### 4.2 Backtraces

`std::backtrace::Backtrace` captures the call stack on demand. `RUST_BACKTRACE=1` enables capture; `RUST_BACKTRACE=full` includes std-internal frames. `anyhow::Error` captures a backtrace when `RUST_BACKTRACE` is set; `eyre::Report` does similarly. `thiserror` does not capture by default; opt in by adding `backtrace: std::backtrace::Backtrace` as a field with `#[from]` only on the wrapped error.

For production binaries, set `RUST_BACKTRACE=1` in the systemd unit. The cost of capture is paid only when an error is constructed.

### 4.3 Panics

`std::panic::set_hook` installs a global hook. The first action of `main` is to install a panic hook that logs the panic at ERROR level via `tracing` and includes the location and any backtrace:

```rust
std::panic::set_hook(Box::new(|info| {
    let backtrace = std::backtrace::Backtrace::force_capture();
    tracing::error!(
        location = ?info.location(),
        payload = %panic_message(info),
        backtrace = %backtrace,
        "panic"
    );
}));
```

`[profile.release] panic = "abort"` is recommended: smaller binaries, no unwinding cost, immediate process termination on panic. The systemd unit's `Restart=on-failure` brings the process back. The alternative (`panic = "unwind"`) lets a panic walk the stack and possibly be caught — most servers do not need this and pay the binary-size cost regardless.

### 4.4 Mandate

`thiserror` per library crate, `anyhow` in the binary. `set_hook` logs panics through `tracing`. `panic = "abort"` in release. `RUST_BACKTRACE=1` in production unit files.

### 4.5 Alternatives

- **`eyre` over `anyhow`.** API-compatible; supports custom report handlers (`color-eyre` for terminal output, `miette` for diagnostics). Pick one project-wide.
- **`panic = "unwind"`.** Larger binary, allows `std::panic::catch_unwind` at thread boundaries (common in test runners and FFI). Not necessary for a server with `Restart=on-failure`.

## 5. Configuration

The Rust ecosystem has multiple configuration layers; the choice depends on the desired layering complexity.

### 5.1 Core stack

For an application of Spank's scope:

- **`clap`** for CLI parsing. Derive API: `#[derive(Parser)]` on a struct, `clap` generates the parser, `--help`, `--version`, shell completions (via `clap_complete`).
- **`serde` + `toml`** for the config file. `#[derive(Deserialize)]` on the config struct; `toml::from_str` parses.
- **`figment`** or **`config`** for layered providers (defaults → file → env → argv).
- **`validator`** crate for declarative field validation, or hand-written `try_from`/`Deserialize` impls for cross-field invariants.

### 5.2 figment

`figment` (used by Rocket) is the more flexible composer:

```rust
use figment::{Figment, providers::{Format, Toml, Env, Serialized}};

let cfg: Config = Figment::new()
    .merge(Serialized::defaults(Config::default()))
    .merge(Toml::file(&args.config_path).nested())
    .merge(Env::prefixed("SPANK_").split("__"))
    .extract()?;
```

Precedence: defaults → TOML file → environment. CLI overrides applied separately by merging another `Serialized` provider built from the parsed clap struct. Validation runs in a `try_from` or post-extract step.

### 5.3 config crate

`config` is older but well-maintained. Similar shape; less ergonomic.

### 5.4 Frozen vs hot-reload

Two policies:

- **Frozen**: parse once, wrap in `Arc<Config>`, share immutably. SIGHUP triggers a graceful restart. Matches `Infra.md` CF-R11.
- **Hot reload**: parse, validate, swap into an `arc_swap::ArcSwap<Config>` on SIGHUP. Readers do `let cfg = handle.current();` (returns `Arc<Config>`, lock-free, never blocks). Failed validation logs and leaves the running config intact.

The choice depends on operational needs. Spank's current Python policy is frozen plus restart; Rust does not change that calculus. Hot reload is feasible if specific fields warrant it (e.g. token list) without making the whole config reloadable.

### 5.5 Validation

`validator` derives field-level validators (`#[validate(range(min = 1, max = 65535))]` on a port field). Cross-field validation goes in a `Config::validate(&self) -> Result<(), ConfigError>` method called after deserialization, returning all errors at once. Same shape as `Infra.md` CF-R5.

### 5.6 Vector's actual config

Vector parses TOML/YAML/JSON via `serde` and `config-rs`-style providers. Validation is per-component: each source/transform/sink defines a `SourceConfig` struct with `serde` derives and a `build` method that constructs the runtime component or returns `Result<_, BuildError>`. The pattern fits Spank's subsystem-config-dataclass approach exactly — `[hec]`, `[indexer]`, `[search]` each map to a typed struct.

### 5.7 Mandate

`clap` derive for CLI; `serde + toml` for files; `figment` for layered composition; `validator` for declarative field rules plus a `validate` method for cross-field. Frozen by default. SIGHUP triggers graceful restart; hot reload only for specifically-flagged fields via `ArcSwap`.

### 5.8 Alternatives

- **`envy` only.** Env-vars only, no file. Right for 12-factor containers that get config from the orchestrator. Limiting for a deployable that operators edit by hand.
- **`dotenvy`.** Loads `.env` into the process environment at startup. Convenient for development; not a substitute for proper config.
- **No layering, single TOML.** Simplest. Loses the 12-factor env-override and the CLI-flag override. Acceptable only for small tools.
- **`config` crate over `figment`.** Older, less ergonomic, equivalent feature set. Still maintained.

## 6. Process Lifecycle and Signals

A long-running server has a startup phase, a steady-state phase, and a shutdown phase. The Rust patterns are well-grooved by axum, hyper, and Vector.

### 6.1 Startup

- Parse config (§5).
- Initialize `tracing` subscriber (§1).
- Install panic hook (§4.3).
- Build subsystems (each returns `Result<Subsystem, BuildError>`).
- Wire signal handlers.
- Enter the runtime.

### 6.2 Signals

`tokio::signal::unix` delivers SIGTERM, SIGINT, SIGHUP, etc. as async streams:

```rust
use tokio::signal::unix::{signal, SignalKind};

let mut term = signal(SignalKind::terminate())?;
let mut int  = signal(SignalKind::interrupt())?;
let mut hup  = signal(SignalKind::hangup())?;

loop {
    tokio::select! {
        _ = term.recv() => break "SIGTERM",
        _ = int.recv()  => break "SIGINT",
        _ = hup.recv()  => { reload_or_restart().await; }
    }
}
```

For non-tokio code paths, `signal-hook` and `signal-hook-tokio` provide richer signal-set handling.

### 6.3 Cancellation propagation

`tokio_util::sync::CancellationToken` propagates cancellation through subsystems. The Commander holds the root token; each subsystem receives a `child_token()`. On shutdown, `root.cancel()` cascades. Subsystems write their main loop as `select!` against `token.cancelled()` plus their work source.

```rust
loop {
    tokio::select! {
        _ = token.cancelled() => break,
        msg = source.recv() => process(msg).await?,
    }
}
```

### 6.4 Graceful shutdown

`axum_server::Handle::graceful_shutdown(Some(Duration::from_secs(30)))`:

- Stop accepting new connections.
- Allow in-flight requests up to 30 seconds.
- Hard-close anything still active.

For HEC and APIServer, this is the canonical shape. Naked `axum::serve().with_graceful_shutdown(fut)` has no timeout — wrap with `axum_server::Handle` always.

For non-HTTP subsystems (file tailer, TCP receiver), the cancellation token plus a join-with-timeout completes the shape:

```rust
let join = tokio::time::timeout(Duration::from_secs(30), JoinSet::join_all(set)).await;
match join {
    Ok(_) => tracing::info!("subsystems drained"),
    Err(_) => tracing::warn!("subsystems did not drain within timeout, hard exit"),
}
```

### 6.5 SIGHUP semantics

Two conventions, project-wide consistent:

- **SIGHUP = graceful restart.** The process re-execs itself or the supervisor restarts. Matches `Infra.md` LC-R7.
- **SIGHUP = config reload.** The process re-reads config and applies updates that are reload-safe (token list, log level); rejects updates that aren't (port bindings) and logs a warning.

Pick one. The reload-safe variant requires `arc_swap::ArcSwap<Config>` (§5.4) and a clear list of which fields are reload-safe.

### 6.6 sd_notify (systemd integration)

`sd-notify` crate integrates with systemd `Type=notify`:

```rust
sd_notify::notify(false, &[NotifyState::Ready])?;          // after init
sd_notify::notify(false, &[NotifyState::Stopping])?;       // entering shutdown
sd_notify::notify(false, &[NotifyState::Status("draining inputs")])?;
```

`Type=notify` units start when `Ready` arrives; the systemd watchdog can be wired similarly. Recommended for production unit files.

### 6.7 Mandate

`tokio::signal::unix` for SIGTERM/SIGINT/SIGHUP. `CancellationToken` from `tokio_util::sync` for hierarchical cancellation. `axum_server::Handle::graceful_shutdown` with timeout for HTTP subsystems. SIGHUP convention chosen project-wide and documented. `Type=notify` systemd unit using `sd-notify`.

### 6.8 Alternatives

- **`tokio::signal::ctrl_c()` only.** Catches SIGINT/Ctrl+C in dev; not enough for production.
- **`signal-hook` only.** Works without tokio; appropriate for a sync binary.
- **No graceful shutdown.** Hard exit drops in-flight requests. Acceptable in dev; HEC ack semantics break in production.

## 7. Async Runtime Configuration

`tokio` runtime tuning is exposed through `tokio::runtime::Builder` and the `#[tokio::main]` macro.

```rust
#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> anyhow::Result<()> { ... }
```

The macro is fine for simple cases. For production, prefer the explicit Builder so the configuration is data-driven from config:

```rust
let rt = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(cfg.runtime.worker_threads)
    .max_blocking_threads(cfg.runtime.blocking_threads)
    .thread_name("spank-worker")
    .thread_stack_size(2 * 1024 * 1024)
    .enable_all()
    .build()?;

rt.block_on(run(cfg))
```

### 7.1 Runtime knobs

- `worker_threads`: number of OS threads for async tasks. Default = number of cores. For an I/O-bound server, this is right. For a server with mixed CPU work, leaving headroom for `spawn_blocking` matters more than the worker count.
- `max_blocking_threads`: cap on the threadpool used by `spawn_blocking` and `tokio::fs`. Default 512. If file I/O is heavy and you're not on `tokio-uring`, raise this.
- `thread_stack_size`: 2 MB is fine; reduce only if measurement shows benefit.
- `event_interval`: how often the scheduler checks I/O readiness. Default 61. Rarely tuned.

### 7.2 Runtime alternatives

- **Current-thread runtime** (`flavor = "current_thread"`): single thread, no work-stealing, no `Send` requirement on tasks. Right for CLI tools and unit tests; wrong for a server.
- **`tokio-uring`**: io_uring-backed runtime. Wraps tokio with a different I/O surface (`tokio_uring::fs::File`, `tokio_uring::net::TcpStream`). Best fit for storage-heavy workloads. Drops some of the tokio ecosystem (axum/hyper need wrapping) — major commitment.
- **`glommio`**: thread-per-core io_uring. Different programming model (no work-stealing, no shared state).

### 7.3 Mandate

Multi-thread runtime configured from `[runtime]` config section. `enable_all()` (I/O + time + signal). Explicit Builder in `main`, not the macro. Runtime built in main thread; `block_on(run(cfg))`.

## 8. Packaging and Distribution

### 8.1 Cargo.toml metadata

Required for a publishable crate:

```toml
[package]
name = "spank"
version = "0.1.0"
edition = "2024"
rust-version = "1.86"
license = "MIT"
description = "Splunk-compatible log search, single binary."
repository = "https://github.com/.../spank"
homepage = "https://example.com/spank"
documentation = "https://docs.rs/spank"
readme = "README.md"
keywords = ["splunk", "logs", "hec", "spl"]
categories = ["command-line-utilities", "database-implementations"]
```

### 8.2 Binary distribution

Three production paths:

- **Static-linked musl binary**: `cargo build --release --target x86_64-unknown-linux-musl`. One file, runs on any glibc-or-musl Linux. Use `cargo-zigbuild` (zig as the linker) for easier cross-compilation including macOS.
- **Container image**: Distroless or scratch base with the static binary copied in. ~15 MB image. `Dockerfile` ships in the repo.
- **Distribution packages**: `cargo-deb` for `.deb`, `cargo-generate-rpm` for `.rpm`. Each pulls metadata from `[package.metadata.deb]` and `[package.metadata.generate-rpm]` in `Cargo.toml`.

### 8.3 cargo-dist

`cargo-dist` (axodotdev/cargo-dist) generates a release pipeline: builds for declared targets, signs artifacts, produces a GitHub release, generates `cargo-binstall` metadata, optional Homebrew tap and `npm` shim. Recommended for releasing CLI tools to a heterogeneous user base; overkill for a single-platform server.

### 8.4 cargo-binstall

End-user installation: `cargo binstall spank` downloads the precompiled binary for the platform and installs into `~/.cargo/bin`. Works when the project publishes binaries with predictable URLs (cargo-dist's default layout).

### 8.5 systemd unit

```ini
[Unit]
Description=Spank log server
After=network-online.target
Wants=network-online.target

[Service]
Type=notify
ExecStart=/usr/local/bin/spank start --config /etc/spank/server.toml
Restart=on-failure
RestartSec=5s
User=spank
Group=spank
StateDirectory=spank
WorkingDirectory=/var/lib/spank
StandardOutput=journal
StandardError=journal
Environment=RUST_BACKTRACE=1

# Hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/spank /var/log/spank
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
```

Same shape as `Infra.md` §5.4 + §11.6 hardening.

### 8.6 Reproducible builds

`cargo build` is reasonably reproducible if the toolchain is pinned (`rust-toolchain.toml`), the source path is stripped (`RUSTFLAGS='--remap-path-prefix=...'`), and the build host is consistent (or use `cargo-zigbuild` to abstract the linker). Bit-exact reproducibility is feasible with effort; usually not required.

### 8.7 Mandate

Static musl binary as the primary artifact. Distroless container as the secondary. systemd unit included in distribution packages. `Type=notify` integration via `sd-notify`. Hardening directives applied.

### 8.8 Alternatives

- **glibc-linked binary.** Smaller (~30%); requires matching glibc on the host. Fine for OS-packaged installs.
- **`cargo-deb` and `cargo-generate-rpm`.** Native packages with dependencies; right when shipping into apt/yum repos.
- **No binary distribution; `cargo install` only.** Requires Rust toolchain on every install host. Acceptable for developer tools, not for end-user servers.

## 9. CLI Conventions

`clap` derive is the prevailing CLI library. Conventions match GNU + 12-factor:

- `--help` / `-h`, `--version` / `-V` always present.
- `--config` / `-c`, `--verbose` / `-v` (repeatable for level), `--quiet` / `-q`.
- Subcommands: `spank start`, `spank demo`, `spank check-config`, `spank cert ...`.
- Exit codes follow `sysexits.h`: 0 success, 64 EX_USAGE, 78 EX_CONFIG, 75 EX_TEMPFAIL, etc. The `sysexits` crate provides constants.
- stdout for primary output (search results, exported config); stderr for diagnostics and progress.
- `clap-verbosity-flag` provides `-v`/`--quiet` mapping to log levels.
- `clap_complete` generates shell completions for bash/zsh/fish; package them in the distribution.

### 9.1 Mandate

`clap` derive. Subcommands. Standard short flags. sysexits. stdout-vs-stderr discipline. Shell completions.

## 10. Health, Readiness, Observability Endpoints

Three endpoints, conventionally on a dedicated port that is not the user-facing API:

| Path | Purpose | Codes |
|---|---|---|
| `/healthz` | Liveness — am I running? | 200 always while the process is responsive. |
| `/readyz` | Readiness — am I ready to serve? | 200 when subsystems are up; 503 during init or drain. |
| `/metrics` | Prometheus exposition | 200 with text/plain. |
| `/metrics/json` | Compact JSON for non-Prometheus consumers | 200 with application/json. |

`tower::ServiceBuilder` plus a small `axum::Router` mounted on the observability port. In Spank, the user-facing HEC-readiness state machine (docs/Sparst.md §11.6) drives `/readyz`: `READY` → 200, anything else → 503.

### 10.1 Separation from the data plane

The observability port is separate from HEC and from the management API: a single saturated listener cannot starve the others. Vector exposes `/metrics` on `api.address` (default 127.0.0.1:8686) distinct from sources.

### 10.2 Mandate

Dedicated observability port. `/healthz`, `/readyz`, `/metrics`. Readiness reflects subsystem state.

## 11. Process and Threading Model

Single binary, single OS process, tokio multi-thread runtime. No fork/exec; no multi-process supervision (let systemd or the container runtime supervise).

### 11.1 Threads

- **tokio worker threads**: `worker_threads` count. Run async tasks.
- **tokio blocking pool**: `max_blocking_threads`. `spawn_blocking` and `tokio::fs` use this.
- **rayon pool** (optional): for CPU-parallel work alongside tokio. Bulk file scans, parser warm-ups. Configured via `rayon::ThreadPoolBuilder` once at startup.
- **explicit `std::thread::spawn`**: for genuine OS threads with non-async semantics. Rare. Test fixtures sometimes use this.

### 11.2 Lifecycle

The Commander pattern from Spast §3 maps to the Rust runtime:

- Main thread calls `rt.block_on(commander.run())`.
- `Commander::run` spawns subsystems via `JoinSet` with cancellation tokens.
- Each subsystem owns its inner tasks (HEC owns per-connection handlers, Indexer owns workers).
- Signal handler triggers `root_token.cancel()`.
- `JoinSet::join_all` with timeout drains.

### 11.3 Mandate

Single-process, multi-thread tokio. `JoinSet` for subsystem set. `CancellationToken` hierarchy. Optional `rayon` pool for CPU work.

## 12. Test Harness Infrastructure

### 12.1 Test runner

`cargo nextest` for parallelism and JUnit output. Replaces `cargo test`.

### 12.2 Test patterns

- `#[tokio::test]` for async tests. `flavor = "multi_thread"` when the test exercises spawned tasks.
- `tempfile::TempDir` for filesystem isolation.
- Bind-to-zero (`TcpListener::bind("127.0.0.1:0")` then read the port) for network isolation. No port-base config in tests.
- `wiremock` for outbound HTTP mocking.
- `axum::body::Body` constructed directly for inbound testing without a real socket.
- `testcontainers-rs` for tests that need real Postgres/Redis. Gated behind `--features integration`.

### 12.3 Property and fuzz

- `proptest` for property tests (parsers, codecs).
- `cargo-fuzz` with `libfuzzer-sys` for fuzzing. Ships under `fuzz/` directory; runs on a schedule, not per-PR.

### 12.4 Snapshots

`insta` for golden-file tests. `cargo insta review` to update.

### 12.5 Benchmarks

`criterion` under `benches/`. Per-PR runs are noisy; nightly schedule with regression gating is the production pattern.

### 12.6 Mandate

Nextest, tokio::test, tempfile + bind-to-zero, proptest, insta, criterion. integration tests gated by feature flag.

## 13. Build-Time Tooling

- `build.rs`: only when generating code is unavoidable (FFI bindgen, version embedding via `vergen`). Keep small.
- `cargo-watch` for the dev loop: `cargo watch -x check -x test`.
- `cargo-expand` for macro debugging.
- `just` (a simple task runner) over Makefile for project-specific scripts. `cargo-make` is heavier; pick `just`.
- `xtask` pattern: a workspace-internal binary crate (`xtask/`) implements project commands (`cargo xtask release`, `cargo xtask lint-summary`) so they share the workspace toolchain pin. Used by Vector under `vector-vrl/cli/`-style binaries.

### 13.1 Mandate

`just` for shell-task runner. `xtask` for project commands. `cargo-watch` recommended for dev. `vergen` for version embedding.

## 14. Side-by-Side with Infra.md

A direct comparison of subsystem decisions in `Infra.md` against this document.

| Subsystem | Infra.md (Python) | Infrust (Rust) | Notes |
|---|---|---|---|
| Logging facade | stdlib `logging` with custom `JSONFormatter` | `tracing` + `tracing-subscriber` | Tracing adds spans; JSON shape similar. |
| Log levels | DEBUG/INFO/WARNING/ERROR/CRITICAL (CRITICAL unused) | TRACE/DEBUG/INFO/WARN/ERROR | TRACE is new; CRITICAL drops. |
| Log sink | stderr only | stderr only | Identical 12-factor stance. |
| Metrics | In-process counters via `get_stats()` | `metrics` crate + `metrics-exporter-prometheus` | Rust path has a Prometheus exposition for free. |
| Tracing | Not addressed | `tracing-opentelemetry` optional | Only Rust enables distributed tracing. |
| Error base | `AppError` + `thiserror`-like enum hierarchy | `thiserror` per crate + `anyhow` in binary | Stronger discipline; library/application split is enforced. |
| Config format | TOML via `tomllib` | TOML via `serde + toml` | Same format. |
| Config layering | defaults → file → CLI → env | defaults → file → env → CLI via `figment` | Equivalent; `figment` is the canonical Rust composer. |
| Config validation | `__post_init__` validate; collect all | `validator` derive + `validate` method | Equivalent shape. |
| Config reload | None; SIGHUP = restart | None by default; `ArcSwap` available for opt-in | Default policy unchanged. |
| Runtime model | threading, no asyncio | tokio multi-thread | Different model; same supervision shape. |
| Signal handling | `signal` module sets event | `tokio::signal::unix` + `CancellationToken` | Hierarchical cancellation in Rust. |
| Graceful shutdown | `ExitStack` reverse-order | `axum_server::Handle::graceful_shutdown` + `JoinSet` with timeout | Both use timeout-bounded shutdown. |
| Packaging | `wheel` + `pip install` | musl static binary + container + `.deb`/`.rpm` | Rust ships fully static; Python depends on interpreter. |
| systemd unit | `Type=simple`, `StandardOutput=journal` | `Type=notify` with `sd-notify` integration | Notify lets systemd track readiness. |
| CLI | `argparse` per `Infra.md` §4.3 | `clap` derive | Clap supports completions for free. |
| Health endpoint | Designed, deferred | `axum::Router` on observability port | Built-in. |
| Test runner | `pytest` | `cargo nextest` | Equivalent for parallel CI. |
| Test isolation | tempdir + bind-to-zero | tempfile + bind-to-zero | Identical. |

The pattern is consistent: Rust does not change Spank's architectural decisions; it changes the components that implement them. The discipline (frozen config, four-source layering, structured logs to stderr, signal-driven graceful shutdown, single supervised process) carries across.

### 14.1 Where Rust changes the calculus

- **Stricter library/application error split.** `thiserror` for libraries vs `anyhow` for applications is stronger than Python's hierarchy and forces the boundary to be visible.
- **Built-in metrics exposition.** `metrics-exporter-prometheus` is one crate and one mount; Python equivalents (prometheus_client) are similar but require more wiring.
- **Type-safe configuration.** `serde::Deserialize` validates shape at parse; mismatched fields refuse to compile-load. Python's frozen dataclasses approximate this at runtime.
- **`async fn` in traits + cancellation tokens.** A more uniform supervision shape than Python's threading-event-plus-stop-method pattern.
- **Static binaries.** Distribution simplifies dramatically.

### 14.2 Where Rust does not help

- **Architectural decisions** — Commander, Supervisor, two-plane data flow, durability watermark — are language-neutral. Spanker.md and `docs/Sparst.md` remain authoritative.
- **Shipper interop** — protocol shape, not language.
- **SPL semantics** — grammar choices, planner shape, operator behavior. Language-neutral.
- **Operator experience** — config-file shape, CLI flag spelling, log line content. Both languages must converge on the same external surface.

The Rust port is an implementation choice. Architecture, product positioning, terminology, and verification discipline survive the choice intact.
