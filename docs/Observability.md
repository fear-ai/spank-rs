# Observability

Scope: how `spank-rs` produces logs, metrics, and baseline numbers, and where each
signal originates. This document is the contract between the runtime and the
operator. Names listed here MUST match the constants in `spank-obs::metrics::names`
and the macros in `spank-obs::lib`; if a name needs to change, change it in code
and here in the same commit.

## 1. Logs

Logs are emitted through `tracing` and rendered by `spank-obs::init_tracing`. The
single entry point is `init_tracing(&TracingConfig)`, called once near the top of
`main`. The function is idempotent — subsequent calls return `Ok(())` without
changing the subscriber, so a partial init can never wedge the process.

Configuration knobs:

- `tracing.format` — `pretty` (development) or `json` (machines). Default `pretty`.
- `tracing.filter` — `tracing_subscriber::EnvFilter` directive. Default
  `info,spank=info`. `RUST_LOG` overrides this when set, matching ecosystem
  expectations.
- `tracing.file` — optional path. When set, a daily-rotated JSON appender is
  added in addition to stdout, and a `WorkerGuard` is held in a process-wide
  `OnceCell` to keep the background flush thread alive for the program lifetime.

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

The naming convention is `spank.<subsystem>.<noun>_<unit>`:

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
| `spank.tcp.syscall_errors_total` | counter, label `op` | Failed syscalls (`accept`, `read`, `write`, `set_nodelay`, `bind`). |
| `spank.store.inserts_total` | counter | Rows committed to the store. |
| `spank.store.insert_duration_seconds` | histogram | Wall-clock duration of `commit()`. |
| `spank.process.panics_total` | counter | Caught panics from worker tasks. |

Call sites use the constants in `spank-obs::metrics::names`, never literal
strings; renaming a metric is one edit instead of grep-and-replace.

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

Profiling is opt-in and out of band. Recommended tools:

- `samply` for sampling CPU profiles on macOS and Linux.
- `tokio-console` (gated by a `tokio_unstable` build) for runtime-task
  introspection. The runtime is configured for it in development; production
  builds do not enable `tokio_unstable`.
- `cargo flamegraph` for one-off flamegraphs; works against the `bench`
  subcommand directly.

No profiler is wired into the default binary. The decision is deliberate: a
profiler that is always-on either costs measurable overhead or hides behind a
flag that nobody flips during an incident. Operators reach for one of the tools
above when investigating, and the metrics above tell them which subsystem to
target.
