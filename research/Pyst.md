# Pyst — Python and Rust Considerations for Spank

`Focus: research` — detailed Python-to-Rust comparison covering data models, storage, networking, observability, concurrency, and the resulting gaps and reduced functionality in the current Rust port. The audience is a developer or model session seeking context on what spank-py does that spank-rs does not yet do. Does not receive implementation status (that belongs in `Plan.md`) or subsystem contracts (those belong in `docs/`).

## Scope

This document consolidates Python-language and Rust-language material relevant to Spank: the execution constraints Python imposes on the current implementation, the Rust OSS ecosystem's fit for equivalent work, and the concrete components, crates, and libraries that would appear in a Rust version. Content is organized by topic, not by language — each section compares where comparison is informative.

## Table of Contents

1. Python Execution Constraints
2. Python Production HTTP Stack
3. Rust HTTP Stack for Spank's HEC and API Server
4. Slowloris and Timeout Defense
5. Rust TLS
6. tokio at Scale
7. Rust Regex
8. ANTLR4 Trajectory and Spank's Grammar
9. Parser Library Choices in Rust
10. Rust OSS in Spank's Segment
11. Rust Reverse-Proxy Projects
12. Vector — Concrete Rust Stack Inventory
13. Language-Choice Summary
14. Rust HEC-to-File Implementations
15. File Load into Embedded Database — Rust SQLite and DuckDB
16. Linux Kernel Primitives for Reducing Copies on the Ingest Path

---

## 1. Python Execution Constraints

The Global Interpreter Lock serializes Python bytecode execution: at any instant exactly one thread in a CPython process is interpreting Python bytecode. Threads blocked in a syscall have released the GIL and are not competing for it; threads inside a GIL-releasing C extension (numpy, zlib, hashlib, some regex paths) likewise run concurrently with Python bytecode on other threads. Pure-Python compute across threads does not scale with thread count; the design must justify every thread as either blocking-I/O or C-extension offload.

CPython's garbage collector is refcount plus a generational cycle collector. Refcount reclaims most objects at the moment their last reference drops; the generational collector runs periodically to find cycles. Patterns that cooperate: short-lived allocations, flat data, no cycles between long-lived objects. Patterns that fight it: callback closures capturing `self` on `self`'s children (cycle), large per-event dict/list graphs in hot loops (growing generational populations), caches that retain objects past their natural lifetime. Collection cycles hold the GIL for their duration; all Python threads pause together.

Memory per record is high. A flat 100-byte log line becomes kilobytes of Python object graph during decode — each string, dict, and list has a header and is heap-allocated. The cost is amortized only if decoded objects are short-lived; a queue of decoded records scales memory linearly with queue depth and with the expansion factor.

Regex GIL behavior is pattern- and input-dependent and not documented as a public contract. CPython 3.11+ releases the GIL around the inner match loop for compiled patterns on large inputs; earlier versions hold it throughout. The safe assumption for design is "regex holds the GIL." The third-party `regex` package has distinct GIL behavior and is sometimes adopted for that reason alone.

Python stdlib regex source: `Modules/_sre/sre.c` and supporting files in CPython; wrapper at `Lib/re/__init__.py`, parsing and compilation at `Lib/re/_parser.py` and `Lib/re/_compiler.py`. Third-party `regex` package source at `github.com/mrabarnett/mrab-regex`.

## 2. Python Production HTTP Stack

Python's standard library contains HTTP pieces but not a production HTTP server. `http.server` is documented in its own docs as not recommended for production; `http.client` is low-level; `asyncio` is a runtime with no HTTP story of its own; `ssl` is OpenSSL bindings. A production Python service assembles a stack from third-party packages:

- HTTP server: gunicorn (WSGI, pre-fork), uvicorn (ASGI on `uvloop`+`httptools`), or hypercorn (HTTP/2).
- Framework: Flask (WSGI), FastAPI or Starlette (ASGI), Django.
- Middleware: Starlette middleware, Flask extensions, or Django middleware — installed per concern (rate limiting, CORS, compression, auth, tracing).
- Async runtime: asyncio (stdlib), typically with uvloop drop-in for performance.
- TLS: usually terminated at a reverse proxy in front of Python; `ssl` stdlib is available but slow.
- HTTP client: requests (sync) or httpx (async).
- JSON: `json` stdlib is slow; production services use orjson or ujson on hot paths.

A FastAPI service with production concerns uses seven to nine components. The apparent simplicity comes from framework-integrated defaults — `pip install fastapi` pulls a pre-wired stack — not from a smaller stack.

## 3. Rust HTTP Stack for Spank's HEC and API Server

Recommended stack for both HEC and the management/query API, one line: **axum 0.8 + hyper 1.x + rustls 0.23 (aws-lc-rs provider) + tower-http + axum-server, on tokio**.

**axum 0.8** (current 0.8.9, 0.8.0 released 2025-01-01). Tokio-team framework. Composes middleware as `tower::Service`, which gives direct access to tower-http (timeouts, concurrency limits, compression, tracing, CORS, request-size limits) as layered components. Thin: a few thousand lines; delegates protocol work to hyper, middleware to tower, TLS to rustls.

**actix-web 4.10.x.** Alternative stack that does not use hyper or tower. Own HTTP implementation (`actix-http`), own middleware. Faster on tight micro-benchmarks by 10–15 percent on saturated GET; gap narrows or disappears on TLS + JSON + I/O workloads. Loses tower ecosystem.

**warp.** Last real release late 2023, no hyper 1.0 migration landed. Legacy; avoid for new work.

**hyper direct.** Viable for the HEC endpoint's two routes with a strict protocol; over-budget for the management API's many routes.

**hyper 1.0.** Shipped 2023-11-15. Three-year no-breaking-change guarantee through late 2026. axum 0.7+ and tonic 0.11+ on hyper 1.x; reqwest migrated in 0.12. Companion `hyper-util` crate still 0.1 and occasionally ships minor breaks — pin explicitly.

**Applied to HEC specifically:**

- HTTP/1.1 and HTTP/2 both supported; ALPN auto-negotiates. Must set `ServerConfig::alpn_protocols = [b"h2", b"http/1.1"]` explicitly; axum-server does not default-set it.
- Per-connection footprint at 10k HTTPS+HTTP/2 connections: roughly 20–60 KB (rustls buffers ~16 KB, tokio task ~2 KB, HTTP/2 frame state, handler state). 10k concurrent connections ≈ 500 MB heap.
- Multi-MB JSON batches via `Body::collect()` with a timeout, or `Body::into_data_stream()` for incremental parsing. Set `tower_http::limit::RequestBodyLimitLayer` explicitly; default is unbounded.
- `Authorization: Splunk <token>` as a `FromRequestParts` extractor or tower layer.
- Response codes handled by axum's `IntoResponse`.
- Ack tracking: `Arc<AckStore>` injected via `axum::extract::State`.
- Backpressure: custom middleware reads a queue-depth gauge and rejects with 429 or 503.
- Graceful shutdown: `axum_server::Handle::graceful_shutdown(Some(Duration))` stops accepting, signals in-flight, hard-closes after timeout. Plain `axum::serve().with_graceful_shutdown()` has no timeout cap — always wrap.

**Applied to the management API:**

- `Body::from_stream(impl Stream<Item = Result<Bytes, E>>)` for streamed responses. hyper serializes to chunked transfer encoding (HTTP/1.1) or DATA frames (HTTP/2). HTTP/2 backpressure works end-to-end because hyper honors the peer window and only polls the stream when the window allows — producer stalls naturally under a slow reader, provided the producer is `poll_next`-driven rather than a background task pushing into an unbounded channel.
- For NDJSON streams: `axum-streams` crate or hand-rolled `futures::stream::unfold`.

## 4. Slowloris and Timeout Defense

Not a solved single-middleware problem in axum or actix. Three timeouts live at different protocol phases and must be composed:

1. **Header-read timeout** — `http1::Builder::header_read_timeout(Duration::from_secs(10))` on the hyper connection builder. Defends against headers never arriving.
2. **Body-read timeout** — wrap `Body::collect()` in `tokio::time::timeout`. Defends against slow-body attacks.
3. **Total-request timeout** — `tower_http::timeout::TimeoutLayer` around the router. Backstop.
4. **Idle / keep-alive timeout** — `http1::Builder::keep_alive(true)` with explicit idle duration. Defends against completed-request-then-idle-hold.

Per-IP limits via `tower-governor`. The naïve `tower::limit::RateLimitLayer` clones per request and does not share state — documented footgun. Connection-count caps via `tower::limit::ConcurrencyLimitLayer`.

None of axum, actix, Nginx, or Go `net/http` default to protective values across all phases. Nginx defaults to 60 seconds on `client_header_timeout` and `client_body_timeout`. Go added `Server.ReadHeaderTimeout` after the issue surfaced; default is still zero. actix-web has `HttpServer::client_request_timeout` with a 5-second default — unusual in having non-zero defaults.

**Load behavior with the three timeouts set to reasonable values (header 10s, body 30–60s, idle 30s) plus a concurrency cap sized to memory and descriptor budget:**

- 10k attacker connections: absorbed. Memory ≈ 300 MB, descriptors 10k. Legitimate traffic unaffected because attacker connections consume buffer but not CPU.
- 100k connections: memory ≈ 3 GB, descriptor count hits kernel limits. Survivable on a large server, not a laptop. Per-IP rate limiting caps before reaching this scale.
- Inflated `Content-Length` with trickle body: body timeout closes connection within the budget; `RequestBodyLimitLayer` caps allocation. 100 MB `Content-Length` with 1 byte/sec dies at 60 seconds after ~60 bytes received.
- Legitimate slow clients (mobile, distant geography): harder to distinguish from attack. Tuning is workload-dependent; no good universal default.

**Production mitigation is three layers:**

1. Proxy in front (Nginx, Caddy, Pingora) for TLS termination and slow-connection absorption. Nginx's event-loop architecture makes it slowloris-resistant; the per-slow-connection cost is kernel buffer plus a small amount of Nginx state, not a thread.
2. Server-side timeouts as defense-in-depth in case the proxy is bypassed.
3. Monitoring slow-connection closure counts; alert on anomaly.

Spank's deployment model assumes a proxy in front for public-facing HEC and documents the three timeouts as a deployment requirement. A CI test with `slowhttptest` catches regressions.

## 5. Rust TLS

**rustls 0.23.x** (current 0.23.38). Production-mature. Default crypto provider switched to `aws-lc-rs` in 0.23 (`ring` still supported). aws-lc-rs is FIPS-capable, faster on most workloads, and adds post-quantum KEMs. TLS 1.2 and 1.3 both supported; SNI default-on; mutual TLS first-class via `ClientCertVerifier` trait; certificate rotation via `ResolvesServerCert` trait (idiomatic hot-reload — swap the resolver without dropping existing connections). No known issues at thousands-of-connections scale.

**Framework integration.** axum → `axum-server` crate with `tls_rustls` feature; `RustlsConfig::from_pem_file` or `from_config` for rotation. `rustls-acme` for ACME provisioning. actix-web → `HttpServer::bind_rustls_0_23` (less flexible). hyper direct → `tokio-rustls::TlsAcceptor` wrapping the TCP listener.

**native-tls (OpenSSL)** remains an option for FIPS-certified OpenSSL builds or OpenSSL PKI interop. Larger footprint, slower handshakes, platform-TLS differences on macOS and Windows. Skip unless compliance requires it.

Recommendation for Spank: rustls 0.23 with aws-lc-rs.

## 6. tokio at Scale

For thousands of concurrent HTTPS connections on tokio (1.40+):

- `tokio::runtime::Builder::new_multi_thread()` with worker count pinned to physical cores, not logical. TLS handshakes are CPU-bound and hyperthreading hurts under handshake storms.
- `TcpSocket::set_nodelay(true)` to avoid Nagle stalls on small acks.
- `hyper::server::conn::http2::Builder` tunables: `max_concurrent_streams`, `initial_stream_window_size`, `initial_connection_window_size`. Defaults are conservative and throttle large-batch POSTs.
- `ulimit -n 65536` minimum for 10k connections.
- `SO_REUSEPORT` via `socket2` for multiple accept loops on multiple cores if needed.

Scheduling-latency pathology at 10k connections: none on recent tokio. Known pain point: long synchronous work inside async tasks starving the scheduler. Mitigation: `tokio::task::spawn_blocking` or `rayon` for heavy JSON parsing on large batches.

## 7. Rust Regex

The `regex` crate (Andrew Gallant, `github.com/rust-lang/regex`, part of `rust-lang` org) uses a hybrid of finite-automata techniques (NFA simulation, lazy DFA, one-pass NFA) chosen per pattern. Guaranteed linear time in input length; no backtracking, no catastrophic ReDoS, because the crate does not expose features (backreferences, lookaround) that would require backtracking. For log parsing this is the correct tradeoff — log patterns do not need backreferences, and linear-time guarantee under adversarial input is a security property.

Throughput an order of magnitude or more above Python's `re` on typical log patterns — not because of instruction speed but because the matcher algorithm is better. Also used by ripgrep, benchmarked against by Hyperscan.

Companion crates: `regex-syntax` for pattern parsing, `regex-automata` for direct automata access, `aho-corasick` for literal multi-pattern search (often faster than regex for "any of these fixed strings").

No GIL interaction. A regex match on one thread does not block any other thread.

## 8. ANTLR4 Trajectory and Spank's Grammar

**Release timeline (selected):**

| Version | Date | Addition |
|---|---|---|
| 4.0 | Jan 2013 | Initial, Java-only |
| 4.5 | 2015-01-23 | C#, Python2, Python3, JavaScript unified |
| 4.6 | 2016-12-15 | C++, Swift, Go added |
| 4.7 | 2017-03-30 | Full 21-bit Unicode, `CharStreams` |
| 4.8 | 2020-01-20 | PHP added |
| 4.9 | 2020-11-24 | Dart added |
| 4.10 | 2022-04-11 | ATN serialization redesigned (breaking), `caseInsensitive` lexer option |
| 4.11.0 | 2022-09-04 | Go target rewritten; cross-target constant-folding |
| 4.12.0 | 2023-02-20 | TypeScript added |
| 4.13.0 | 2023-05-21 | Go runtime split to `antlr4-go/antlr` repo |
| 4.13.2 | 2024-08-03 | Latest release, maintenance only |

No 4.14 has shipped. The 20-month gap since 4.13.2 is the longest since 4.7.2 → 4.8. Terence Parr still owns the repo but is absent from the 2025–2026 commit log. Eric Vergnaud and target-specific contributors handle merges. Project is in late-maturity maintenance.

**Rust target — `rrevenantt/antlr4rust`.** Repo created 2019-09-03; last master commit 2022-10-22; last release `antlr4-4.8-2-Rust0.3.0-beta` on 2022-07-22. Pinned to a patched **ANTLR4 4.8 fork**. Not ported to 4.10's new ATN serialization, 4.11, 4.12, or 4.13. 37 open issues, not archived. Effectively dormant. No signal that `antlr/antlr4` plans to accept Rust as an official target. No viable alternative fork has emerged. For a long-lived production server this is a material runtime-maintenance risk.

**Spank's grammar — `src/spank/search/grammar/FilterQuery.g4`, 229 lines.** Feature footprint:

- Core parser/lexer syntax: alternation, `?`, `*`, `+`, grouping.
- Character classes including inversions (`~["\\]`, `~[ \t\r\n=()'"<>!,[\]]`).
- Fragment rules (`fragment SIGN`, `fragment DIGIT`, `fragment SEGMENT`, etc.).
- Case-insensitive keywords via bracket alternation (`[Ll][Ii][Kk][Ee]`). ANTLR 4.10's `caseInsensitive` lexer option would simplify ~20 lines but is not adopted.
- Manual precedence hierarchy (`orExpression → andExpression → unaryExpression → primary`) rather than ANTLR's direct-left-recursion sugar.
- `-> skip` lexer command.
- `EOF` in the top-level rule.

**Not used:** semantic predicates, lexer modes, embedded actions, grammar imports, AST rewrites, channel redirection beyond `skip`.

Grammar exercises only stable ANTLR4 core; no feature from 4.10+ is required. Port-risk as a grammar is minimal. Risk is in the runtime.

**Port options if moving off Python:**

- Mainstream ANTLR targets (C#, C++, JavaScript, TypeScript, Python3, Go, Java) — grammar compiles unchanged, runtimes are production-grade.
- Rust via `antlr4rust` — grammar compiles (4.8 covers it) but runtime is dormant. Not recommended.
- Hand-port to a native Rust parser library — LALRPOP, nom, lalrpop, or a hand-written Pratt parser. Bounded effort (229 lines, standard constructs, no semantic predicates or modes).

## 9. Parser Library Choices in Rust

Four tiers, each best at a different problem shape.

**LALRPOP** — LR(1) parser generator in the yacc/bison lineage. Grammar in a dedicated `.lalrpop` file with EBNF-like syntax and Rust code actions; generates a bottom-up parser at compile time. Good for designed languages with evolving grammars, significant recursive structure, and expression-precedence requirements. Compile-time ambiguity check. Tradeoffs: error messages are poor without effort, grammar file is a separate mini-language. Vector's VRL uses LALRPOP.

**nom** — parser-combinator library. Parsers are Rust functions that compose; no separate grammar file, no codegen. Good for byte-oriented position-sensitive formats (protocol framing, binary formats, line-oriented text), zero-copy parsing on `&[u8]` slices, formats that do not benefit from a formal grammar. Error types carry position and expected-token data naturally. Vector uses nom for DNS, syslog, various framing decoders. Current 8.x; 7.x transitively present.

**regex** — the `regex` crate. For pattern extraction from loosely structured text, classification, filtering. Linear-time guarantee, no ReDoS. Poor for deeply nested grammars or structured output beyond capture groups.

**Hand-written** — straight Rust code. Good for simple stable formats where performance and readability argue against a framework. Vector's `prometheus-parser` is line-oriented hand-written code for the exposition format. Loki's protocol wrapping is hand-adapted around prost-generated Protocol Buffers code.

Using the wrong tier costs readability or performance or both: regex for a nested expression language is unmaintainable; LALRPOP for line-oriented syslog is overkill; nom for two fields out of an HTTP header is verbose where one regex line suffices.

For Spank's FilterQuery grammar specifically, LALRPOP is the closest analog to ANTLR in authoring style. Porting effort: a few days for someone fluent in LALRPOP.

## 10. Rust OSS in Spank's Segment

**Vector (vectordotdev/vector, Datadog).** Broadest feature set in the observability-pipeline segment — many sources and sinks, VRL transform language, on-disk buffering, Kubernetes-native. Dominant Rust entry with no peer Rust project of comparable scope. MPL-2.0. Detailed inventory in §12.

**Quickwit (quickwit-oss/quickwit).** Distributed search engine for logs and traces; index files on object storage (S3, GCS). Built on Tantivy (Rust Lucene-equivalent, same team). Direct competitor to Elasticsearch for log search. Acquired by Datadog in 2024; open-source continuation ongoing.

**GreptimeDB (GreptimeTeam/greptimedb).** Time-series database in Rust with log-ingestion support. Backend rather than pipeline; Vector ships a sink for it.

**Parseable (parseablehq/parseable).** Log storage and query in Rust; Parquet-on-object-storage. Newer, narrower scope than Quickwit.

**Tantivy.** Rust Lucene-equivalent underpinning Quickwit. Not a server; a search library.

No single Rust OSS covers the full Splunk-replacement scope (ingest + storage + search + UI). The Rust approach in the wild is composition — Vector for ingest plus Quickwit or a time-series database plus a UI layer — not monolith.

## 11. Rust Reverse-Proxy Projects

**Pingora (cloudflare/pingora).** Cloudflare's Rust proxy framework, open-sourced early 2024. A library for building proxies, not a drop-in Nginx replacement; Cloudflare uses it internally to serve a large fraction of edge traffic (publicly cited "more than 40 million requests per second"). Async on tokio. Apache-2.0.

**River (memorysafety/river).** Nginx/HAProxy replacement built on Pingora. Sponsored by the Internet Security Research Group (Let's Encrypt) via their Prossimo memory-safety initiative. Explicitly positioned as "production reverse proxy in Rust." Pre-1.0 as of 2026 but actively developed with institutional backing.

**Sozu (sozu-proxy/sozu).** Mature Rust HTTP reverse proxy, predates Pingora. Hot-reload configuration, multi-worker architecture for zero-downtime reloads. MPL-2.0.

**static-web-server.** Small, focused static-file HTTP server. Not a proxy — just the "Nginx serving static content" role.

**Institutional context — Prossimo.** ISRG has funded Rust rewrites of foundational Internet infrastructure: `rustls` (TLS), `hickory-dns` (formerly Trust-DNS), and River (HTTP proxy). Explicit motivation is memory-safety in the Internet's critical path.

For Spank deployment, a fronting proxy (Nginx, Caddy, Pingora, River) is independent of Spank's implementation language — it speaks HTTP to Spank regardless of what Spank is written in.

## 12. Vector — Concrete Rust Stack Inventory

Vector 0.55.0, Rust toolchain 1.92, edition 2024. Workspace has 31 internal crates under `lib/` plus the main `vector` crate and `vdev`. `Cargo.lock` contains **1,233 packages** total (full transitive graph for all-features build). `Cargo.toml` is 1,175 lines.

**Async runtime.** `tokio 1.49.0` with `full`. `tokio-stream`, `tokio-util`, `tokio-openssl`, `tokio-tungstenite` (WebSockets), `tokio-postgres`. Optional `console-subscriber` for tokio-console debugging. Plumbing: `async-stream`, `async-trait`, `futures 0.3.31`, `futures-util`, `pin-project`.

**HTTP stack (mixed, mid-migration).** `hyper 0.14.32` with `client, server, http1, http2, stream` — not 1.x. `axum 0.6.20` (older than current 0.8). `warp 0.3.7` retained for earlier code paths. `reqwest 0.11` and a parallel `reqwest_12 = { package = "reqwest", version = "0.12" }` — two reqwest versions in the same binary during migration. `h2 0.4.11` for direct HTTP/2 use. `http 0.2.9` and `http-1` (`http 1.0`) side-by-side. `hyper-openssl`, `hyper-proxy`. `tonic 0.11` for gRPC with `tls, tls-roots`; `tonic-health`. `async-graphql 7.0.17` and `async-graphql-warp` — Vector's management API is GraphQL over warp.

**Middleware.** `tower 0.5.2` with `buffer, limit, retry, timeout, util, balance, discover`. `tower-http 0.4.4` with `compression-full, decompression-full, trace`. `tracing-tower` from a git rev on `tokio-rs/tracing`.

**TLS.** OpenSSL, not rustls, as the primary. `openssl 0.10.73` with `vendored` (builds own OpenSSL into the binary), `tokio-openssl`, `hyper-openssl`, `postgres-openssl`, `openssl-probe`. rustls appears transitively for integrations that insist on it (`rumqttc` with `use-rustls`, `sqlx` with `tls-rustls-ring`, `mongodb` with `rustls-tls`, `databend-client` with `rustls`). Vector chose OpenSSL for enterprise-PKI compatibility.

**Serialization.** `serde 1.0.219`, `serde_json 1.0.143` with `preserve_order, raw_value`, `serde_yaml 0.9.34`, `serde_with`, `serde_bytes`, `serde-toml-merge`, `toml 0.9.8`. Binary: `rmp-serde` and `rmpv` (MessagePack), `prost 0.12` plus `prost-reflect` plus `prost-types` (Protocol Buffers), `apache-avro 0.16.0`, `arrow 56.2.0` plus `arrow-schema` (Apache Arrow IPC), `csv 1.3`.

**Regex and parsing.** `regex 1.12.3` with `std, perf`. `nom 8.0.0` direct, `nom 7.1.3` transitively. `pest` transitively. `lalrpop 0.22.0` plus `lalrpop-util` — VRL's parser generator. `hickory-proto 0.25.2` for DNS. Internal: `dnsmsg-parser`, `dnstap-parser`, `prometheus-parser`, `loki-logproto`.

**Storage and buffering.** No rocksdb, no sled, no lmdb. `lib/vector-buffers` uses `rkyv` (zero-copy deserialization) for on-disk record format. Database sinks (not internal storage): `tokio-postgres 0.7.13`, `postgres-openssl`, `sqlx 0.8.6` (Postgres backend), `mongodb 3.3.0`, `redis 0.32.4`, `databend-client`, `async-nats`.

**Compression.** `flate2 1.1.2` with `zlib-rs` backend (pure-Rust rewrite, faster than zlib-ng in many cases). `zstd 0.13.0`, `snap 1.1.1`, `async-compression 0.4.27` with `gzip, zstd`. `lz4` via `pulsar` features.

**Cloud SDKs.** AWS uses official `aws-sdk-*` crates (Smithy-generated), not rusoto. Twenty AWS-related crates: `aws-runtime`, `aws-config`, `aws-credential-types`, `aws-sdk-{cloudwatch, cloudwatchlogs, elasticsearch, firehose, kinesis, kms, s3, secretsmanager, sns, sqs, sts}`, `aws-sigv4`, `aws-smithy-{async, http, runtime, runtime-api, types}`, `aws-types`. Each service is a separate crate; feature-gating reduces build time and binary size. Azure: `azure_core 0.30`, `azure_identity`, `azure_storage_blob`. GCP: `goauth`, `smpl_jwt`. Generic object storage: `opendal 0.54`.

**Message brokers.** `rdkafka 0.39.0` (wrapper around C `librdkafka`; statically links `curl-static`, `libz`). `lapin 4.3.0` (AMQP/RabbitMQ). `pulsar 6.7.0` (Apache Pulsar). `async-nats 0.42.0`. `rumqttc 0.24.0` (MQTT). `redis` for Redis streams.

**Kubernetes and containers.** `kube 3.0.1` plus `k8s-openapi 0.27.0` pinned to `v1_31`. `bollard 0.19.1` for Docker.

**Metrics, tracing.** `metrics 0.24.2`, `metrics-tracing-context 0.17.0`, `metrics-util 0.18.0`. `tracing 0.1.44`, `tracing-core`, `tracing-futures`, `tracing-subscriber 0.3.22` with `ansi, env-filter, fmt, json, registry, tracing-log`. Internal `tracing-limit`.

**Configuration.** TOML (`toml 0.9.8`), YAML (`serde_yaml`), JSON (`serde_json`). Internal `vector-config`, `vector-config-common`, `vector-config-macros` implement schema and validation, plus config-schema generation for the docs site.

**Memory allocator.** `tikv-jemallocator 0.6.0` replaces the system allocator with jemalloc on supported platforms. Meaningful for a log pipeline's small-buffer allocation pattern.

**File and I/O.** `notify 8.1.0` with `macos_fsevent` for filesystem events. `listenfd` for systemd socket activation. `glob`, `socket2`, `stream-cancel`, `tokio-util` with `io, time`.

**Utility crates of note.** `arc-swap` (atomic `Arc` swapping for config reload), `dashmap` (concurrent hashmap), `bytes` (reference-counted buffers), `indexmap` (insertion-ordered maps), `smallvec`, `bloomy` (Bloom filter), `bytesize`, `url`, `uuid`, `chrono`, `chrono-tz`, `governor 0.10.0` (rate limiting), `lru 0.16.3`, `seahash`, `hashbrown 0.14.5`, `evmap` (eventually-consistent concurrent map), `thread_local`, `deadpool 0.12.2` (connection pool), `maxminddb 0.27.0` (GeoIP), `hostname`, `humantime`, `percent-encoding`, `encoding_rs`, `roaring` (compressed bitmaps).

**Internal workspace crates — 31 under `lib/`.** Structural: `vector-lib` (top-level aggregator), `vector-core` (event types, schemas, traits), `vector-common`, `vector-common-macros`, `vector-config`, `vector-config-common`, `vector-config-macros`, `vector-buffers`, `vector-stream`, `vector-lookup`, `vector-api-client`, `vector-tap`, `vector-top`. Parsing: `codecs`, `file-source`, `file-source-common`, `prometheus-parser`, `loki-logproto`, `opentelemetry-proto`, `dnsmsg-parser`. VRL family: `vector-vrl/{functions, metrics, category, cli, dnstap-parser, enrichment, tests, web-playground}`. Dev: `k8s-e2e-tests`, `k8s-test-framework`, `fakedata`, `docs-renderer`, `tracing-limit`, `vdev`.

**VRL itself is external.** `vrl = { git = "https://github.com/vectordotdev/vrl.git", branch = "main" }`. Datadog split VRL out of the Vector repo in 2023 so other consumers (Data Prepper, etc.) can reuse it.

**Notable observations:**

- 1,233 packages because Vector integrates with everything. Trimmed builds with fewer features pull a fraction.
- OpenSSL deliberately over rustls for enterprise-PKI compatibility. Spank has no such legacy; rustls is the cleaner choice.
- HTTP stack is mid-migration — hyper 0.14 + axum 0.6 + warp 0.3 + reqwest 0.11 and 0.12 simultaneously. Vector is conservative on major bumps because sink integrations lag.
- Parser choice varies by format: LALRPOP for VRL, nom for framing and syslog, regex for general matching, hand-written for Prometheus and Loki. Not inconsistency; right tool per tier.
- 31 internal crates — Vector is a collection of libraries glued together by the main crate, not a single program.
- jemalloc and rkyv are explicit allocation-cost choices.

For Spank's HEC endpoint specifically, Vector's `sources/splunk_hec.rs` is the canonical prior-art reference — on warp + hyper 0.14 in 0.55.0, not the stack a new project would choose in 2026, but protocol handling and ack-tracking logic is directly relevant.

**Subset Spank would plausibly start with for analogous core work:** tokio, hyper + axum or hyper direct, rustls, serde + serde_json, regex, nom for specialized parsers, tracing, metrics, rdkafka if Kafka is in scope, AWS SDK crates if S3 is in scope. Twelve to fifteen direct dependencies for a feature-comparable core, plus integration-specific per source and sink. Much smaller than Vector's 1,233.

## 13. Language-Choice Summary

The architecture in Spanker.md is language-neutral — execution decomposition, storage generic-with-specialization, shared-data discipline, pressure adaptation, ordering invariants apply to any language.

Python advantages for Spank: development velocity, large ecosystem for log parsing and database drivers, excellent debugging, natural fit for CLI and small management surfaces. Sufficient for laptop and small-VPS tier on a well-written implementation.

Python disadvantages: the GIL forecloses pure-Python compute parallelism; memory per record is high; GC behavior under sustained high-rate ingestion is nontrivial; stdlib HTTP is not production-grade; the real stack (gunicorn/uvicorn + FastAPI/Starlette + uvloop + ssl + orjson) is not smaller than Rust's, just framework-integrated.

Rust advantages: no GIL; linear-time regex with no ReDoS; memory-safe production HTTP via axum + hyper + rustls with published deployments at comparable or larger scale than Spank targets; deterministic destruction; per-record memory an order of magnitude lower; explicit layering makes each component substitutable.

Rust disadvantages: steeper learning curve; explicit layer composition visible at development time; ANTLR4 Rust target is dormant (hand-port the grammar or accept the fork's stale-runtime risk); integration ecosystem for less common log-source types is thinner than Python's.

The decision is an engineering tradeoff, not a foundational one. Architecture survives the choice.

## 14. Rust HEC-to-File Implementations

Scope: Rust projects that accept Splunk HEC on the wire and terminate events on a local filesystem (NDJSON, gzipped NDJSON, Parquet, rotated log files). This is the shape of a minimal Rust replacement for a Splunk Universal Forwarder inverted — HEC in, files on disk.

### 14.1 Vector — the only mature match

Vector (`vectordotdev/vector`, MPL-2.0) is the single production-grade Rust project that delivers this topology in-tree. HEC source at `src/sources/splunk_hec/mod.rs` registers `POST /services/collector/event[/1.0]`, `POST /services/collector/raw[/1.0]`, `GET|POST /services/collector/health[/1.0]`, `POST /services/collector/ack`. Auth via `Authorization: Splunk <token>` or `?token=`, gzip request bodies supported, `valid_tokens` list in `SplunkConfig`, full indexer-ack protocol with per-channel ack IDs (`acknowledgements.rs`, `IndexerAcknowledgement` struct). A Splunk UF, `fluent-plugin-splunk-hec`, the Splunk SDK, Cribl Stream, the OTel `splunkhecexporter`, and curl examples from the Splunk documentation all work against it unchanged.

File sink at `src/sinks/file/mod.rs`. `path` is a Strftime/template expression; one open handle per rendered path; `idle_timeout_secs` closes idle files. `encoding` supports `json` (NDJSON), `text`, `native_json`, `csv`, `logfmt`, `gelf`, `avro`. `compression` supports `none | gzip | zstd` via `async-compression`. Parquet is not in the file sink; it exists only in the object-store sinks (S3, GCS, Azure Blob) — to land Parquet on a local filesystem, point `aws_s3` at a MinIO instance or post-process NDJSON via DataFusion or Arrow.

Minimal configuration for "HEC in, gzipped NDJSON on disk, partitioned by date and index":

```yaml
sources:
  hec:
    type: splunk_hec
    address: 0.0.0.0:8088
    valid_tokens: ["..."]
    acknowledgements: { enabled: true }
sinks:
  disk:
    type: file
    inputs: ["hec"]
    path: "/data/hec/%Y-%m-%d/{{ .splunk.index }}/{{ .host }}.ndjson.gz"
    encoding: { codec: json }
    compression: gzip
```

Durability caveat: the file sink calls `sync_all` only at rotation or close; HEC indexer-ack confirms the event reached Vector's internal buffer, not that bytes are durable on disk. The standard remedy is `buffer: { type: disk, max_size: ... }` on the sink, which persists the batch to a local Vector-managed queue before acking upstream. Metadata mapping is preserved under `event.splunk_*` by default; `lookup_v2` paths are configurable and easy to misconfigure. Multi-event JSON arrays on `/event` are accepted, line-delimited JSON is also accepted, and `Content-Type` is largely ignored — matching real HEC.

### 14.2 Adjacent projects without HEC compatibility

- **Quickwit** (`quickwit-oss/quickwit`, AGPL-3.0): ingest APIs are Quickwit-native, Elasticsearch `_bulk`, and OTLP/HTTP. No `/services/collector` route anywhere in the tree. Output is Quickwit splits (tantivy index files) on object store, not replayable NDJSON.
- **Parseable** (`parseablehq/parseable`, AGPL-3.0): Parseable-native JSON POST to `/api/v1/ingest` with `X-P-Stream` header, plus OTel/HTTP. No HEC shim. Output is Parquet partitioned by stream and time with an Arrow staging area — the on-disk layout is Parseable's, not raw events.
- **GreptimeDB**: Loki-compatible push, OTel, and a native logs pipeline. No HEC receiver.

### 14.3 Small Rust projects

`yaleman/splunk-rs` is a client library (`HecClient`, `send_events`), no server side. GitHub's `splunk-hec` topic is dominated by Go, Java, and Ruby. The closest non-Rust analogue is the OTel Collector `splunkhecreceiver` paired with its `file` exporter — Go, equivalent shape.

### 14.4 Takeaway for Spank

For a Rust implementation with the "HEC in, files on disk" contract, Vector's source-and-sink pair is the reference to read: `src/sources/splunk_hec/mod.rs` for wire handling and ack semantics, `src/sinks/file/mod.rs` for templated-path rotation and encoding. Both predate hyper 1.x; a new project would rebuild them on axum 0.8 + hyper 1.x + rustls 0.23 per section 3 of this document. No Rust OSS competitor terminates HEC on a local filesystem; the clear field means either adopt Vector as-is or design from the Vector reference.

## 15. File Load into Embedded Database — Rust SQLite and DuckDB

Scope: Rust libraries, tuning PRAGMAs, and published benchmark results for loading files (NDJSON, CSV, Parquet, gzipped variants, plain text) into SQLite or DuckDB. This is the steady-state ingest shape for a Spank bulk-import command and the background-flush shape for the live pipeline.

### 15.1 Rust SQLite crates

**rusqlite** (`rusqlite/rusqlite`, 0.31). Thin FFI over `sqlite3.c`. The `bundled` feature compiles SQLite from the amalgamation, pinning version and enabling `SQLITE_ENABLE_*` defines (column metadata, FTS5, JSON1, RTREE). The `system` build links `libsqlite3` and inherits whatever the OS ships, which on stale macOS images can be missing JSON1. For bulk load, `bundled` with `-C opt-level=3` is consistently 5–15 percent faster than `system` per rusqlite issue #1089 reports. `unlock_notify` matters only under multi-process contention and is irrelevant for single-writer ingest. `extra_check` enables `sqlite3_extended_errcode` at a small per-call cost and should be disabled for bulk. The incremental `blob` API is irrelevant for log ingest unless storing opaque payloads.

**sqlx** (`launchbadge/sqlx`). Async, compile-time-checked. The SQLite backend ultimately calls `libsqlite3-sys`. Async buys nothing for bulk insert because SQLite serializes writes; tokio task scheduling and per-future polling make sqlx measurably slower than rusqlite for insert-heavy workloads. Benchmarks in `diesel-rs/diesel` PR #3633 and independent posts (ThorstenHans 2023 "rusqlite vs sqlx") show rusqlite 1.5–2.5x faster on single-thread insert. Use sqlx only when the rest of the process is async and already paying the runtime cost.

**libsql** (`tursodatabase/libsql`). Turso's fork; adds WAL replication, virtual WAL, and a native vector index. Bulk-insert path is identical to upstream SQLite; no ingest improvement. Useful only for the replication features.

**sqlite-loadable** (`asg017/sqlite-loadable-rs`) lets you author loadable extensions and virtual tables in Rust. A plausible ingest pattern: expose NDJSON or Parquet as a virtual table and run `INSERT INTO events SELECT * FROM vtab` inside one transaction. **sqlite-zstd** (`phiresky/sqlite-zstd`) provides row-level transparent compression, cutting disk size 3–5x on log data per the README (vendor number) at roughly a 30 percent write-throughput cost.

### 15.2 Bulk-insert tuning recipe

Avinash Sajjanshetty's 2021 post "Towards inserting one billion rows in SQLite under a minute" is the canonical reference; rusqlite's `examples/` reproduce the same shape. The wins stack in this order:

- `BEGIN IMMEDIATE` once, commit per ~50k–100k rows. Three orders of magnitude over autocommit — 85 inserts/sec autocommit versus roughly 1M/sec batched on M1, the single largest lever.
- Prepared `INSERT` reused via `Statement::execute`. Re-preparing per row costs 30–50 percent.
- `PRAGMA journal_mode = WAL` for concurrent reads during ingest. `MEMORY` is faster but loses durability on crash; `OFF` is faster still but corrupts on crash.
- `PRAGMA synchronous = NORMAL` — the standard ingest setting, fsync only at WAL checkpoint. `OFF` adds maybe 10 percent and risks corruption on power loss.
- `PRAGMA temp_store = MEMORY`, `cache_size = -262144` (256 MiB), `mmap_size = 30000000000`, `page_size = 8192` or `16384`. Set before any DDL. Page size beyond 4096 helps wide rows; benchmark per shape.
- Multi-row `VALUES (..), (..), (..)` up to SQLite's 32k bound-parameter ceiling; roughly 50 percent gain over single-row prepared insert at batch 500 (Sajjanshetty).
- `execute_batch` parses a semicolon script — for DDL, not parameterized load.
- Virtual tables via `sqlite-loadable` or the built-in `csv` vtable plus `INSERT INTO real SELECT * FROM t` is competitive with the CLI `.import` and stays in-process. The CLI itself is a C-side prepared-insert loop in a transaction; replicating it in Rust matches throughput within noise.

Independent confirmation: kerkour.com 2023 "SQLite the only database you will ever need" reports roughly 400k inserts/sec single-threaded with WAL+NORMAL on a NVMe laptop — consistent with Sajjanshetty's numbers at smaller scale.

### 15.3 DuckDB from Rust

**duckdb-rs** (`duckdb/duckdb-rs`) wraps the C API. Three ingest paths:

- `COPY tbl FROM 'file.parquet'` and `INSERT INTO tbl SELECT * FROM read_parquet('file.parquet')` push the entire load into DuckDB's vectorized executor. This is by far the fastest path and the intended one for columnar files. `read_csv_auto` handles CSV with type inference; `read_json_auto` handles NDJSON; both detect `.gz` and `.zst` automatically and decompress internally with no Rust-side decoder.
- `Appender` API for row-at-a-time programmatic insert, roughly 5–10x faster than prepared `INSERT` per DuckDB docs, still much slower than `COPY` for bulk.
- Arrow C Data Interface: `Connection::register_arrow` ingests `RecordBatch` streams from the `arrow-rs` `parquet` crate directly, bypassing file parsing entirely.

DuckDB's own benchmarks (`duckdb.org/2021/12/03/duck-arrow.html`, `duckdb.org/2024/06/26/benchmarks-over-dataframes.html`) claim 10–100x over SQLite on aggregate-heavy workloads on Parquet. These are vendor figures, but the load-and-aggregate gap on Parquet is real and reproduced by ClickBench (`benchmark.clickhouse.com`), where DuckDB sits near the top and SQLite is not competitive on analytics. For pure row-oriented ingest of NDJSON to a transactional store, SQLite still wins.

### 15.4 Reading side

- **serde_json** is the baseline. **simd-json** (`simd-lite/simd-json`) reports 1.5–3x over serde_json; Discord's 2022 engineering post and `serde-rs/json-benchmark` confirm roughly 2x on `twitter.json`. **sonic-rs** (`cloudwego/sonic-rs`) claims further gains at 1.2–1.5x over simd-json on its own benches, with less independent verification.
- **csv** (`BurntSushi/rust-csv`) — reuse a single `ByteRecord` across rows; `Reader::from_reader` over a `BufReader` of arbitrary size. BurntSushi's repo numbers are 200–400 MB/s parse on a single core for typical CSV.
- **parquet** crate (`apache/arrow-rs`) exposes `RecordBatchReader`; stream 8k-row batches into DuckDB or into SQLite via a column-wise bulk insert.
- **flate2** with the `zlib-ng` backend is roughly 2x faster than the default `miniz_oxide` backend for gzip per the README. **zstd** (`gyscos/zstd-rs`) wraps libzstd; decompression at 500–1500 MB/s depending on level.
- Line framing: `BufRead::read_until(b'\n')` is adequate; switching to explicit `memchr::memchr` over a refilled buffer gains 10–20 percent on long lines per the memchr README. For NDJSON-over-gzip the bottleneck is decompression, not framing.

### 15.5 Production references

- `csvs-to-sqlite` (`simonw/csvs-to-sqlite`) and `sqlite-utils` (`simonw/sqlite-utils`): Python, informative for schema inference and `ANALYZE` invocation post-load; both use `executemany` inside a transaction with WAL+NORMAL.
- `datasette-loadable` ecosystem (asg017): demonstrates virtual-table ingest patterns.
- Vector has no `sinks/sqlite`; the closest sinks are `clickhouse` and `file`. SQLite was discussed in `vectordotdev/vector` issue #2150 and rejected on throughput grounds for their multi-sink topology.
- Parseable's staging-to-Parquet flusher is a useful reference for batch-size and flush-cadence discipline when the terminal format is columnar.

### 15.6 Recommendation sketch for a Spank Rust build

For the default SQLite backend: rusqlite with `bundled`, prepared `INSERT` in `BEGIN IMMEDIATE` transactions of 50k rows, `WAL` + `synchronous=NORMAL` + `temp_store=MEMORY` + `cache_size=-262144`; flate2 with the `zlib-ng` feature; simd-json for NDJSON decode; BurntSushi `csv` with `ByteRecord` reuse. For the DuckDB backend (columnar, read-heavy tenants): route Parquet through `COPY FROM` and NDJSON through `read_json_auto`; use the Appender only for the live-tail path where per-row latency matters. Benchmark harness lives alongside the code and asserts a regression gate (see `docs/Sparst.md §9.4`).

## 16. Linux Kernel Primitives for Reducing Copies on the Ingest Path

Scope: Linux kernel features relevant to moving bytes from a file or network socket into an embedded database with the fewest physical copies and the lowest syscall overhead. The section enumerates each primitive with what it does and does not do, its version, and its realistic value; three end-to-end paths follow.

### 16.1 sendfile(2)

Transfers bytes from a file fd to a socket fd without crossing into user space (since 2.2, unified with splice in 2.6.33). The destination must be a socket; the source must be a mmap-able regular file. Typical win is roughly 2x throughput for static-file HTTP serving (nginx, Apache mod_sendfile; Linux Journal 2003 "Zero Copy I"). Not relevant to the Spank pipeline: the bytes need to reach user space for parsing or `sqlite3_bind_*`. Noted and set aside.

### 16.2 splice(2) and vmsplice(2)

splice(2) (since 2.6.17) moves data between two file descriptors via a kernel pipe buffer with no user-space copy; vmsplice(2) gifts user pages into a pipe. The hard restriction is "one of the file descriptors must refer to a pipe," so socket-to-file splicing always routes through an intermediate pipe fd. Relevant path: socket → pipe → regular file, a true zero-copy network-to-disk route used by HAProxy and by some Kafka producers. Not relevant for SQLite or DuckDB: SQLite issues `pread`/`pwrite` on the database file (see `sqlite3.c` `unixRead`/`unixWrite`); bytes must exist in a user buffer before `sqlite3_step` runs the prepared INSERT. Splice cannot deliver into `sqlite3_bind_blob`.

### 16.3 tee(2)

Duplicates data between two pipes without consuming the source (since 2.6.17); pipe fds only. Plausible use is forking the HEC stream to both an archive file and a parser. In practice the in-memory queue already exists and user-space `memcpy` is negligible next to JSON parse and SQLite bind. Not useful.

### 16.4 mmap(2), MAP_POPULATE, MAP_HUGETLB, madvise

mmap maps a file into the process address space; the kernel populates PTEs on fault from the page cache. MAP_POPULATE (2.5.46) prefaults; MAP_HUGETLB (2.6.32) backs the mapping with explicit huge pages from hugetlbfs. `madvise` with MADV_SEQUENTIAL hints LRU behavior, MADV_WILLNEED triggers readahead, MADV_HUGEPAGE (2.6.38) opts the region into transparent huge pages.

Critical clarification: mmap eliminates the `read(2)` copy from page cache into a user buffer, but the CPU still touches every byte through the page-table mapping when the parser reads it; there is no physical copy elision unless the consumer is a DMA engine. mmap is zero-syscall-copy, not zero-physical-copy. Linus's well-known position (LKML 2000-04, restated on realworldtech 2018) is that for purely sequential one-shot reads, `read(2)` into a reusable buffer beats mmap by avoiding TLB churn and minor faults. SQLite exposes the tradeoff via `PRAGMA mmap_size`; the SQLite docs (`sqlite.org/mmap.html`) recommend it for read-heavy workloads, not write-heavy. For an INSERT-dominated path it is not a clear win.

### 16.5 io_uring

Asynchronous syscall submission/completion ring (since 5.1, production-ready around 5.11 with multishot accept, fixed buffers, and the security regression fixes). Batches `recv`, `read`, `write`, `fsync`, `accept`, `openat`; since 5.6 provides registered file descriptors and registered buffers that skip per-op fd lookup and `copy_from_user` of the iovec. Real wins: ScyllaDB's Seastar reactor (ScyllaDB 2020 blog), RocksDB MultiGet (6.21 release notes), Ceph BlueStore's io_uring backend. Crates: `tokio-uring`, `glommio` (Datadog), `rio` (Tyler Neely, sled). Neither SQLite nor DuckDB issues io_uring submissions internally — SQLite's VFS is `pread`/`pwrite`, DuckDB uses standard buffered I/O. But the file reader and the HEC socket are independent of the DB and can use it. Highest-leverage primitive on this list for Spank's shape.

### 16.6 MSG_ZEROCOPY and TCP_ZEROCOPY_RECEIVE

MSG_ZEROCOPY is the send-side `sendmsg` flag (since 4.14, Willem de Bruijn, commit 52267790ef52); the kernel pins user pages and DMAs directly, signaling completion via the error queue. TCP_ZEROCOPY_RECEIVE is the receive-side getsockopt (since 4.18, Eric Dumazet), requiring page-aligned mmap'd buffers and large MSS. Google's original patch posting and the netdev 2018 talk justified it on memcache and RPC workloads with roughly 50 percent CPU reduction at 32–64 KB receive sizes; the cover letter explicitly states small messages lose to per-op setup (page-table work, MSS alignment). A syslog or HEC line is typically under 1 KB. For these payloads zerocopy-rx is not justified without a batched framing that lets the kernel see multi-MSS contiguous buffers.

### 16.7 AF_XDP and XDP

eBPF-based packet-redirection bypass of the kernel TCP/IP stack (XDP since 4.8, AF_XDP since 4.18). Useful at 10 Mpps and up; reframes the entire networking model and forfeits TCP. Overkill for HEC. Noted and set aside.

### 16.8 O_DIRECT

`open(2)` flag that bypasses the page cache; buffers must be aligned (typically 512 B or filesystem block size). Interacts poorly with SQLite — the VFS assumes page-cache semantics for crash safety, and the docs warn against it. Relevant for a custom one-shot log reader that does not want to evict warmer pages; combine with a reused user-space ring buffer. Worth evaluating for a bulk-import command that scans multi-GB archives not destined to be reread.

### 16.9 posix_fadvise(2)

Hints to the page cache. POSIX_FADV_SEQUENTIAL doubles kernel readahead; POSIX_FADV_WILLNEED triggers immediate readahead of a range; POSIX_FADV_DONTNEED drops clean pages. Cheap and monotonically beneficial for sequential large-file ingest. Pitfall on DONTNEED: it does not write back dirty pages first, so calling it on a file that has been written through mmap or `write` leaves the data in cache regardless; `sync_file_range` or `fdatasync` first (Linus, LKML 2010-12 reply to Andrew Morton).

### 16.10 copy_file_range(2)

Kernel-side copy between two file fds (since 4.5); on filesystems with reflink support (XFS, Btrfs) it is a CoW operation with no data movement. Cross-filesystem support since 5.3. Useful for moving sealed buckets between tiers or for archive and restore in the storage layer; not on the ingest hot path.

### 16.11 readahead(2) and POSIX_FADV_WILLNEED

readahead(2) (since 2.4.13) synchronously initiates readahead on a range; equivalent in effect to POSIX_FADV_WILLNEED. Worth issuing on file open in the bulk-import path before the first read.

### 16.12 Huge pages

Transparent Huge Pages (since 2.6.38) opportunistically coalesce 4 KB pages into 2 MB pages; controlled by `/sys/kernel/mm/transparent_hugepage/enabled`. Explicit hugetlbfs requires a reserved pool and MAP_HUGETLB. If THP is `always` (default on many distros) and the SQLite mmap region is 2 MB-aligned and large enough, it gets THP without code changes. Cost: occasional latency spikes from khugepaged compaction; MongoDB and Redis recommend `madvise` or `never`. For an ingest service the spikes are usually tolerable; measure before tuning.

### 16.13 Path A — HEC (network) → SQLite INSERT

Copies in flight: NIC DMA into the kernel skb pool; skb → user recv buffer on `recv`; user buffer → SQLite-internal buffer via `sqlite3_bind_text`/`bind_blob` (one `memcpy` unless SQLITE_STATIC is passed); SQLite page cache → WAL on write; WAL → main DB on checkpoint; fsync barriers throughout. Realistic cuts: pass SQLITE_STATIC to `sqlite3_bind_blob` with the recv buffer, deferring its release until after `sqlite3_step` — saves one `memcpy` per row but requires lifetime discipline (the buffer must outlive the step). Use io_uring to batch `recv` with the WAL fsync, amortizing syscall cost across many rows. TCP_ZEROCOPY_RECEIVE is unattractive at typical log-line size per Google's own published numbers. Net: io_uring and SQLITE_STATIC are the two interventions worth pursuing.

### 16.14 Path B — File → SQLite or DuckDB INSERT

Copies: disk → page cache (DMA), page cache → user buffer (`read`) or page-table mapping (mmap), user buffer → parser working memory, parser output → bind buffer. Cuts: `posix_fadvise(POSIX_FADV_SEQUENTIAL)` and `readahead` at open; mmap with MADV_SEQUENTIAL for the scan; SQLITE_STATIC binds against slices of the mmap'd region (safe because the mmap outlives the prepared statement). For DuckDB the more dramatic move is `COPY FROM 'file.parquet'` or `read_csv_auto`, which elides the user-space parse entirely — DuckDB pulls the file through its own vectorized scanner directly into the column store. This is the largest available win on the file path and is independent of any kernel primitive.

### 16.15 Path C — HEC → NDJSON file on disk (Vector pattern)

Copies: NIC DMA → skb → user recv buffer → `write(2)` → page cache → eventual writeback. `splice(socket_fd, NULL, pipe_w, NULL, len, 0)` followed by `splice(pipe_r, NULL, file_fd, NULL, len, 0)` removes the user-space copy and is a real zero-copy network-to-disk path. The restriction is that bytes must pass through unmodified; this works for `/services/collector/raw` where the body is the log line as-is, but not for `/services/collector/event`, which requires parsing the JSON envelope to extract `event`, `host`, `source`, `time`. For the parsed endpoint, fall back to io_uring batching of `recv` and `write`; that is the pragmatic ceiling without giving up the framing.

### 16.16 Summary for Spank

The interventions with measurable payoff on Spank's shape are: io_uring for batched socket and disk operations on the input side (both paths), SQLITE_STATIC binds against stable buffers to save one `memcpy` per row, `posix_fadvise(POSIX_FADV_SEQUENTIAL)` and `readahead` on bulk file scans, and `splice` for the raw-endpoint network-to-file path. DuckDB's in-engine `COPY FROM` is the largest single win for columnar file ingest and is not a kernel technique at all. TCP_ZEROCOPY_RECEIVE, AF_XDP, and MAP_HUGETLB do not fit the workload size and are set aside.

## Appendix A — Rust Projects and Linux Resources Cited

This appendix lists every Rust crate, project, and Linux resource named in this document, with a documentation link and a source-repository link where one exists. It is the single index for follow-up reading.

### A.1 Rust HTTP, networking, and runtime

| Name | Documentation | Source |
|---|---|---|
| tokio | https://docs.rs/tokio | https://github.com/tokio-rs/tokio |
| hyper | https://docs.rs/hyper | https://github.com/hyperium/hyper |
| hyper-util | https://docs.rs/hyper-util | https://github.com/hyperium/hyper-util |
| axum | https://docs.rs/axum | https://github.com/tokio-rs/axum |
| axum-server | https://docs.rs/axum-server | https://github.com/programatik29/axum-server |
| axum-streams | https://docs.rs/axum-streams | https://github.com/abdolence/axum-streams-rs |
| tower | https://docs.rs/tower | https://github.com/tower-rs/tower |
| tower-http | https://docs.rs/tower-http | https://github.com/tower-rs/tower-http |
| tower-governor | https://docs.rs/tower_governor | https://github.com/benwis/tower-governor |
| actix-web | https://docs.rs/actix-web | https://github.com/actix/actix-web |
| actix-http | https://docs.rs/actix-http | https://github.com/actix/actix-web |
| warp | https://docs.rs/warp | https://github.com/seanmonstar/warp |
| reqwest | https://docs.rs/reqwest | https://github.com/seanmonstar/reqwest |
| tonic | https://docs.rs/tonic | https://github.com/hyperium/tonic |

### A.2 Rust TLS and crypto

| Name | Documentation | Source |
|---|---|---|
| rustls | https://docs.rs/rustls | https://github.com/rustls/rustls |
| aws-lc-rs | https://docs.rs/aws-lc-rs | https://github.com/aws/aws-lc-rs |
| ring | https://docs.rs/ring | https://github.com/briansmith/ring |
| native-tls | https://docs.rs/native-tls | https://github.com/sfackler/rust-native-tls |
| rcgen | https://docs.rs/rcgen | https://github.com/rustls/rcgen |
| webpki | https://docs.rs/webpki | https://github.com/rustls/webpki |

### A.3 Rust async runtimes (alternatives to tokio)

| Name | Documentation | Source |
|---|---|---|
| async-std | https://docs.rs/async-std | https://github.com/async-rs/async-std |
| smol | https://docs.rs/smol | https://github.com/smol-rs/smol |
| glommio | https://docs.rs/glommio | https://github.com/DataDog/glommio |
| monoio | https://docs.rs/monoio | https://github.com/bytedance/monoio |
| tokio-uring | https://docs.rs/tokio-uring | https://github.com/tokio-rs/tokio-uring |
| rio | https://docs.rs/rio | https://github.com/spacejam/rio |

### A.4 Rust parsers, regex, grammars

| Name | Documentation | Source |
|---|---|---|
| regex | https://docs.rs/regex | https://github.com/rust-lang/regex |
| regex-syntax | https://docs.rs/regex-syntax | https://github.com/rust-lang/regex |
| regex-automata | https://docs.rs/regex-automata | https://github.com/rust-lang/regex |
| aho-corasick | https://docs.rs/aho-corasick | https://github.com/BurntSushi/aho-corasick |
| memchr | https://docs.rs/memchr | https://github.com/BurntSushi/memchr |
| nom | https://docs.rs/nom | https://github.com/rust-bakery/nom |
| LALRPOP | https://lalrpop.github.io/lalrpop/ | https://github.com/lalrpop/lalrpop |
| pest | https://docs.rs/pest | https://github.com/pest-parser/pest |
| chumsky | https://docs.rs/chumsky | https://github.com/zesterer/chumsky |
| antlr4rust | (dormant) | https://github.com/rrevenantt/antlr4rust |
| tree-sitter | https://docs.rs/tree-sitter | https://github.com/tree-sitter/tree-sitter |

### A.5 Rust serialization and encoding

| Name | Documentation | Source |
|---|---|---|
| serde | https://serde.rs/ | https://github.com/serde-rs/serde |
| serde_json | https://docs.rs/serde_json | https://github.com/serde-rs/json |
| simd-json | https://docs.rs/simd-json | https://github.com/simd-lite/simd-json |
| sonic-rs | https://docs.rs/sonic-rs | https://github.com/cloudwego/sonic-rs |
| rkyv | https://docs.rs/rkyv | https://github.com/rkyv/rkyv |
| csv | https://docs.rs/csv | https://github.com/BurntSushi/rust-csv |
| parquet | https://docs.rs/parquet | https://github.com/apache/arrow-rs |
| arrow | https://docs.rs/arrow | https://github.com/apache/arrow-rs |
| flate2 | https://docs.rs/flate2 | https://github.com/rust-lang/flate2-rs |
| zstd | https://docs.rs/zstd | https://github.com/gyscos/zstd-rs |
| async-compression | https://docs.rs/async-compression | https://github.com/Nullus157/async-compression |

### A.6 Rust embedded databases and storage

| Name | Documentation | Source |
|---|---|---|
| rusqlite | https://docs.rs/rusqlite | https://github.com/rusqlite/rusqlite |
| libsqlite3-sys | https://docs.rs/libsqlite3-sys | https://github.com/rusqlite/rusqlite |
| sqlx | https://docs.rs/sqlx | https://github.com/launchbadge/sqlx |
| libsql | https://docs.turso.tech/libsql | https://github.com/tursodatabase/libsql |
| sqlite-loadable | https://docs.rs/sqlite-loadable | https://github.com/asg017/sqlite-loadable-rs |
| sqlite-zstd | https://github.com/phiresky/sqlite-zstd | https://github.com/phiresky/sqlite-zstd |
| duckdb-rs | https://docs.rs/duckdb | https://github.com/duckdb/duckdb-rs |
| sled | https://docs.rs/sled | https://github.com/spacejam/sled |
| diesel | https://docs.rs/diesel | https://github.com/diesel-rs/diesel |

### A.7 Rust observability

| Name | Documentation | Source |
|---|---|---|
| tracing | https://docs.rs/tracing | https://github.com/tokio-rs/tracing |
| tracing-subscriber | https://docs.rs/tracing-subscriber | https://github.com/tokio-rs/tracing |
| metrics | https://docs.rs/metrics | https://github.com/metrics-rs/metrics |
| log | https://docs.rs/log | https://github.com/rust-lang/log |
| env_logger | https://docs.rs/env_logger | https://github.com/rust-cli/env_logger |

### A.8 Log-pipeline and observability projects

| Name | Documentation | Source |
|---|---|---|
| Vector | https://vector.dev/docs/ | https://github.com/vectordotdev/vector |
| Quickwit | https://quickwit.io/docs | https://github.com/quickwit-oss/quickwit |
| Parseable | https://www.parseable.com/docs | https://github.com/parseablehq/parseable |
| GreptimeDB | https://docs.greptime.com/ | https://github.com/GreptimeTeam/greptimedb |
| Tantivy | https://docs.rs/tantivy | https://github.com/quickwit-oss/tantivy |
| OpenTelemetry Collector contrib | https://opentelemetry.io/docs/collector/ | https://github.com/open-telemetry/opentelemetry-collector-contrib |
| OTel splunkhecreceiver | (in repo above) | https://github.com/open-telemetry/opentelemetry-collector-contrib/tree/main/receiver/splunkhecreceiver |
| splunk-rs (client) | https://docs.rs/splunk | https://github.com/yaleman/splunk-rs |

### A.9 Rust reverse proxies

| Name | Documentation | Source |
|---|---|---|
| Pingora | https://github.com/cloudflare/pingora/blob/main/docs/quick_start.md | https://github.com/cloudflare/pingora |
| River | https://github.com/memorysafety/river | https://github.com/memorysafety/river |
| Sozu | https://www.sozu.io/ | https://github.com/sozu-proxy/sozu |
| static-web-server | https://static-web-server.net/ | https://github.com/static-web-server/static-web-server |

### A.10 Bulk-storage references and benchmark sources

| Name | URL |
|---|---|
| SQLite mmap design notes | https://sqlite.org/mmap.html |
| Sajjanshetty 2021, "Towards inserting one billion rows in SQLite under a minute" | https://avi.im/blag/2021/fast-sqlite-inserts/ |
| kerkour.com 2023, "SQLite the only database you will ever need" | https://kerkour.com/sqlite-for-servers |
| DuckDB ingest benchmark, Arrow integration | https://duckdb.org/2021/12/03/duck-arrow.html |
| DuckDB benchmarks over dataframes | https://duckdb.org/2024/06/26/benchmarks-over-dataframes.html |
| ClickBench (third-party analytics benchmark) | https://benchmark.clickhouse.com/ |
| ScyllaDB, "How io_uring and eBPF will revolutionize programming in Linux" | https://www.scylladb.com/2020/05/05/how-io_uring-and-ebpf-will-revolutionize-programming-in-linux/ |
| RocksDB 6.21 release notes | https://github.com/facebook/rocksdb/releases/tag/v6.21.3 |
| simd-json benchmarks | https://github.com/serde-rs/json-benchmark |
| memchr README throughput | https://github.com/BurntSushi/memchr |

### A.11 Linux kernel resources

| Topic | Manual / source | Notes |
|---|---|---|
| sendfile(2) | https://man7.org/linux/man-pages/man2/sendfile.2.html | since 2.2 |
| splice(2) | https://man7.org/linux/man-pages/man2/splice.2.html | since 2.6.17 |
| vmsplice(2) | https://man7.org/linux/man-pages/man2/vmsplice.2.html | since 2.6.17 |
| tee(2) | https://man7.org/linux/man-pages/man2/tee.2.html | since 2.6.17 |
| mmap(2), MAP_POPULATE, MAP_HUGETLB | https://man7.org/linux/man-pages/man2/mmap.2.html | |
| madvise(2), MADV_SEQUENTIAL, MADV_WILLNEED, MADV_HUGEPAGE | https://man7.org/linux/man-pages/man2/madvise.2.html | MADV_HUGEPAGE since 2.6.38 |
| io_uring | https://man7.org/linux/man-pages/man7/io_uring.7.html | since 5.1, prod-ready ~5.11 |
| io_uring intro by Jens Axboe | https://kernel.dk/io_uring.pdf | |
| MSG_ZEROCOPY (send) | https://www.kernel.org/doc/html/latest/networking/msg_zerocopy.html | since 4.14 |
| TCP_ZEROCOPY_RECEIVE getsockopt | https://lwn.net/Articles/752188/ | since 4.18 |
| AF_XDP | https://www.kernel.org/doc/html/latest/networking/af_xdp.html | since 4.18 |
| XDP overview | https://prototype-kernel.readthedocs.io/en/latest/networking/XDP/ | since 4.8 |
| O_DIRECT (open(2)) | https://man7.org/linux/man-pages/man2/open.2.html | |
| posix_fadvise(2) | https://man7.org/linux/man-pages/man2/posix_fadvise.2.html | |
| copy_file_range(2) | https://man7.org/linux/man-pages/man2/copy_file_range.2.html | since 4.5; cross-fs since 5.3 |
| readahead(2) | https://man7.org/linux/man-pages/man2/readahead.2.html | |
| Transparent Huge Pages | https://www.kernel.org/doc/html/latest/admin-guide/mm/transhuge.html | since 2.6.38 |
| Linus on mmap vs read for sequential | https://www.realworldtech.com/forum/?threadid=181725&curpostid=181759 | |
| Linus on POSIX_FADV_DONTNEED dirty-page pitfall | https://lore.kernel.org/lkml/1292975239.4396.43.camel@laptop/ | |
| Willem de Bruijn, MSG_ZEROCOPY commit | https://lore.kernel.org/all/20170814173316.205129-1-willemdebruijn.kernel@gmail.com/ | |
| Eric Dumazet, TCP_ZEROCOPY_RECEIVE patch | https://lore.kernel.org/netdev/20180416214620.30791-1-edumazet@google.com/ | |

### A.12 Other prior-art references cited in this document

| Name | URL |
|---|---|
| ANTLR4 release timeline | https://github.com/antlr/antlr4/releases |
| ANTLR4 Rust target (dormant) | https://github.com/rrevenantt/antlr4rust |
| Python CPython source (`Modules/_sre/sre.c`) | https://github.com/python/cpython/blob/main/Modules/_sre/sre.c |
| Python `regex` package (mrabarnett) | https://github.com/mrabarnett/mrab-regex |
| HAProxy splice usage | https://www.haproxy.com/blog/haproxy-and-splice |
| Vector splunk_hec source | https://github.com/vectordotdev/vector/tree/master/src/sources/splunk_hec |
| Vector file sink | https://github.com/vectordotdev/vector/blob/master/src/sinks/file/mod.rs |
| Splunk HEC reference | https://docs.splunk.com/Documentation/Splunk/latest/Data/UsetheHTTPEventCollector |

---

## 17. Storage Backend Deep-Dive

Scope: the three-tier storage model (hot SQLite, warm Parquet, cold object store), the tuning recipe for each, the DuckDB analytics path, and the concurrency argument behind the bucket-per-file pattern.

### 17.1 The bucket-per-file concurrency argument

SQLite's WAL mode allows concurrent readers and one writer per database file. The bucket-per-file pattern — one `.db` per tag — converts what would otherwise be a single writer bottleneck into N independent WAL locks, one per active ingest stream. The consequence is that two sources writing to different tags never contend; two sources writing to the same tag serialize at the WAL level, which is the minimum serialization cost given SQLite's architecture. The alternative of a single shared database with table-per-tag reintroduces single-writer contention across all tags and removes the option of moving a completed bucket to warm storage as a self-contained unit.

### 17.2 Hot-tier SQLite tuning recipe

The five PRAGMA settings that matter for write-heavy ingest, applied in `SqliteBackend::tune`:

- `PRAGMA journal_mode=WAL` — enables concurrent readers; the write lock is acquired only at commit, not at transaction start.
- `PRAGMA synchronous=NORMAL` — fsyncs only at WAL checkpoints, not at every commit. Acceptable durability for an ingest sink that accepts client retries on 503; unacceptable for a ledger.
- `PRAGMA temp_store=MEMORY` — keeps temporary tables and indices in RAM, avoiding temp-file creation on every bulk sort.
- `PRAGMA cache_size=-262144` — 256 MB page cache per connection. The negative sign means kibibytes. Combined with WAL mode, this keeps hot pages in the process address space across transactions.
- `PRAGMA mmap_size=268435456` — 256 MB of the database file mapped into virtual address space. The kernel's page cache is shared with mmap readers, reducing copy overhead for concurrent readers.

The `BEGIN IMMEDIATE` transaction mode acquires the write lock at transaction start rather than at first write statement. This converts `SQLITE_BUSY` from a mid-transaction error (which would require rollback and retry) to a pre-transaction error (which can be retried at the caller boundary without lost writes). The prepared INSERT with bulk row binding over a 50k-row batch amortizes statement parsing and wire overhead. The combination of `BEGIN IMMEDIATE` + prepared INSERT + the five PRAGMAs produces approximately 500k–1M inserts per second on a mid-range NVMe, depending on row width.

### 17.3 SQLITE_STATIC bind optimization

`rusqlite`'s `ToSql` trait copies string and blob parameters into SQLite's internal buffer by default. For bulk inserts where the row data is already in a Rust `Vec<u8>` or `&str` with lifetime guaranteed to outlast the statement, `SQLITE_STATIC` passes the pointer directly, saving one `memcpy` per field per row. The API surface is `params_from_iter` with `rusqlite::types::ToSql` implementations that use `unsafe { sqlite3_bind_text(..., SQLITE_STATIC) }`. The lifetime discipline is: the bind must be issued and the statement must be stepped within the same scope as the source data. Any `await` point between bind and step breaks the lifetime contract and must not be introduced.

### 17.4 DuckDB as analytics layer

DuckDB is unsuitable for the hot-write path for two reasons. First, it has a single-writer architecture with no WAL-mode equivalent; concurrent writers require external serialization. Second, it does not support the per-file isolation that the bucket-per-file pattern relies on — DuckDB catalogs are process-scoped. DuckDB's strength is as an analytics layer over completed (immutable) buckets: `COPY FROM 'bucket-2024-01.parquet'` is the fastest DuckDB ingest path, roughly 2–5x faster than row-by-row inserts via the C Appender API for read-heavy analytical queries. The Appender API is appropriate for the live-tail path (where per-row latency is bounded by the consumer) but not for bulk historical ingestion.

### 17.5 Warm and cold tiers: Parquet via arrow-rs

Completed hot buckets are candidates for compaction to Parquet. The Rust ecosystem for this is: `arrow` (Apache Arrow in-memory columnar format), `parquet` (Parquet file writer/reader built on arrow), and `datafusion` (SQL query engine over Arrow datasets). The transformation path is: `rusqlite::Statement::query_map` → iterate rows → build `arrow::array::RecordBatch` → `parquet::arrow::AsyncArrowWriter::write_batch`. The resulting `.parquet` file can be queried by DuckDB, DataFusion, or uploaded to S3-compatible object stores as the cold tier. DataFusion's `SessionContext::register_parquet` allows SQL queries spanning multiple Parquet files with predicate pushdown, making it the natural SPL-to-SQL translation layer for the warm and cold tiers.

### 17.6 Throughput targets and regression gates

The `spank bench` subcommand (100k-row SQLite bulk insert, release profile) establishes the baseline. For planning purposes the targets are: hot-tier SQLite at 500k rows/second on a mid-range NVMe, hot-to-warm Parquet compaction at 1–5 GB/min depending on row width and compression codec, cold-tier upload bounded by object store throughput. Any change to the storage path should be verified against the bench baseline before merge; a regression greater than 20% on the hot-tier number warrants investigation before landing.

## 18. The Partition Layer Gap

Scope: what spank-py's partition coordinator does, what the current spank-rs `PartitionManager` trait surface provides, and what is missing above it.

### 18.1 What spank-py's partition layer does

In spank-py, the `IndexPartition` class represents one bucket (a directory containing rawdata, tsidx, and metadata). The `IndexWriter` coordinates the lifecycle of partitions for a single index: it creates hot buckets when the active one is full or aged, rotates hot to warm, moves warm to cold, enforces retention by dropping frozen buckets, and coordinates crash recovery by detecting incomplete bucket directories on startup. The hot bucket is the only writable partition; warm and cold are read-only. Reads scatter-gather across all partitions in time order, yielding a merged stream to the query engine. The `IndexWriter` holds the authoritative view of partition state; nothing above it makes bucket-lifecycle decisions.

The concrete operations the coordinator must perform:

1. Create a new hot bucket directory, apply schema, set metadata (`earliest_time`, `latest_time`, size watermark).
2. Rotate hot to warm: close the active writer, fsync, update bucket metadata, rename the directory from `db.hot` to `db.warm`.
3. Rotate warm to cold: compress or archive the Parquet file, move to cold storage.
4. Enforce retention: list buckets by `latest_time`, drop those outside the retention window.
5. Scatter-gather read: for a time-range query, collect the buckets whose `[earliest_time, latest_time]` interval intersects the query range, open each, merge results in time order.
6. Crash recovery: on startup, scan the bucket directory for any bucket in an intermediate state (e.g., `db.hot` with no metadata, indicating an interrupted rotation), replay or discard as appropriate.

### 18.2 What spank-rs currently provides

The `spank-store::traits` module defines three traits: `BucketWriter` (append rows, commit), `BucketReader` (count, scan by time range), and `PartitionManager` (create hot bucket, list buckets). `SqliteBackend` implements all three. The traits are the correct interface boundary — any backend (SQLite, DuckDB, Parquet) implements them.

What is absent: the coordinator above the traits. There is no Rust equivalent of `IndexWriter`. No code in the `serve` path calls `PartitionManager::create_hot` to rotate buckets based on size or age. No code performs retention. No code scatter-gathers across multiple buckets for a time-range query. No code performs crash recovery. The trait surface is right; the coordinator that drives it does not exist yet.

### 18.3 Impact on the current ingest path

Because the Partition coordinator does not exist, the `serve` subcommand cannot wire `SqliteBackend` to the HEC consumer. The current wiring uses `FileSender` instead: HEC events are written as JSON-lines files per tag. `FileSender` requires no bucket lifecycle management and no cross-bucket coordination. This is the deliberate short-term choice recorded in `Tracks.md §7`.

The consequence is that all search queries on the `/services/search/jobs` endpoint (currently a 501 stub) would need to read the JSON-lines files rather than querying SQLite or Parquet. The 501 stub masks this gap — there is nothing for the search layer to query yet.

### 18.4 Design sketch for the Rust coordinator

The coordinator is a single long-lived tokio task, `PartitionCoordinator::run`, holding a `PartitionManager` and the following state: the active hot bucket handle, a size watermark, an age deadline (next rotation time), and a retention window. It receives `RotateSignal` and `RetentionTick` messages via a channel. On `RotateSignal` or when the watermark or deadline fires, it calls `PartitionManager::create_hot` (opening a new bucket), swaps the `BucketWriter` reference held by the consumer, and rotates the old bucket to warm. Retention runs on a separate tick. Crash recovery runs at startup before the first hot bucket is opened.

## 19. Current spank-rs Architecture and Wiring

Scope: what is actually wired in the `serve` and `bench` subcommands as of the implementation pass covered by `Tracks.md`, and where the gaps are between what is implemented and what is wired.

### 19.1 The serve path

The `serve` subcommand in `crates/spank/src/main.rs` wires the following:

- API server: `spank_api::router::build` + `spank_api::serve` on the configured bind address.
- HEC receiver: `spank_hec::routes` backed by `HecState`, authenticated via `HecTokenAuthenticator`, bounded channel with `try_send` backpressure, gzip decode, JSON and raw-line parsing.
- HEC consumer: `spank_hec::receiver::spawn_consumer` draining the channel and dispatching `Rows` to `FileSender`. `FileSender` writes per-tag JSON-lines files to `cfg.hec.output_dir`.
- TCP receiver (optional): `spank_tcp::serve` if `cfg.tcp.bind` is set, outputting `ConnEvent` to a consumer that writes per-connection log files.
- `HecPhase` managed via `arc_swap::ArcSwap` in `ApiState`. Phase transitions: `STARTED` on construction, `SERVING` after subsystems are ready, `STOPPING` on shutdown, `DEGRADED` on subsystem failure.

`SqliteBackend` is not in the serve path. The store traits exist and the `SqliteBackend` compiles, but no part of `serve` opens a database file or calls `BucketWriter::append`.

### 19.2 The bench path

The `bench` subcommand is the only path where `SqliteBackend` is exercised: it opens a `tempfile::tempdir`, creates a hot bucket, builds 100k `Record` values, calls `BucketWriter::append`, and commits. The output is `sqlite bulk_insert n=100000 elapsed_ms=X inserts_per_sec=Y`. This path validates the storage layer in isolation but does not validate it under concurrent ingest or with the HEC consumer in the loop.

### 19.3 The HecPhase arc-swap pattern

`HecPhase` is stored as `Arc<ArcSwap<HecPhase>>` in `ApiState`. `ArcSwap` allows lock-free atomic replacement of the `Arc<HecPhase>` pointer. On the hot path (HEC request handler checking phase admission), the read is: `let guard = self.phase.load(); let phase = guard.deref();` — two pointer dereferences with no lock. The write (phase transition) is: `self.phase.store(Arc::new(new_phase))`. The `ArcSwap` approach is preferred over `RwLock<HecPhase>` because read-side contention under high HEC request throughput degrades a `RwLock` more than an `ArcSwap`.

### 19.4 Gaps between implemented and wired

The table below lists each implemented component and its wiring status in the `serve` path.

| Component | Implemented | Wired in serve |
| - | - | - |
| `FileSender` | yes | yes — HEC consumer |
| `SqliteBackend` | yes | no — bench only |
| `PartitionCoordinator` | no | no |
| `BucketWriter` trait | yes | no |
| Search endpoint | no (501 stub) | no |
| Index management endpoint | no (501 stub) | no |
| Auth management endpoint | no (501 stub) | no |
| HEC ack endpoint | no | no |
| Persistent queueing (WAL) | no | no |
| Shipper jitter | no | no |
| TCP counted drops | no (silent drop) | no |

## 20. Splunk On-Disk Formats

Scope: the structure of a Splunk bucket directory, the rawdata format, the tsidx inverted index, and the Bloom filter. This is the format that a search-compatible implementation must be able to read or produce; it is not a public specification and the details are inferred from documentation and inspection.

### 20.1 Bucket directory layout

A Splunk bucket is a directory on the indexer's filesystem, named with a triple `db_<latest>_<earliest>_<bucket_id>` (for hot: `hot_v1_<bucket_id>`). The directory contains:

```
db_1700000000_1699990000_12345/
  rawdata/
    journal.gz          raw events, gzip-compressed, variable-length records
    slices.dat          index into journal.gz: offset table for record lookup
  tsidx                 inverted term index (proprietary binary format)
  bloomfilter           Bloom filter over tsidx terms
  Strings.data          string table referenced by tsidx
  optimize.db           SQLite file used by Splunk for bucket metadata
  bucket.db             (hot only) SQLite WAL journal before compaction
  .metadata/            JSON bucket manifest
```

The `rawdata/journal.gz` file contains all raw events for the bucket in order of indexing. Each record is a variable-length binary blob containing the event text, the `_time` field (Unix timestamp), and an offset pointer. `slices.dat` provides an offset table so the indexer can seek to any record without decompressing the entire journal.

### 20.2 The tsidx inverted term index

The `tsidx` file is Splunk's proprietary inverted index over the event text. Each unique term that appears in any event in the bucket has an entry mapping it to a sorted list of event offsets (positions in `journal.gz`). The format is not publicly documented; inspection suggests it is a flat binary file with a term dictionary and a posting list per term. The `Strings.data` file holds the string table for terms that exceed an inline length limit.

For a Rust implementation that does not intend to read `.tsidx` files produced by Splunk, the equivalent is a term-frequency inverted index built at bucket compaction time over the Parquet column containing the raw event text. `tantivy` (a Rust full-text search library) provides a compatible inverted index that can be co-located with a Parquet bucket. Search queries that are not time-bounded use the inverted index to prune buckets before opening them.

### 20.3 Bloom filters

The `bloomfilter` file is a per-bucket Bloom filter over the terms in `tsidx`. A search query tests the Bloom filter before opening the bucket: a definite negative skips the bucket entirely; a positive (which may be a false positive) opens the bucket and queries `tsidx`. The Bloom filter reduces disk I/O for selective term searches across a large number of buckets.

The Rust equivalent is `bloomfilter::Bloom` (or a hand-rolled FNV-hash Bloom filter) serialized alongside the Parquet file at compaction time. The filter should be sized for the expected number of unique terms per bucket and the target false-positive rate; 1% false-positive rate at 10M unique terms requires approximately 12 MB. This is an optional optimization; the base implementation queries each bucket without a Bloom filter.

### 20.4 Implications for spank-rs

A spank-rs that aims for Splunk search-API compatibility must produce buckets that either match the Splunk on-disk format or provide a translation layer. The current `FileSender` path produces neither. The target storage model defined in `docs/Sparst.md §4` aims for Parquet-based warm/cold storage with a `tantivy`-backed inverted index; this is not format-compatible with Splunk's tsidx but is query-compatible with Splunk's SPL search semantics when the query engine is spank-rs's own SPL executor.

## 21. SPL Functional Requirements by Tier

Scope: the SPL command set organized into three implementation tiers, with SQL equivalents where they exist. The tiers correspond to implementation priority: Tier 1 is minimum viable search, Tier 2 is analyst-ready, Tier 3 is Splunk-equivalent. Each tier is stated as a set of SPL commands and the SQL or algorithmic equivalent.

### 21.1 Tier 1 — Minimum viable search

Tier 1 provides the operations a log consumer needs to answer "what happened and when": time filtering, keyword search, field selection, basic limiting, and ordering. These operations map directly to SQL SELECT.

| SPL command | SQL equivalent | Notes |
| - | - | - |
| `search <terms>` | `WHERE _raw LIKE '%term%'` or FTS index | Time-range filter applied before full-text scan. |
| `earliest=`, `latest=` | `WHERE _time BETWEEN ? AND ?` | Maps to `time_event` column index. |
| `fields <f1>,<f2>` | `SELECT f1, f2` | Requires JSON field extraction from `_raw`. |
| `head <N>` | `LIMIT N` | Combined with `ORDER BY _time DESC`. |
| `tail <N>` | `ORDER BY _time ASC LIMIT N` | Reversed. |
| `sort by <field>` | `ORDER BY field ASC/DESC` | Single-column sort is standard SQL. |
| `where <expr>` | `WHERE <expr>` | Scalar expression; subset of SPL eval syntax. |
| `rename <f> AS <g>` | Column alias in SELECT | Applied at projection layer. |

The implementation path: SPL parser produces a `SearchAst`; the Tier-1 executor translates `SearchAst` nodes to a `rusqlite` `Statement` or a DataFusion `LogicalPlan` against the Parquet files for the relevant time range. The `time_event` index in `SqliteBackend` makes time-range queries efficient on the hot tier; predicate pushdown in DataFusion handles the warm tier.

### 21.2 Tier 2 — Analyst-ready

Tier 2 provides aggregation, field extraction, and time-series visualization. These require a more sophisticated execution model than simple SQL projection because SPL's `stats` and `timechart` commands operate over a streaming row set, not a bounded table.

| SPL command | SQL/algorithmic equivalent | Notes |
| - | - | - |
| `stats count by <field>` | `SELECT field, COUNT(*) GROUP BY field` | Standard SQL aggregation. |
| `stats avg,min,max,sum by <field>` | `SELECT field, AVG(v), MIN(v), MAX(v), SUM(v) GROUP BY field` | Multi-aggregate. |
| `eval <field>=<expr>` | Computed column or `SELECT expr AS field` | SPL eval has a richer expression language than SQL. |
| `rex field=_raw "<regex>"` | Regex capture group extraction | No SQL equivalent; requires regex engine per row. |
| `timechart span=<N>m count` | `SELECT date_bin('N minutes', _time) AS bucket, COUNT(*) GROUP BY bucket` | DataFusion has `date_bin`. |
| `dedup <field>` | `SELECT DISTINCT ON (field)` | PostgreSQL dialect; DataFusion supports it. |

The execution model for Tier 2 is a pipeline: `Source` (scatter-gather across buckets) → `Filter` (Tier-1 predicates) → `Transform` (eval, rex) → `Aggregate` (stats, timechart) → `Limit` (head/tail). Each stage is a `Stream` in the DataFusion sense. The Python equivalent is the generator pipeline in `spank-py`'s search executor.

### 21.3 Tier 3 — Splunk-equivalent

Tier 3 provides the commands that make SPL distinct from SQL: streaming statistics, event grouping, external data enrichment, and multi-source joins. These have no direct SQL equivalent and require stateful execution across an unbounded event stream.

| SPL command | Algorithmic approach | Notes |
| - | - | - |
| `streamstats` | Sliding-window aggregation over ordered stream | Requires stateful per-group accumulators with time-ordered input. |
| `eventstats` | Two-pass: first pass computes group stats, second pass decorates each row | Cannot be expressed as a single SQL query. |
| `transaction` | Group consecutive events by field equality with gap tolerance | Stateful grouping; output row count differs from input. |
| `lookup` | Hash join against a static CSV or KV-store table | DataFusion supports hash joins; the lookup table must be materialized. |
| `join` | Full join between two sub-searches | DataFusion supports joins; sub-searches run as sub-plans. |

Tier 3 is the correct scope for a second implementation phase. The architectural prerequisite is a DataFusion custom `ExecutionPlan` node for each stateful SPL operator. Tier 3 is not blocked by the storage layer — it is blocked by the SPL parser and planner not yet existing.

## 22. Gaps and Reduced Functionality vs spank-py

Scope: a factual inventory of capabilities that exist in spank-py but are absent or incomplete in the current spank-rs implementation. Each item names the gap, its impact, and the implementation path.

### 22.1 Search endpoint (501 stub)

`GET /services/search/jobs` and `POST /services/search/jobs` return 501 Not Implemented. No SPL parser, no query planner, no search executor exists in the tree. Impact: any client that attempts a search against spank-rs receives an error. Implementation path: SPL Tier-1 parser using `nom` or `pest`, a `LogicalPlan` translator, and a DataFusion executor over the warm-tier Parquet files. The hot-tier SQLite path requires a parallel translator.

### 22.2 No Partition coordinator

As documented in `§18`, no coordinator drives bucket rotation, retention, or crash recovery. Impact: `SqliteBackend` cannot be wired into the serve path because no code manages the bucket lifecycle above the `BucketWriter` trait. The `FileSender` workaround does not scale beyond development volumes and is not query-compatible with a future search layer. Implementation path: `PartitionCoordinator::run` as described in `§18.4`.

### 22.3 No index management endpoint

`GET /services/data/indexes` returns 501 Not Implemented. In Splunk this endpoint lists indexes, their size, event count, and retention settings. Impact: monitoring tools and forwarder configurations that query the index list fail. Implementation path: a handler that reads bucket metadata from the Partition coordinator and returns a JSON array.

### 22.4 No authentication management endpoint

`GET /services/authentication/users` returns 501 Not Implemented. In Splunk this endpoint lists principals and their roles. Impact: management tools that enumerate users fail. Implementation path: a handler backed by the `TokenStore`; no RBAC is required for Tier-1 compatibility.

### 22.5 No HEC acknowledgment endpoint

The HEC `POST /services/collector/ack` endpoint, used by indexer acknowledgment (IXF) mode, is not implemented. When a Splunk forwarder sends events with `X-Splunk-Request-Channel`, it expects the server to track which channels have been committed and return ack tokens. Impact: forwarders configured for guaranteed delivery will not work; they will either time out or disable ack mode and fall back to fire-and-forget. Implementation path: an in-memory ack registry keyed by channel ID, persisted to a SQLite sidecar file so tokens survive restarts.

### 22.6 No persistent queueing on the HEC receive side

If the HEC consumer falls behind (slow storage, large burst), the bounded channel fills and `try_send` returns `QueueFull` (HTTP 503). The client must retry. This is documented in `Tracks.md §3`. There is no WAL or disk-backed queue between the HEC receiver and the consumer. Impact: under a sustained burst that exceeds the consumer's throughput, the sender receives 503s for the duration of the burst, relying on client-side retry. Implementation path: an embedded write-ahead log (e.g., a SQLite FIFO table or a `redb`-backed queue) that the receiver appends to under backpressure, decoupling the receive rate from the commit rate.

### 22.7 Shipper lacks jitter (thundering-herd hazard)

`spank_shipper::TcpSender::run` uses pure exponential backoff (100ms initial, 30s cap) with no jitter. Under a fleet restart where all shippers reconnect simultaneously, all instances will retry at the same interval cadences, creating a synchronized load spike on the receiver. Impact: acceptable for a single-instance deployment; a fleet-deployment hazard. Implementation path: add `rand::thread_rng().gen_range(0..base_delay/2)` as a jitter term before each sleep. This is the standard full-jitter formula from the AWS Architecture Blog on exponential backoff.

### 22.8 TCP receiver drops lines silently on full channel

In `spank_tcp::receiver::run_connection`, `tx.try_send(ConnEvent::Line { ... })` is called and the result is silently discarded with `let _ = ...`. A full channel drops the line with no counter increment. Impact: data loss under backpressure is undetected. A monitoring operator cannot distinguish "no events arrived" from "events arrived but were dropped". Implementation path: replace with `match tx.try_send(...) { Err(TrySendError::Full(_)) => { metrics::increment_counter!(names::TCP_DROPS_TOTAL); } ... }` and add `TCP_DROPS_TOTAL` to `spank-obs::metrics::names`.

### 22.9 No tsidx or full-text index

The current storage layer has no full-text inverted index. Searches on `_raw` require a full-table scan of every bucket in the time range. Impact: for small volumes this is acceptable; for tenants with TB-scale warm storage the scan time makes interactive search impractical. Implementation path: `tantivy`-backed inverted index written at bucket compaction time, as described in `§20.3`.
