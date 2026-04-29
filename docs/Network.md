# Network — Library Stack and Rationale

`Focus: reference` — the rationale for every network-library choice in the tree: HTTP server, TLS posture, async runtime, bounded channels, raw TCP, compression, observability transport, and storage. Written from the perspective of someone who has to operate the result, not someone shopping a library list. Does not receive code snippets that belong inside the source crates' rustdoc, or work items that belong in `Plan.md`.

This document changes when a library is added, replaced, or its operational mode changes. Sibling reference documents: `docs/Errors.md` (the error and backpressure model that the channel choices feed into), `docs/Observability.md` (the metrics emitted from the TCP and storage paths described here).

---

## Table of Contents

1. [HTTP server — `axum` 0.7 + `axum-server` 0.7](#1-http-server--axum-07--axum-server-07)
2. [TLS — deferred](#2-tls--deferred)
3. [Async runtime — `tokio` 1.40 (multi-thread)](#3-async-runtime--tokio-140-multi-thread)
4. [Bounded channels — `tokio::sync::mpsc`](#4-bounded-channels--tokiosynmpsc)
5. [TCP — raw `tokio::net`](#5-tcp--raw-tokionet)
6. [Compression — `flate2`](#6-compression--flate2)
7. [Observability transport — `metrics-exporter-prometheus`](#7-observability-transport--metrics-exporter-prometheus)
8. [Storage — `rusqlite`](#8-storage--rusqlite)
9. [What we deliberately did not take](#9-what-we-deliberately-did-not-take)
10. [Inspection points](#10-inspection-points)

---

## 1. HTTP server — `axum` 0.7 + `axum-server` 0.7

The API surface (`spank-api`) is built on `axum`. The current `spank-api::server::serve`
implementation uses `axum::serve(listener, ...).with_graceful_shutdown(shutdown)` directly —
`axum-server` is a workspace dependency but its bind-and-serve path is not active yet. The
combination wins on three operational properties:

- *Tower middleware composition.* Auth, request logging, metrics, and timeouts
  are all `tower::Service` layers. We get `tower-http`'s `TraceLayer` and
  `RequestBodyLimitLayer` for free, with no custom code in the hot path.
- *Hyper 1.x foundation.* `axum` 0.7 sits on `hyper` 1.x, which means we are on
  the supported track for HTTP/2 and HTTP/1 keep-alive tuning without piling
  workarounds on a deprecated stack.
- *Graceful shutdown that actually works.* `axum::serve().with_graceful_shutdown`
  stops the listener and lets in-flight handlers finish. Combined with our lifecycle
  token, the API drains cleanly under SIGINT. `axum-server`'s `Handle::graceful_shutdown`
  adds a per-connection timeout on top of that; it is the intended upgrade path for
  production once a bounded drain timeout is required.

Alternatives considered: `actix-web` performs slightly better in synthetic
benchmarks but ties the runtime to `actix`'s actor model and adds an
abstraction layer between the request and the tokio task. `warp` is filter-based
and has lost momentum. `rocket` requires nightly features for several useful
patterns. `axum` was the lowest-friction choice given that the tree is already on tokio.

## 2. TLS — deferred

There is no TLS termination in the binary. The expectation is that production
deployments terminate TLS at a load balancer (Envoy, Caddy, nginx) and forward
HTTP/1.1 keep-alive to `spank`. When TLS becomes a binary concern, the route is
`rustls` via `axum-server`'s `rustls` feature — which is exactly why `axum-server`
is already a workspace dependency. `axum-server` supports `rustls` natively via
`axum_server::tls_rustls::RustlsConfig`; the migration from `axum::serve` to
`axum_server::bind_rustls` is the planned step when TLS lands. OpenSSL is not on the
table. Rationale: `rustls` keeps us in the safe-Rust dependency tree; OpenSSL would
add a system library to the build matrix and a CVE channel we would otherwise
not own.

## 3. Async runtime — `tokio` 1.40 (multi-thread)

The whole tree is `#[tokio::main]`-free; the runtime is built explicitly in
`main.rs::build_runtime` with `Builder::new_multi_thread().enable_all()`. Worker
thread count is configurable through `runtime.worker_threads`. The reason for
explicit construction over the macro: `cfg.runtime.worker_threads` flows in
from figment, and the macro takes a literal. Threads are named `spank-worker`
so they show up identifiably in profiles.

Alternative: `async-std`. Out — the rest of the ecosystem we lean on (`hyper`,
`axum`, `tracing-tokio-console`, `tokio-rustls`) is tokio-native, and a mixed
runtime is worse than picking one and committing.

## 4. Bounded channels — `tokio::sync::mpsc`

HEC ingress and TCP event flow are both `tokio::sync::mpsc::channel` with
explicit capacity. We use `try_send` on the producer side so backpressure shows
up as a `QueueFull` error instead of a hidden `await`. See `docs/Errors.md §3`
for why this matters operationally.

Alternative: `flume` has marginally better single-producer throughput. No benchmark
currently shows we are paying for `tokio::sync::mpsc` overhead in any subsystem we
care about, and gaining a second channel implementation costs more in cognitive
overhead than it recovers. If profiling later identifies an mpsc bottleneck, the
migration is local.

## 5. TCP — raw `tokio::net`

`spank-tcp` and `spank-shipper` use `tokio::net::TcpListener`, `TcpStream`,
`AsyncReadExt`, and `AsyncWriteExt` directly. There is no framing crate in the
loop. The receiver newline-frames in-line with a small buffer; we fully own
syscall attribution (`accept`, `read`, `write`, `set_nodelay`, `bind`) and
report each as a labeled `spank.tcp.syscall_errors_total` increment. The
shipper backs off exponentially on connect failure (100ms initial, 30s cap)
with structured `error_event!` per attempt.

Alternative: `tokio-util::codec::LinesCodec` plus `Framed`. Out for two reasons.
First, the codec layer hides syscall errors behind a `tokio::io::Error` that has
already been stripped of operation context — the framing layer does not know whether
the underlying error was a `read` or a `poll_read_ready`. Second, the line cap
(`tcp.max_line_bytes`) is a security boundary; encoding it into the codec is fine,
but the codec's own buffer growth strategy is opinionated and not what is wanted
under attack. Direct `read_buf` with explicit bounds is fewer surprises.

## 6. Compression — `flate2`

HEC accepts gzip; we decode with `flate2`'s `read::GzDecoder`. The crate is
mature, single-implementation, no surprises. Brotli and zstd are not yet
required by any client we care about; when they become required, `async-compression`
is the bridge.

## 7. Observability transport — `metrics-exporter-prometheus`

Prometheus is a pull model; the exporter renders into a string on demand and
the API serves it on `/metrics/prometheus`. The decision to render on demand
rather than push (statsd, OTLP) is operationally cheap: no exporter agent on
the host, no aggregation outside the binary, scrapes are visible in the
request log. OTLP support can be added as a parallel export later without
touching call sites — the `metrics` facade decouples them.

## 8. Storage — `rusqlite`

Not strictly a network library, but worth noting for the stack picture.
`rusqlite` is a synchronous bindings crate; the writer is therefore wrapped in
`spawn_blocking` at call sites that touch tokio. `sqlx` was rejected because
its async-everywhere model adds executor cost to a workload that is already
CPU-bound on serialization, and the SQLite driver inside `sqlx` is `rusqlite`
under the hood anyway. PRAGMA tuning (WAL, NORMAL synchronous, MEMORY temp
store, 256MB mmap) lives in `SqliteBackend::tune`.

## 9. What we deliberately did not take

The following crates were explicitly considered and rejected. Each rejection is
permanent unless the stated condition changes.

- `tonic` (gRPC). The Splunk wire surface is HTTP/JSON; there is no gRPC
  consumer to justify the build-time cost of `prost` and `protoc`.
- `actix-rt`. See §3.
- `mio`/`socket2` directly. The tokio abstractions are sufficient for everything
  currently in scope; dropping to raw fds would be the right call only if we
  needed `SO_REUSEPORT`-style accept sharding.
- `tracing-opentelemetry`. Adds a dependency tree that pulls in tonic, prost,
  and a vendor-specific exporter pinning. When OTLP is wanted, it will be added
  behind a feature flag.

## 10. Inspection points

The following are the highest-risk areas in the network stack for a reviewer.
Each names the exact code path and what to verify.

1. The graceful-shutdown wiring in `spank-api::server::serve` — currently
   `axum::serve().with_graceful_shutdown(shutdown)`; confirm that the shutdown
   future actually drains in-flight requests rather than dropping them on
   cancellation. When `axum-server` is activated for TLS (§2), the drain
   timeout should move to `axum_server::Handle::graceful_shutdown`.
2. The accept-loop backoff in `spank-tcp::listener::serve` — bounded backoff
   on accept errors prevents fd-exhaustion spin, but the bounds (10ms → 1000ms)
   were picked from intuition; a real load test should set them.
3. The TCP receiver's buffer growth in `spank-tcp::receiver::run_connection` —
   the line cap is enforced, but the reuse strategy across reads is worth a
   second pair of eyes.
4. The shipper reconnect strategy in `spank-shipper::tcp::TcpSender::run` —
   pure exponential backoff (100ms → 30s cap) with no jitter. The jitter
   concern is a standard fleet-operations observation: when many shipper
   instances reconnect simultaneously after a receiver restart, synchronized
   retry storms can prevent the receiver from recovering. The topic surfaced
   from `Tracks.md Track 4` and the inspection points carried forward here.
   Jitter is deferred; add it before deploying to a fleet of more than a
   handful of shippers.

---

## References

[1] axum project, *axum::serve*, docs.rs/axum — `with_graceful_shutdown` documentation.
[2] axum-server project, *axum_server::tls_rustls*, docs.rs/axum-server — `RustlsConfig` and `bind_rustls`.
[3] Tokio project, *tokio::sync::mpsc*, docs.rs/tokio — bounded channel and `try_send` semantics.
[4] Bram van den Heuvel, *Thundering herd problem*, general distributed systems literature — synchronized retry storms under receiver restart.
