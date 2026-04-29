# Observability — Logs, Metrics, and Profiling

`Focus: reference` — the contract between the runtime and the operator: log macros, metric names and types, baseline workload, and profiler choices. Does not receive task lists, status text, or per-metric implementation notes that belong in `Plan.md`.

This document changes when a metric is added or renamed, a log macro category changes, or the profiling recommendation changes. Metric names listed here must match the constants in `spank-obs::metrics::names` and the macros in `spank-obs::lib`; if a name needs to change, change it in code and here in the same commit. Sibling reference documents: `docs/Errors.md` (the error and backpressure paths whose state the metrics surface).

---

## Table of Contents

1. [Logs](#1-logs)
2. [Metrics](#2-metrics)
3. [Baseline numbers](#3-baseline-numbers)
4. [Profiling](#4-profiling)

---

## 1. Logs

Logs are emitted through `tracing` and rendered by `spank-obs::init_tracing`. The
single entry point is `init_tracing(&TracingConfig)`, called once near the top of
`main`. The function is idempotent — subsequent calls return `Ok(())` without
changing the subscriber, so a partial init can never wedge the process.

Configuration knobs expose three controls. `tracing.format` accepts `pretty`
(development) or `json` (machines), defaulting to `pretty`. `tracing.filter`
accepts a `tracing_subscriber::EnvFilter` directive, defaulting to `info,spank=info`;
`RUST_LOG` overrides this when set, matching ecosystem expectations. `tracing.file`
is an optional path; when set, a daily-rotated JSON appender is added in addition
to stdout, and a `WorkerGuard` is held in a process-wide `OnceCell` to keep the
background flush thread alive for the program lifetime.

Categorized macros wrap `tracing::info!` and `tracing::error!` with a `category`
field so dashboards and log shippers can route on a single key:

- `ingest_event!` — record movement, queue admission, sentinel propagation.
- `lifecycle_event!` — phase changes, signals, child task start/stop.
- `error_event!` — operational errors with the full `SpankError` recovery class.
- `audit_event!` — auth decisions, principal allow/deny.

All four set `target = "spank.<category>"` so an operator can subset by target
without parsing JSON.

## 2. Metrics

Metrics are emitted via the `metrics` facade and exported by
`metrics-exporter-prometheus`. `spank-obs::install_prometheus()` installs the
recorder and returns a `MetricsHandle`, whose `render()` produces the Prometheus
exposition for `/metrics/prometheus`. A second constructor,
`install_prometheus_with_listener(addr)`, binds a dedicated HTTP listener for
deployments that prefer to keep scrapes off the API port.

The naming convention is `spank.<subsystem>.<noun>_<unit>`. The table below lists
every metric currently defined; the type, label (if any), and operational meaning
for each.

| Name | Type | Meaning |
| - | - | - |
| `spank.hec.requests_total` | counter | HEC requests received, before auth. |
| `spank.hec.bytes_in_total` | counter | Compressed body bytes received on HEC. |
| `spank.hec.outcome_code_total` | counter, label `code` | Splunk HEC outcome code counts (0=success, 9=server-busy, etc). |
| `spank.queue.depth_current` | gauge | In-flight items in the HEC ingress channel. |
| `spank.queue.full_total` | counter | Times the bounded HEC channel rejected a try-send. |
| `spank.file.bytes_read_total` | counter | Bytes consumed by `FileMonitor`. |
| `spank.file.lines_read_total` | counter | Lines emitted by `FileMonitor`. |
| `spank.tcp.bytes_in_total` | counter | Bytes read on TCP receiver sockets. |
| `spank.tcp.bytes_out_total` | counter | Bytes written on TCP shipper sockets. |
| `spank.tcp.connections_current` | gauge | Open inbound TCP connections. |
| `spank.tcp.syscall_errors_total` | counter, label `syscall` | Failed syscalls (`accept`, `read`, `write`, `set_nodelay`, `bind`). |
| `spank.store.inserts_total` | counter | Rows committed to the store. |
| `spank.store.insert_duration_seconds` | histogram | Wall-clock duration of `commit()`. |
| `spank.process.panics_total` | counter | Caught panics from worker tasks. |

Call sites use the constants in `spank-obs::metrics::names`, never literal
strings; renaming a metric is one edit instead of grep-and-replace.

Two items in this table are flagged for future resolution. `spank.tcp.syscall_errors_total`
uses the label key `syscall` in the emitting code (`spank-tcp::receiver`); any
dashboard or alert expression must use `syscall`, not `op` — any existing dashboards
built against `op` must be updated. `spank.store.insert_duration_seconds` is listed
as a live histogram but the SQLite backend (`spank-store::sqlite`) does not currently
emit it — only the counter `spank.store.inserts_total` is incremented at the commit
site; the histogram constant exists in `spank-obs::metrics::names` and instrumenting
`SqliteBackend::commit` is the remaining step.

## 3. Baseline numbers

The `spank bench` subcommand runs a small SQLite bulk-insert workload to
establish a per-host floor. It opens a temp directory, creates a hot bucket,
materializes 100 000 `Record` rows, calls `append` and `commit`, and prints
`elapsed_ms` and `inserts_per_sec`. The workload is intentionally narrow: it
exercises the storage write path with the tuned PRAGMAs (`WAL`, `NORMAL`,
`MEMORY` temp store, 256 MB mmap) and nothing else, so a regression in the
number points at the storage layer.

Sample run (M-series Mac, debug build):

```
$ cargo run -q --bin spank -- bench
sqlite bulk_insert n=100000 elapsed_ms=… inserts_per_sec=…
```

Release builds typically deliver an order of magnitude improvement; record both
when capturing a baseline. A future `benches/` harness will wrap the same path
under `criterion` for jitter-aware reporting; until then, the subcommand is the
canonical baseline.

## 4. Profiling

Profiling is opt-in and out of band. Three tools cover the common investigation
scenarios. `samply` provides sampling CPU profiles on macOS and Linux with low
overhead and a flamegraph UI. `tokio-console` (gated by a `tokio_unstable` build)
provides runtime-task introspection; the runtime is configured for it in
development, but production builds do not enable `tokio_unstable`. `cargo flamegraph`
produces one-off flamegraphs and works against the `bench` subcommand directly.

No profiler is wired into the default binary. The decision is deliberate: a
profiler that is always-on either costs measurable overhead or hides behind a
flag that nobody flips during an incident. Operators reach for one of the tools
above when investigating, and the metrics in §2 tell them which subsystem to
target.

---

## References

[1] `metrics` crate, *metrics facade*, docs.rs/metrics — counter, gauge, histogram macros and recorder API.
[2] `metrics-exporter-prometheus`, docs.rs/metrics-exporter-prometheus — `MetricsHandle::render()` and `install_prometheus_with_listener`.
[3] `tracing-subscriber`, docs.rs/tracing-subscriber — `EnvFilter`, `fmt::Layer`, `OnceCell`-based subscriber init.
[4] `tracing-appender`, docs.rs/tracing-appender — `RollingFileAppender` and `WorkerGuard` lifetime.
