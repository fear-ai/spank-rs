# Tracks — outcomes, deferrals, and inspection points

This document is the per-track close-out for the implementation pass. Each track
has the same shape: scope, what landed, what was deferred and why, and the
specific places a reviewer should look hardest. The build is green
(`cargo build --workspace` and `cargo test --workspace` both clean) at the time
this document was written; any drift after that point should update the relevant
track here in the same commit.

The companion documents are:

- `docs/Observability.md` — log macros, metric names, and baseline.
- `docs/Errors.md` — the `SpankError` taxonomy and shutdown composition.
- `docs/Network.md` — the network stack review.

## Track 1 — Tracing and configuration plumbing

Scope: a single `init_tracing` entry point, a layered figment config stack, and
a unified messaging vocabulary for the rest of the tree to depend on.

Landed:

- `spank-obs::init_tracing(&TracingConfig)` — idempotent, JSON or pretty,
  optional daily-rotated file appender held alive by a process-wide
  `WorkerGuard` in a `OnceCell`. RUST_LOG overrides the configured filter to
  match ecosystem expectation.
- `spank-obs::lib` — `ingest_event!`, `lifecycle_event!`, `error_event!`,
  `audit_event!` macros, each tagging the record with a `category` and a
  `target = "spank.<category>"` so dashboards can route on a single key.
- `spank-cfg::load(Option<&Path>)` — figment stack with defaults → optional TOML
  file → `SP_*` environment overrides → final `validate()`. `render_toml(&cfg)`
  prints the effective merged config; the `spank show-config` subcommand wires
  it.

Deferred:

- A genuine config-schema export (JSON Schema) for editor completion. The
  TOML render serves as a runnable spec for now.
- `tokio-console` is documented in `docs/Observability.md` but not enabled by
  default; turning it on requires a `tokio_unstable` build flag and we have
  not made that the default for security reasons.

Inspection points:

- `spank-cfg::validate` is currently minimal (queue depth and content length).
  Add validators here as new invariants surface; do not push validation into
  call sites.
- `init_tracing` swallows `try_init` errors deliberately so a test that has
  already installed a subscriber does not poison the process. Verify this is
  what you want before changing it.

## Track 2 — Tokio API server

Scope: a multi-thread tokio runtime, an axum router with the Splunk-shaped
endpoints we currently need (real or stubbed), and a graceful shutdown bound
to the lifecycle token.

Landed:

- `spank-api::router::build` — `/health` returning 200/503 by `HecPhase`;
  `/services/server/info` with build metadata; `/metrics/prometheus` (text
  exposition) and `/metrics` (alias); 501 stubs for
  `/services/search/jobs`, `/services/data/indexes`,
  `/services/authentication/users`.
- `spank-api::server::serve(router, addr, lifecycle)` — `axum_server::bind`
  with `with_graceful_shutdown` driven by the lifecycle token.
- Multi-thread runtime built explicitly in `main.rs::build_runtime` so
  `runtime.worker_threads` flows from figment without involving the
  `#[tokio::main]` macro.

Deferred:

- The 501 endpoints will be filled in by Tracks beyond this set
  (search, indexes, principals).
- Per-route auth middleware. The HEC routes carry their own auth; the
  Splunk-style management endpoints are not yet authenticated and are
  therefore behind the API bind, which should default to `127.0.0.1`.

Inspection points:

- `ApiState::set_phase` writes through an `arc_swap` so readers on the hot
  path do not contend. Confirm phase transitions go through
  `HecPhase::can_transition_to` somewhere upstream (currently a comment;
  enforced by convention, not by the type).
- The `/health` endpoint returns 503 in `STARTED` and `STOPPING`. If load
  balancers should drain on `STOPPING` but treat `STARTED` as not-yet-ready,
  the codes are correct; verify against the LB you target.

## Track 3 — HEC receiver

Scope: accept Splunk HTTP Event Collector requests, authenticate, decode,
parse, and hand off to a consumer task that writes per-tag files.

Landed:

- `spank-hec::receiver::HecState` and `routes()` — phase admission gate, body
  length cap, token auth via `Authenticator`, gzip decode via `flate2`, JSON
  envelope and raw line parsing, tag derivation from `index`/`source`/`channel`,
  bounded `try_send` into the consumer channel with `QueueFull` → HEC code 9 /
  HTTP 503.
- `spank-hec::receiver::spawn_consumer` — drains the channel, dispatches `Rows`
  to the `Sender` (`FileSender` by default) and treats `Sentinel::end(tag)` as
  "flush this tag, signal Drain".
- `spank-hec::sender::FileSender` — per-tag `BufWriter`, `sync_all` on flush,
  tag sanitization to keep filenames safe.

Deferred:

- Channel-based ordering guarantees beyond per-tag flush. A future consumer
  may need to interleave tags; today each tag is independent.
- Replay or persistent queueing on the receive side. We rely on the client to
  retry on 503; an embedded WAL is out of scope.

Inspection points:

- `processor::parse_event_body` iterates JSON envelopes; confirm behavior on
  malformed objects matches Splunk's (skip-and-continue vs. fail-fast).
  Splunk's behavior is permissive; our default is permissive too, but the
  HEC code returned to the client should be reviewed.
- Tag derivation rules — currently `channel ?? source ?? index ?? "default"`.
  Some Splunk deployments depend on `sourcetype` participating in tag keying;
  add it here if the deployment requires it.

## Track 4 — File reader and TCP shipper

Scope: read a file (one-shot or tail-with-rotation) and ship its lines over
TCP with reconnect.

Landed:

- `spank-files::FileMonitor::run(line_tx, lifecycle)` — `OneShot` reads to EOF
  then emits `FileOutput::Done(Sentinel::end(path))`; `Tail` follows growth and
  detects rotation by inode change every 200ms.
- `spank-shipper::TcpSender::run(rx, lifecycle)` — connects with exponential
  backoff (100ms → 30s), structured `error_event!` per attempt, writes lines
  with newline framing.
- `spank::main::ship` subcommand wires the two through a bridge that translates
  `FileOutput::Line` to `String` and exits on `Done`.

Deferred:

- Jittered backoff. The shipper currently uses pure exponential backoff; this
  is a thundering-herd hazard at fleet scale. Flagged in
  `docs/Network.md §10`.
- TLS for the shipper. See network-libraries §2 — terminate at an LB.
- Watermarking / persistent ack. The shipper is fire-and-forget; if the
  receiver acks (it does not today), the watermark goes here.

Inspection points:

- Rotation detection by inode change every 200ms misses rapid rotate-twice
  scenarios. If your log rotator can rotate within a 200ms window, narrow the
  poll interval.
- The bridge in `main::ship` translates the file's `FileOutput::Done` to a
  channel-close instead of forwarding a `Sentinel`. The shipper exits cleanly,
  but a future consumer that wants to see the sentinel will need to widen the
  bridge channel type.

## Track 5 — Tracing, metrics, and profiling baseline

Scope: settle the names and shapes; produce one published baseline number so
regressions have a reference.

Landed:

- Metric name constants in `spank-obs::metrics::names`, with the full table
  reproduced in `docs/Observability.md`. Call sites must use the constants.
- The `spank bench` subcommand performs a 100k-row SQLite bulk insert as the
  documented baseline workload.
- Baseline doc covers logs, metrics, and profiler choices (`samply`,
  `tokio-console`, `cargo flamegraph`).

Deferred:

- A `criterion` harness under `benches/`. The directory exists; populating it
  requires adding `criterion` as a workspace dev-dependency, which under
  CLAUDE.md §7 needs explicit approval. The `bench` subcommand serves the same
  role for now and produces a release-mode number on demand.
- Continuous baseline tracking (CI emits the number). Out of scope until CI
  exists.

Inspection points:

- The metric name table is a contract with dashboards. Renames here are
  breaking changes for whoever scrapes us; treat that table as a stable
  surface.
- Histograms for store insert duration use the default Prometheus exporter
  buckets. If your SLOs sit between 10ms and 1s, that is fine; recheck if
  you push beyond.

## Track 6 — Error, recovery, and shutdown

Scope: a single error type with a single recovery classifier, and a documented
shutdown composition.

Landed:

- `SpankError` taxonomy with `Recovery` classifier (`Retryable`,
  `Backpressure`, `FatalComponent`, `FatalProcess`).
- `Lifecycle` (cancellation token tree), `Drain` (tag-keyed Notify with
  latched signaled-set), and `Sentinel` (End / Checkpoint with tag) form the
  shutdown primitives.
- `serve` orchestration in `main.rs` wires SIGINT → `lifecycle.shutdown()`,
  axum `with_graceful_shutdown`, and per-subsystem `tokio::time::timeout`
  bounded by `runtime.shutdown_seconds`.
- Documented end-to-end in `docs/Errors.md`.

Deferred:

- A panic hook that increments `spank.process.panics_total` and tries to log
  the panic into the tracing subscriber. Today panics in spawned tasks land
  on stderr and the metric stays at zero. Small, focused follow-up.

Inspection points:

- `Drain::wait` returns `bool` to indicate whether the wait timed out; today
  no caller checks the return. If you make a wait fallible, audit those
  call sites.
- The shutdown budget is per-handle, not per-process. A worst-case sequential
  shutdown costs N × `shutdown_seconds`. Confirm that is acceptable.

## Track 7 — Storage interface

Scope: a backend trait surface and a working SQLite implementation, with hooks
for DuckDB and Postgres backends to land later.

Landed:

- `spank-store::traits` — `BucketWriter`, `BucketReader`, `PartitionManager`.
- `spank-store::SqliteBackend` — opens a directory, creates hot buckets,
  applies the tuned PRAGMAs (`WAL`, `NORMAL` synchronous, `MEMORY` temp store,
  256MB mmap), exposes a writer with `BEGIN IMMEDIATE` on first append and a
  prepared INSERT, and a reader with `count()` and `scan_time_range()`.
- Round-trip test under `tempfile`.

Deferred:

- `spank-store::duck` and `spank-store::pg` modules exist as stubs; their real
  bodies require pulling `duckdb` and `tokio-postgres` into the workspace,
  which is a Cargo.toml change requiring approval per CLAUDE.md §7.
- A migration story. The schema is a single CREATE-IF-NOT-EXISTS; bumping it
  in production needs a strategy that does not exist yet.

Inspection points:

- The `BucketWriter::append` contract is "stage rows; commit makes them
  durable". `commit` calls SQLite COMMIT; verify your call sites do not
  accidentally rely on partial visibility before commit.
- `scan_time_range` uses the `time_event` index; if your queries are
  predominantly on `time_index`, add a second index here.

## Track 8 — TCP receiver

Scope: accept TCP connections, frame lines, attribute syscall errors, and emit
events for downstream consumption.

Landed:

- `spank-tcp::serve(addr, max_line_bytes, tx, lifecycle)` — bind, accept loop
  with bounded backoff (10ms → 1000ms) on accept errors, per-connection
  `tokio::spawn`, gauge increment/decrement on `spank.tcp.connections_current`.
- `spank-tcp::receiver::run_connection` — `set_nodelay` (logged but
  non-fatal), `read_buf` loop with newline framing, line-cap enforcement,
  emits `ConnEvent::Opened`, `Line`, `Closed` with full syscall attribution
  (`spank.tcp.syscall_errors_total{op}`).
- `main::serve` wires an optional consumer that writes per-connection log
  files (one file per `peer-conn_id`).

Deferred:

- TLS termination on the receiver. Out — see network-libraries §2.
- A backpressure path between `run_connection` and the consumer. The
  channel is bounded; on `Full` we currently drop the line (the `_ = tx.try_send(...)`).
  Should be a counted drop with a metric increment, not a silent one.

Inspection points:

- The accept-loop backoff bounds (10ms → 1000ms) are intuition values. A
  real load test should set them. Same for the line cap default.
- The per-connection consumer in `main::spawn_tcp_to_files` opens a file
  per connection without bounding the file count; a SYN-flood-style attack
  on a public bind could exhaust fds. Production deployments should put the
  bind behind an internal subnet.

## Track 9 — Network library review

Scope: produce a single document a Rust network library implementer can read
to understand the stack and where to look hardest.

Landed:

- `docs/Network.md` covering axum, axum-server, tokio, mpsc, raw
  `tokio::net`, flate2, metrics-exporter-prometheus, rusqlite, and what we
  deliberately omitted.
- A "Inspection points" section the reviewer can use as a TODO list.

Deferred:

- An automated dependency audit (`cargo deny`, `cargo audit`) wired to CI. CI
  does not exist yet; when it does, this is one of the first things to add.

Inspection points:

- The four items called out in `docs/Network.md §10` are the
  reviewer's primary targets:
  1. axum-server graceful-shutdown drain semantics under cancellation.
  2. Accept-loop backoff bounds in `spank-tcp::listener::serve`.
  3. TCP receiver buffer reuse strategy across reads.
  4. Shipper reconnect strategy — needs jitter before fleet deployment.

## Build status

`cargo build --workspace` and `cargo test --workspace` are clean. Two notes:

- The two `spank-cfg` tests share the process environment; they were combined
  into a single sequential test (`defaults_and_validation`) so the env-var
  override does not race with the defaults check. If you split them again,
  serialize them with a `Mutex` or `serial_test`.
- The metrics handle's `Debug` impl is hand-written because `PrometheusHandle`
  does not derive `Debug`. This is intentional and should not be reverted to
  `#[derive(Debug)]` without a verification build.

## Requested feedback

Specific decisions to ratify, push back on, or defer:

1. The shipper's no-jitter exponential backoff is acceptable for development
   and a fleet-deployment hazard. Decide whether to add jitter now or wait.
2. The TCP receiver currently drops on `try_send` Full without a metric
   increment. Should be counted; small follow-up.
3. The 501 stubs at `/services/search/jobs`, `/services/data/indexes`, and
   `/services/authentication/users` are placeholders; confirm the order in
   which they should be filled in (search vs. indexes vs. principals).
4. `criterion` and `serial_test` are obvious adds but require a Cargo.toml
   change. Approve or defer.
5. The DuckDB and Postgres backend stubs are not present — should they be
   added as feature-gated stubs now (forces the trait surface to bear weight)
   or land with the real implementation later?
