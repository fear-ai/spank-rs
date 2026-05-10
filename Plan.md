# Plan — Work Tracking

`Focus: plan` — the single tracking document for all in-flight, open, and deferred work. Content that belongs here: work item status, open decisions, deferred gaps with re-opening conditions, phase gate criteria as they accumulate evidence. Content that does not belong here: stable technical contracts (those go in the Reference docs under `docs/`), methodology and standards (those are in `Procesp.md`), architectural design targets (those are in `docs/Sparst.md`).

Work item schema: ID `CODE-L#`, status (`open` / `in-progress` / `done` / `deferred`), owner, target, one-line description. IDs are assigned at intake and do not change when an item moves status. An item with a target that spans more than one crate or file is a signal it needs decomposition. The full labeling rules are in `Appendix A`.

This document is updated in the same commit as any code or documentation change that affects an item here.

---

## Table of Contents

1. [Phase 0 — Spine](#1-phase-0--spine)
2. [Phase 1 — Storage and query](#2-phase-1--storage-and-query)
3. [Phase 2 — Protocol parity](#3-phase-2--protocol-parity)
4. [Phase 3 — Management API and auth](#4-phase-3--management-api-and-auth)
5. [Phase 4 and beyond](#5-phase-4-and-beyond)
6. [Deferred — no active phase](#6-deferred--no-active-phase)

Appendices

- [Appendix A: Work item labeling](#appendix-a-work-item-labeling)

---

## 1. Phase 0 — Spine

Phase 0 covers the foundational implementation pass: tracing, configuration, the API server, HEC receiver, file reader, TCP shipper, storage interface, TCP receiver, error taxonomy, and the observability baseline. The exit criterion for Phase 0 is that CI is red on every current violation of the standards defined in `Procesp.md §8`. The items below are the open and deferred work left from that pass.

Five decisions from `Tracks.md §Requested feedback` are recorded here as open items. Each must be resolved — either accepted, rejected, or explicitly deferred — before Phase 0 closes.

| ID | Status | Owner | Target | Description |
|----|--------|-------|--------|-------------|
| SHIP-JIT1 | deferred | — | `spank-shipper::tcp::TcpSender::run` | Deferred: add jitter to exponential backoff. Re-open before fleet deployment. Current state: pure exponential, 100 ms → 30 s cap. See `docs/Network.md §10`. |
| TCP-BP1 | done | — | `spank-tcp::receiver::run_connection` | Fixed: replaced `.await` send with `try_send`; added `spank.tcp.lines_dropped_total` counter constant; on `Full`, counter incremented and `error_event!` emitted. `docs/Errors.md §6` and `docs/Observability.md §2` updated. |
| API-STUB1 | done | — | `spank-api::router` | Done: `/services/data/indexes` implemented — returns configured index names from `HecConfig::tokens[*].allowed_indexes` with zero counts (live counts deferred to Phase 1 store query). `ApiState` extended with `known_indexes: Arc<Vec<String>>` populated at startup. `/services/search/jobs` and `/services/authentication/users` remain 501 stubs; fill order is search then auth. |
| ENG-DEP1 | done | — | `Cargo.toml` (workspace) | Done: `serial_test = "3.0"` added as workspace dependency; `spank-cfg` declares it as a dev-dependency; `defaults_and_validation` test annotated `#[serial]`. `criterion` deferred to ENG-BENCH1 — re-open when a benchmark regression requires statistical validation. |
| STOR-BACK1 | done | — | `spank-store`, `spank-cfg` | Done: `DuckDb` and `Postgres` variants added to `StoreBackend` in `spank-cfg` and to `Backend` in `spank-store` behind `#[cfg(feature = "duckdb")]` / `#[cfg(feature = "postgres")]`. Feature flags declared in `spank-store/Cargo.toml`. Stub types compile cleanly when flags are absent; real implementations replace the stubs. |
| OBS-DRAIN1 | done | — | `spank-core::Drain`, `spank/src/main.rs` | Done: `Drain::wait` rewritten to use `Notified::enable()` (race-free). Production shutdown path documents task-join equivalence. Per-tag wiring deferred to HEC-ACK1 — tracked separately in the deferred section. |
| ENG-PANIC1 | done | — | `spank/src/main.rs` | Done: `std::panic::set_hook` installed after `install_prometheus()` so the recorder is live before the counter is incremented. Hook increments `spank.process.panics_total` then delegates to the default hook. `metrics` added as a direct dependency of the `spank` crate. |
| CFG-VAL1 | done | — | `spank-cfg::validate` | Done: added validators for `api.bind`, `hec.bind`, `tcp.bind` (SocketAddr parse); `shipper.destinations[*].addr` (SocketAddr parse); `runtime.shutdown_seconds > 0`; `runtime.worker_threads > 0` if set; `hec.tokens[*].value` non-empty; `files.sources[*].workers >= 1`. Tests added for all new validators. |
| HEC-PHASE1 | done | — | `spank/src/main.rs` | Done: `main::serve` now calls `HecPhase::can_transition_to` before `set_phase`; returns an error if the transition is illegal. |
| HEC-HEALTH1 | done | — | `spank-api::router`, `spank-hec::receiver` | Done: `DEGRADED` now returns 200 with `"status":"degraded"` body in both `/health` and `/services/collector/health`. `STARTED` and `STOPPING` return 503. `/health` body adds `"status"` field alongside existing `"phase"` and `"admits_work"`. HEC health response uses `code: 0` for DEGRADED (node functional) and `code: 9` for unavailable states. |
| HEC-DEAD1 | done | — | `spank-core::error`, `docs/Errors.md §1` | Done: `docs/Errors.md §1` updated to document the inbound/outbound split explicitly. `SpankError::Hec` is the outbound surface (forwarder receives a wire error from a downstream HEC endpoint); `RequestOutcome` is the inbound surface (translates ingest outcomes into HTTP responses). No call site change needed until the forwarder (HEC-DES2) is implemented. |
| OBS-HIST1 | done | — | `spank-store::sqlite` | Done: `SqliteBackend::commit` now records `spank.store.insert_duration_seconds` histogram. `docs/Observability.md §2` note updated. |
| CFG-VAL2 | done | — | `spank-cfg` | Done: test for `max_content_length == 0` added to `defaults_and_validation`. |
| HEC-CHAN2 | done | — | `spank-hec::receiver` | Done: Empty or whitespace-only `X-Splunk-Request-Channel` header treated as absent. Added `.map(str::trim).filter(|s| !s.is_empty())` to tag derivation chain before `principal.name` fallback. Prevents empty header from collapsing all no-channel requests into a single `""` routing bucket. Vector `required_channel()` accepts empty without trimming — identified as a latent bug in vendor survey. Documented in `docs/HECst.md §3.3`. |
| SENT-CHKPT1 | done | — | `spank-core::sentinel` | Done: `SentinelKind::Checkpoint` and `Sentinel::checkpoint()` removed. `SentinelKind` is now a single-variant enum (`End`). Module doc updated with re-add note. Re-open when a mid-stream checkpoint use case is concretely designed. |
| CFG-SCHEMA1 | deferred | — | `spank-cfg` | JSON Schema export for editor completion. The TOML render from `render_toml` serves as a runnable spec. Re-open when a third-party editor integration is targeted. |
| OBS-CONSOLE1 | deferred | — | `spank-obs::init_tracing` | `tokio-console` support. Requires `tokio_unstable` build flag; not the default for security reasons. Re-open when async debugging tooling is the focus task. |
| ENG-BENCH1 | deferred | — | `benches/` | `criterion` harness under `benches/`. Directory exists; requires `criterion` as workspace dev-dependency (see ENG-DEP1). Re-open after ENG-DEP1 resolves. |
| ENG-CI1 | deferred | — | CI | Automated dependency audit (`cargo deny`, `cargo audit`) wired to CI. CI does not exist yet. Re-open when CI is established. |
| ENG-CI2 | deferred | — | CI | Continuous baseline tracking — CI emits the benchmark number. Re-open when CI is established. |

---

## 2. Phase 1 — Storage and query

Phase 1 begins after Phase 0 exit. Its scope is the full `spank-store` implementation beyond the SQLite baseline: bucket lifecycle management, time-range queries against real data, and the first SPL commands over the store. Phase 1 exit criterion: `spank search` can execute at least the five most-used SPL commands against a locally-populated SQLite store and return correct results.

No items are currently assigned to Phase 1. Items will be created when Phase 0 closes and the Phase 1 scope is confirmed against `docs/Sparst.md §12` Phase 1 exit criteria.

---

## 3. Phase 2 — Protocol parity

Phase 2 scope is HEC protocol completeness and the management API endpoints deferred in Phase 0: `/services/search/jobs`, `/services/data/indexes`, routing determined by API-STUB1. Phase 2 exit criterion: a Splunk SDK pointed at `spank serve` can submit events and retrieve basic index and search metadata without errors.

No items are currently assigned to Phase 2. Sequencing depends on API-STUB1 resolution.

---

## 4. Phase 3 — Management API and auth

Phase 3 scope is per-route authentication middleware, principals management, and the `/services/authentication/users` endpoint deferred in Phase 0. Phase 3 exit criterion: the four target audiences (SPL learner, CI fixture user, detection engineer, small-scale deployer) can each complete their primary workflow end-to-end against `spank serve` with authentication enforced.

No items are currently assigned to Phase 3.

---

## 5. Phase 4 and beyond

Phase 4 is the first deployment-ready phase: static binary, single-node Shank bundle, installation documentation. Content and exit criteria are defined in `docs/Sparst.md §12`. No work items assigned yet.

Phases 5 and beyond (fleet features, Relay, search cluster) are out of scope until Phase 4 exits.

---

## 6. Deferred — no active phase

Items whose re-opening condition is not tied to a specific phase. Each entry has a stated condition; an item with no stated condition is a deletion candidate.

| ID | Target | Description | Re-open condition |
|----|--------|-------------|-------------------|
| HEC-ORD1 | `spank-hec::receiver` | Channel-based ordering guarantees beyond per-tag flush. A future consumer may need to interleave tags; today each tag is independent. | Re-open when a multi-tag interleaved consumer is designed. |
| HEC-WAL1 | `spank-hec::receiver` | Replay or persistent queueing on the receive side. Currently relies on client retry on 503; embedded WAL is out of scope. | Re-open when SLA requires durable delivery without client retry. |
| HEC-UUID1 | `spank-hec::receiver` | Channel header UUID validation. OTel receiver validates UUID format (returns code 11 for non-UUID values); Splunk Enterprise does not. Current spank-rs: no validation (matches Splunk Enterprise). Re-open if a deployment requires strict channel format enforcement or if the OTel interop path requires it. | Re-open when strict channel validation is required by a specific deployment or interop target. |
| HEC-TIME1 | `spank-hec::processor` | `time` field unit detection. Vector detects seconds/milliseconds/nanoseconds by cutoff comparison (sec < 2400 epoch, ms < year 10000, else ns). Current spank-rs always treats the numeric value as fractional seconds. Clients sending integer millisecond or nanosecond timestamps will get wrong event times. | Re-open when a client sending non-second integer timestamps is targeted, or at Phase 2 protocol parity. |
| HEC-CHAN1 | `spank-hec::receiver` | Channel from query parameter. Splunk spec and Vector/OTel both accept `?channel=UUID` as a fallback when the header is absent. Current spank-rs reads the header only. | Re-open alongside HEC-ACK1 — the ack poll client may use the query param form. |
| HEC-TAG1 | `spank-hec` | Tag derivation: `channel ?? source ?? index ?? "default"`. Some deployments require `sourcetype` to participate in tag keying. | Re-open when a deployment with `sourcetype`-keyed routing is targeted. |
| HEC-META1 | `spank-hec::processor` | Metadata carry-forward across envelopes. Vector and Splunk Enterprise propagate the last-seen `host`, `source`, `sourcetype`, `index` value from one envelope to the next within a single request body. spank-rs deserializes each envelope independently: an absent field falls back to `""` or `"main"`, not the value from the previous envelope. Multi-envelope bodies where only the first envelope sets metadata will have wrong values on subsequent events. Fix: thread a `DefaultMetadata` struct across the `parse_event_body` iterator. See `docs/HECst.md §2.11`. | Re-open at Phase 2 protocol parity or when a shipper producing this pattern is targeted. |
| HEC-AUTH1 | `spank-hec::receiver`, `spank-hec::outcome` | Three auth code gaps: (1) code 3 ("Invalid authorization") for malformed header vs. code 2 for absent header — currently both return code 2; (2) code 4 returns HTTP 401 but Splunk spec assigns HTTP 403; (3) disabled-token code 1 (HTTP 403) requires a disabled-flag in `TokenStore`. Vector returns 401 for code 4 (aligned with spank-rs); spank-py assigns 403. See `docs/HECst.md §1.9`. | Re-open when conformance testing against real Splunk Enterprise verifies the correct HTTP status for code 4, or when token disable is required. |
| HEC-SIZE1 | `spank-hec::receiver`, `spank-hec::outcome` | Body-too-large returns code 6 with a message string. Correct Splunk code is 27 with text "Request entity too large". Requires `RequestOutcome::body_too_large()` constructor and a corresponding handler change. See `docs/HECst.md §1.9`. | Re-open at Phase 2 protocol parity. |
| HEC-CTOK1 | `spank-hec::token_store` | Token lookup uses `HashMap::get` which short-circuits on first differing byte — a timing side channel for token enumeration. At CI/development scale this is not meaningful; at production scale with concurrent requests it is. Fix: constant-time comparison using `subtle::ConstantTimeEq` (already a transitive dependency). See `docs/HECst.md §3.7`. | Re-open before any public-facing or production deployment. |
| FILE-ROT1 | `spank-files::FileMonitor` | Rotation detection every 200 ms misses rapid rotate-twice scenarios. Narrow the poll interval if the log rotator can rotate within 200 ms. | Re-open when a specific log rotator is targeted that can rotate within 200 ms. |
| SHIP-SENT1 | `spank::main::ship` | The bridge translates `FileOutput::Done` to a channel-close rather than forwarding a `Sentinel`. A future consumer that wants to see the sentinel will need to widen the bridge channel type. | Re-open when a downstream consumer that observes sentinels is added. |
| SHIP-TLS1 | `spank-shipper` | TLS for the TCP shipper. Current recommendation: terminate at an LB. | Re-open when a deployment without an LB is targeted. |
| SHIP-ACK1 | `spank-shipper` | Watermarking and persistent ack. The shipper is fire-and-forget; if the receiver acks (it does not today), the watermark goes here. | Re-open when the receiver implements ack and the durability guarantee is required. |
| HEC-ACK1 | `spank-hec::receiver`, `spank-core::Drain` | HEC ACK protocol: `channel` header parsing, `ackId` generation, `POST /services/collector/ack` poll endpoint, and per-tag `Drain::wait` wiring so the ack response is only sent after durable commit. Requires a tag registry keyed on channel/ackId. `Drain::wait` is race-free and ready; no caller exists yet. | Re-open when HEC protocol parity (Phase 2) includes ACK support. |
| TCP-BACKOFF1 | `spank-tcp::listener::serve` | Accept-loop backoff bounds (10 ms → 1000 ms) are intuition values. Set by a real load test. | Re-open at the load-testing phase. |
| STOR-BACK2 | `spank-store::duck`, `spank-store::pg` | DuckDB and Postgres stubs. Conditional on STOR-BACK1 resolution. | Re-open after STOR-BACK1 resolves in favor of early stubs, or when the real implementation is scheduled. |
| STOR-MIG1 | `spank-store::SqliteBackend` | Migration story. Schema is a single `CREATE IF NOT EXISTS`; bumping it in production needs a strategy. | Re-open before the first schema change that must survive an upgrade in production data. |
| STOR-IDX1 | `spank-store::SqliteBackend` | `scan_time_range` uses the `time_event` index. If queries are predominantly on `time_index`, a second index is needed. | Re-open when query patterns from production or benchmarks show `time_index`-dominated access. |
| API-AUTH1 | `spank-api::router` | Per-route auth middleware for management endpoints. The HEC routes carry their own auth; the Splunk-style management endpoints are unauthenticated. The API bind should default to `127.0.0.1` until Phase 3 auth lands. | Phase 3 scope item. |
| TCP-FD1 | `spank-tcp::listener::serve`, `main::spawn_tcp_to_files` | Per-connection log file consumer opens a file per connection without bounding the file count. A SYN-flood on a public bind could exhaust file descriptors. Production deployments must put the bind behind an internal subnet. | Re-open when a production deployment targets a public-facing TCP bind. |
| FILE-WIN1 | `spank-files::monitor` | `inode_of()` returns `Ok(0)` on non-Unix platforms (`#[cfg(not(unix))]`), silently disabling rotation detection in `Tail` mode. The suppression is undocumented and callers see a successful return, not an error. | Re-open when a Windows or other non-Unix build target is added. |
| SHIP-HALF1 | `spank-shipper::tcp::TcpSender::run` | The shipper drops the read half of `into_split()`. A peer that closes its write end without closing the read end (half-closed TCP) does not trigger reconnect until the next write fails. Fire-and-forget deployment is unaffected; reliable-delivery semantics are not. | Re-open when reliable delivery semantics or ACK-based watermarking (SHIP-ACK1) are required. |
| MAIN-SHUT1 | `spank/src/main.rs::serve` | `lifecycle.shutdown()` is called twice: once by the signal handler task and once unconditionally after `api_join` exits. The double call is harmless (cancellation is idempotent) but couples a bind-error or listen-error exit to the graceful-shutdown path, masking the distinction between a clean stop and a startup failure in logs and metrics. | Re-open when the error-handling paths in `serve` are being redesigned or when startup-failure observability is a focus task. |

---

## Appendix A: Work item labeling

This appendix defines the ID scheme for all work items in this document. It adapts the `MODULE-Xn` convention from `spank-py/Product.md §22` to spank-rs usage, preserving the parts that generalize and discarding the parts that were Python-project-specific.

**ID format: `CODE-L#`**

Every work item ID has three parts. `CODE` is the functional domain code — two to five uppercase letters identifying the subsystem or cross-cutting concern. `L` is the topic chain letter — one to five uppercase letters identifying the specific concern or workstream within that code. `#` is a one-based integer giving the step or item number within that chain. Examples: `TCP-DROP1` (TCP domain, DROP chain, item 1), `ENG-ARCH2` (engineering cross-cutting, ARCH chain, item 2), `STOR-BACK1` (storage domain, BACK chain, item 1).

The chain letter encodes the topic's functional meaning, not its arrival order. When a chain has only one item it is still numbered `1`, not left unnumbered. An item that is a prerequisite for another is noted in its description ("Prerequisite for X"); dependencies are not encoded in the ID itself.

**Domain codes in use:**

| Code | Domain | Typical chains |
|------|--------|----------------|
| `API` | REST API surface — router, handlers, health, phase | STUB (501 stubs), AUTH (auth middleware), HEALTH (health endpoint) |
| `ARCH` | Cross-cutting architecture decisions | standalone when the decision does not belong to a single subsystem |
| `CFG` | Configuration loading and validation | VAL (validators), SCHEMA (schema export) |
| `ENG` | Engineering infrastructure — build, CI, tooling, dev-dependencies | DEP (dependencies), PANIC (panic hook), BENCH (benchmarks), CI (continuous integration) |
| `FILE` | File reader and rotation | ROT (rotation detection) |
| `HEC` | HEC receiver, processor, authentication, tag derivation | PHASE (phase transitions), HEALTH (health codes), PARSE (JSON parsing), TAG (tag derivation), WAL (queueing), ORD (ordering) |
| `OBS` | Observability — metrics, tracing, Drain | DRAIN (drain wait), CONSOLE (tokio-console) |
| `SHIP` | TCP shipper — backoff, TLS, ack, sentinel | JIT (jitter), TLS (TLS), ACK (ack/watermark), SENT (sentinel forwarding) |
| `STOR` | Storage backends and schema | BACK (backend stubs), MIG (migrations), IDX (indexes) |
| `TCP` | TCP receiver — drops, backoff, file descriptor limits | DROP (silent drops and counter), BACKOFF (accept-loop backoff), FD (fd exhaustion) |

**Chain letter conventions.** Multi-letter chains (`ARCH`, `DRAIN`, `PARSE`) are preferred when the topic needs a memorable mnemonic — particularly for items that will be referenced frequently across document sections. Single-letter chains (`A`, `B`, `C`) are acceptable for short-lived sequences where the number alone gives enough context, but the preference is to name the chain. When a `CODE` has only one logical workstream, the chain letter may repeat the code abbreviation (`ENG-CI1`, `ENG-CI2`) rather than inventing a separate letter.

**`ARCH` as a standalone code.** In spank-py, `ARCH` appears both as a chain letter within `ENG` (`ENG-ARCH7`) and can stand as a top-level code for decisions that span more than one subsystem. In spank-rs, `ARCH` is reserved as a standalone code for architectural decisions that do not map cleanly to a single domain code. An item that belongs to both HEC and TCP, for example, would carry `ARCH-HT1` rather than being arbitrarily assigned to one domain.

**Status vocabulary.** Four values: `open` — not started; `in-progress` — active work; `done` — complete, recorded as resolved content in the relevant Reference doc; `deferred` — explicitly out of scope with a named re-opening condition. An item with no re-opening condition is a deletion candidate.

**Relationship to spank-py conventions.** The `[OWNER.TYPE]` two-axis tag (`M`, `D`, `MD` owner; `impl`, `test`, `doc`, `res`, `ana` type) from `spank-py/Product.md §22` is not carried forward. At the current project scale — a developer-model pair — ownership and type are captured in the item description rather than as a structured tag. If the project scales to multiple developers or a more formal sprint structure, the owner/type tag is the natural addition. The `Z`-suffix artifact (historical completions before the chain-letter scheme) has no equivalent here; all completed items carry their chain letter with `done` status.
