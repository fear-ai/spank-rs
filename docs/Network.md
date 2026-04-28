# Rust Network Library Review

Scope: which crates `spank-rs` depends on for network I/O, why each was chosen,
and what the alternatives would have cost. Written from the perspective of
someone who has to operate the result, not someone shopping a library list.

## 1. HTTP server — `axum` 0.7 + `axum-server` 0.7

The API surface (`spank-api`) is built on `axum`, with `axum-server` providing
the bind-and-serve loop with `graceful_shutdown` keyed off a tokio cancellation
token. The combination wins on three operational properties:

- *Tower middleware composition.* Auth, request logging, metrics, and timeouts
  are all `tower::Service` layers. We get `tower-http`'s `TraceLayer` and
  `RequestBodyLimitLayer` for free, with no custom code in the hot path.
- *Hyper 1.x foundation.* `axum` 0.7 sits on `hyper` 1.x, which means we are on
  the supported track for HTTP/2 and HTTP/1 keep-alive tuning without piling
  workarounds on a deprecated stack.
- *Graceful shutdown that actually works.* `axum_server::Server::bind`
  implements `with_graceful_shutdown` such that the listener stops accepting
  while in-flight handlers finish. Combined with our lifecycle token, the API
  drains cleanly under SIGINT. Hand-rolled `hyper::Server` code paths in older
  designs needed extra plumbing for the same behavior.

Alternatives considered. `actix-web` performs slightly better in synthetic
benchmarks but ties the runtime to `actix`'s actor model and adds an
abstraction layer between the request and the tokio task. `warp` is filter-
based and has lost momentum. `rocket` requires nightly features for several
useful patterns. `axum` was the lowest-friction choice given that we are
already on tokio.

## 2. TLS — deferred

There is no TLS termination in the binary. The expectation is that production
deployments terminate TLS at a load balancer (Envoy, Caddy, nginx) and forward
HTTP/1.1 keep-alive to `spank`. When TLS becomes a binary concern, the route is
`rustls` via `axum-server`'s `rustls` feature; OpenSSL is not on the table.
Rationale: `rustls` keeps us in the safe-Rust dependency tree; OpenSSL would
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
up as a `QueueFull` error instead of a hidden `await`. See
`docs/Errors.md §3` for why this matters operationally.

Alternatives. `flume` has marginally better single-producer throughput; we do
not have a benchmark that says we are paying for `tokio::sync::mpsc` overhead
in any subsystem we currently care about, and gaining a second channel
implementation costs us more in cognitive overhead than we would recover. If
profiling later identifies an mpsc bottleneck, the migration is local.

## 5. TCP — raw `tokio::net`

`spank-tcp` and `spank-shipper` use `tokio::net::TcpListener`, `TcpStream`,
`AsyncReadExt`, and `AsyncWriteExt` directly. There is no framing crate in the
loop. The receiver newline-frames in-line with a small buffer; we fully own
syscall attribution (`accept`, `read`, `write`, `set_nodelay`, `bind`) and
report each as a labeled `spank.tcp.syscall_errors_total` increment. The
shipper backs off exponentially on connect failure (100ms initial, 30s cap)
with structured `error_event!` per attempt.

Alternative: `tokio-util::codec::LinesCodec` plus `Framed`. Out for two
reasons. First, the codec layer hides syscall errors behind a `tokio::io::Error`
that has already been stripped of operation context — the framing layer does
not know whether the underlying error was a `read` or a `poll_read_ready`.
Second, our line cap (`tcp.max_line_bytes`) is a security boundary; encoding it
into the codec is fine, but the codec's own buffer growth strategy is opinionated
and not what we want under attack. Direct `read_buf` with explicit bounds is
fewer surprises.

## 6. Compression — `flate2`

HEC accepts gzip; we decode with `flate2`'s `read::GzDecoder`. The crate is
mature, single-implementation, no surprises. Brotli and zstd are not yet
required by any client we care about; when they become required, `async-
compression` is the bridge.

## 7. Observability transport — `metrics-exporter-prometheus`

Prometheus is a pull model; the exporter renders into a string on demand and
the API serves it on `/metrics/prometheus`. The decision to render on demand
rather than push (statsd, OTLP) is operationally cheap: no exporter agent on
the host, no aggregation outside the binary, scrapes are visible in the
request log. OTLP support can be added as a parallel export later without
touching call sites — the `metrics` facade decouples them.

## 8. Storage — `rusqlite` 0.32

Not strictly a network library, but worth noting for the stack picture.
`rusqlite` is a synchronous bindings crate; the writer is therefore wrapped in
`spawn_blocking` at call sites that touch tokio. `sqlx` was rejected because
its async-everywhere model adds executor cost to a workload that is already
CPU-bound on serialization, and the SQLite driver inside `sqlx` is `rusqlite`
under the hood anyway. PRAGMA tuning (WAL, NORMAL synchronous, MEMORY temp
store, 256MB mmap) lives in `SqliteBackend::tune`.

## 9. What we deliberately did *not* take

- `tonic` (gRPC). The Splunk wire surface is HTTP/JSON; there is no gRPC
  consumer to justify the build-time cost of `prost` and `protoc`.
- `actix-rt`. See §3.
- `mio`/`socket2` directly. The tokio abstractions are sufficient for everything
  we currently do; dropping to raw fds would be the right call only if we needed
  `SO_REUSEPORT`-style accept sharding, and we do not.
- `tracing-opentelemetry`. Adds a dependency tree that pulls in tonic, prost,
  and a vendor-specific exporter pinning. When we want OTLP, we will add it
  behind a feature flag.

## 10. Inspection points

The Network Library Implementer reviewing this stack should look hardest at:

1. The `axum-server` graceful-shutdown wiring in `spank-api::server::serve` —
   confirm that `with_graceful_shutdown` actually drains rather than dropping
   in-flight requests on cancellation.
2. The accept-loop backoff in `spank-tcp::listener::serve` — bounded backoff
   on accept errors prevents fd-exhaustion spin, but the bounds (10ms → 1000ms)
   were picked from intuition; a real load test should set them.
3. The TCP receiver's buffer growth in `spank-tcp::receiver::run_connection` —
   the line cap is enforced, but the reuse strategy across reads is worth a
   second pair of eyes.
4. The shipper reconnect strategy in `spank-shipper::tcp::TcpSender::run` —
   exponential backoff with no jitter is a thundering-herd hazard at scale; a
   review may want to add jitter before this ships to a real fleet.
