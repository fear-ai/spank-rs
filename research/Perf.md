# Perf — Spank Performance, Layout, and Implementation Notes

`Focus: research/performance` — concentrated design material for beating a general-purpose pipeline such as Vector on a narrow local-search workload. Covers hardware assumptions, syscall paths, parsing, normalization, tokenization, indexing, durable commits, query skip/selectivity, code structure, optional optimized implementations, and benchmark discipline. Does not track implementation status; use `Plan.md` or `Tracks.md` for that.

## Scope

Spank should not try to out-Vector Vector as a universal observability router. The plausible win is narrower: ingest known log families, commit them durably to local storage, and prepare search structures shaped around Sigma/SPL-style predicates. This document records the performance argument and the code structure needed to keep optimized paths optional, testable, and replaceable.

## Table of Contents

1. Core Thesis
2. Ingest vs Search Prep
3. Hardware and Runtime Shape
4. Network and File Syscall Paths
5. Buffer, Batch, Value, and Alignment Sizing
6. Parse and Normalize Computations
7. Syslog Profile
8. Apache Combined Log Profile
9. Tokenization and Index Construction
10. Search-Shaped Storage
11. Durable Commit Modes
12. Skip Rate and Query Selectivity
13. SIMD, `memchr`, and CPU Dispatch
14. Code Structure and Optional Optimized Paths
15. Vector Comparison
16. Benchmark Protocol

---

## 1. Core Thesis

Vector is optimized for broad routing, transforms, and sinks. Spank can beat Vector only by specializing:

- avoid generic event-pipeline tax;
- parse known log types into fixed columns;
- store raw bytes once and typed searchable facts separately;
- build Sigma/SPL-shaped side indexes;
- batch and align work for cache, core, and NVMe behavior;
- keep local durability semantics explicit.

The kernel is the same for Rust and Python, and largely the same for Spank and Vector. `recv`, `read`, `write`, `fdatasync`, and `fsync` are not inherently cheaper because the caller is Spank. The win must come from the work between syscalls: parse, normalize, tokenize, batch, bind, index, compact, and query planning.

---

## 2. Ingest vs Search Prep

Keep two planes.

```text
Input
  -> framing
  -> parse
  -> normalize
  -> local durable store
  -> async search-prep/index builders
  -> query-optimized sidecars
```

**Ingestion into local store** synchronously does:

- frame line/body;
- parse minimum required fields;
- normalize typed columns;
- append raw payload;
- commit durable batch;
- record raw offsets.

**Ingestion should not synchronously do by default:**

- large regex sets;
- full-text segment merge;
- Tantivy commit;
- Parquet compaction;
- expensive user-agent parsing;
- global dictionary optimization.

**Search prep** asynchronously does:

- tokenize selected fields;
- build field dictionaries;
- build bitmaps/postings;
- build Bloom filters;
- write Parquet row groups;
- build Tantivy or custom index sidecars;
- compute per-bucket stats.

Two acknowledgment modes follow:

| Mode | Ack after | Tradeoff |
|---|---|---|
| durable ingest | local WAL/SQLite/file commit | fast; search index may lag |
| searchable ingest | side index committed too | slower; stronger query freshness |

Default HEC ack should mean durable local ingest commit. Search-prep lag should be observable, not hidden.

---

## 3. Hardware and Runtime Shape

Starting allocation on a 16 physical-core server:

| Role | Cores | Notes |
|---|---:|---|
| network/file input | 2 | accept/read/framing; avoid heavy parse here |
| parse + normalize | 6 | CPU hot path; profile carefully |
| tokenize/index build | 4 | batch-local maps, sort, delta encode |
| local writers | 2 | one writer owner per active bucket |
| API/query | 1 | query admission and light planning |
| slack/OS/IRQs | 1 | leave room for kernel and interrupts |

Rules:

- pin long-lived workers only after baseline benchmarks;
- keep writer ownership single-threaded per bucket;
- avoid one global hot queue;
- use bounded channels for real backpressure;
- keep CPU-heavy work out of Tokio worker tasks unless it yields or uses `spawn_blocking`/Rayon.

Tokio tasks are user-space futures multiplexed onto OS threads. A task switch avoids a kernel context switch, but scheduling is cooperative. A CPU loop without `.await` can starve peers. For parse/index work, a dedicated Rayon pool or dedicated worker threads may be cleaner than treating everything as async.

---

## 4. Network and File Syscall Paths

### 4.1 Network HEC Path

Typical copies:

```text
NIC DMA
  -> kernel socket buffer
  -> user receive/TLS/HTTP buffer
  -> parser/normalizer views or copies
  -> durable store bind/write buffers
```

Linux readiness path is usually `epoll`; macOS/BSD readiness path is usually `kqueue`. Tokio reaches these through `mio`. Python async stacks such as uvloop use the same kernel family; the difference is that Rust does parse/normalize/index work without a GIL.

Useful knobs:

- front proxy for TLS if handshakes dominate;
- `SO_REUSEPORT` if multiple accept loops are needed;
- `TCP_NODELAY` for small ack-like responses;
- bounded body size;
- bounded ingest queue;
- explicit body-read and header-read timeouts;
- NIC IRQ/RPS/XPS tuning only after profiling.

### 4.2 File Ingest Path

Regular files are not readiness-driven like sockets. Tokio file I/O is normally blocking-pool backed. Current `spank-rs` file monitor is deliberately simple: read to EOF, sleep, `stat`/inode check, reopen on rotation.

Typical syscalls:

```text
open
read
lseek
stat/fstatat
close
```

If using native watching:

- Linux: `inotify_init1`, `inotify_add_watch`, `read` on inotify fd, `inotify_rm_watch`;
- macOS: FSEvents for recursive/high-level watching, sometimes `kqueue`/`EVFILT_VNODE` for fd-level vnode events.

Bulk file ingest knobs:

- `posix_fadvise(POSIX_FADV_SEQUENTIAL)`;
- `readahead`;
- large reusable read buffers;
- `memchr` newline framing;
- avoid `mmap` for one-shot sequential scans unless benchmarked;
- consider `O_DIRECT` only for huge one-shot imports that should not pollute page cache.

---

## 5. Buffer, Batch, Value, and Alignment Sizing

Typical log profiles:

| Profile | Avg event | Shape | Parse cost |
|---|---:|---|---|
| syslog/plain text | 100–400 B | line + few fields | low |
| firewall/netflow JSON | 300–900 B | flat JSON | medium |
| Sysmon/EDR JSON | 800 B–3 KiB | nested-ish JSON + long strings | high |
| Kubernetes/container logs | 500 B–2 KiB | JSON wrapper + raw payload | medium/high |
| Apache combined | 150–800 B | delimiter format | low/medium |

Starting ranges:

| Knob | Typical range | Starting value |
|---|---:|---:|
| read buffer | 256 KiB–4 MiB | 1 MiB |
| parse batch | 4K–64K events | 16K |
| raw bytes per batch | 4–64 MiB | 16 MiB |
| SQLite commit batch | 10K–100K rows | 50K |
| Parquet row group | 64K–1M rows | 256K |
| channel depth | 2–8 batches | 4 |

Example: 16K Sysmon events at 1 KiB average:

```text
raw buffer:       ~16 MiB
fixed columns:    ~1–2 MiB
token scratch:     ~2–8 MiB
total batch:      ~20–30 MiB
```

That fits in many server L3 caches, not L2. Per-core parse workers should operate on slices/sub-batches rather than all workers mutating one hot structure.

### 5.1 Normalized Row Sizing

Avoid retaining generic maps:

```rust
HashMap<String, Value>
```

Prefer batch-column layout:

```rust
struct Batch {
    time_ns: Vec<i64>,
    event_id: Vec<u32>,
    host_id: Vec<u32>,
    user_id: Vec<u32>,
    image_id: Vec<u32>,
    raw_offset: Vec<u32>,
    raw_len: Vec<u32>,
    raw: Vec<u8>,
}
```

Typical fixed-column cost:

| Field | Type | Bytes/event |
|---|---|---:|
| `_time` | `i64` | 8 |
| `event_id` | `u32` | 4 |
| interned host/user/image | `u32` each | 4 each |
| raw offset + length | `u32 + u32` | 8 |
| IPv4 | `u32` | 4 |
| IPv6 | `[u8; 16]` | 16 |
| port | `u16` | 2 |
| flags/action/result | `u8`/`u16`/`u32` enum | 1–4 |

A rich normalized row can be ~48–128 fixed bytes plus raw payload, instead of a multi-kilobyte object graph.

### 5.2 Alignment

Useful boundaries:

| Boundary | Use |
|---:|---|
| 64 B | cache line; queue heads, counters, worker state |
| 4 KiB | page boundary; I/O buffers |
| 2 MiB | huge page boundary; large arenas where measured useful |
| 16/32/64 B | SIMD vector loads for SSE/AVX/AVX-512/NEON |

Avoid false sharing:

```rust
#[repr(align(64))]
struct WorkerCounters {
    parsed: AtomicU64,
    dropped: AtomicU64,
}
```

Do this for hot counters and queue producer/consumer cursors. Do not align every row; columnar layout and batch locality matter more.

---

## 6. Parse and Normalize Computations

Rough single-core ranges in optimized Rust, assuming data is already in memory:

| Stage | Conservative | Good | Excellent |
|---|---:|---:|---:|
| newline framing with `memchr` | 3–8 GB/s | 10–20 GB/s | 20+ GB/s |
| flat JSON parse | 150–400 MB/s | 500–900 MB/s | 1+ GB/s with SIMD parser |
| field normalization | 1–5M events/s | 5–15M events/s | field-count dependent |
| tokenization | 300 MB/s–1 GB/s | 1–3 GB/s | higher for ASCII/simple rules |
| SQLite hot insert | 100k–400k rows/s | 500k–1M rows/s | batch/NVMe dependent |

These are planning ranges, not claims. Every number needs a fixture and benchmark before becoming a target.

Fast-path parser principles:

- parse bytes first; decode UTF-8 only when needed;
- keep offsets into raw buffer;
- avoid allocation per field;
- intern repeated strings per batch;
- use specialized byte parsers for integers/IPs/timestamps;
- classify before applying regex;
- split strict fast path from tolerant slow path;
- store parse errors as counters and sampled examples.

---

## 7. Syslog Profile

Example:

```text
Apr 29 12:34:56 host1 sshd[1234]: Failed password for invalid user root from 10.0.0.5 port 54321 ssh2
```

Targets:

```text
timestamp = Apr 29 12:34:56
host      = host1
process   = sshd
pid       = 1234
message   = Failed password...
src_ip    = 10.0.0.5
src_port  = 54321
user      = root
action    = failed_password
```

### 7.1 Parse Computation

Naive:

```rust
let parts: Vec<&str> = line.split_whitespace().collect();
```

Problems: vector allocation, lost offsets, variable message grammar, timestamp lacks year/timezone, process grammar has edge cases.

Optimized:

```text
1. fixed timestamp slice: bytes[0..15]
2. delimiter scan after host
3. delimiter scan for ':' after process/pid
4. message = rest
5. daemon-specific extractor for sshd/sudo/kernel/cron
```

Rough per-line work:

- 3–6 delimiter searches;
- 1 timestamp parse;
- 1 host intern lookup;
- optional process/pid parse;
- message classifier;
- optional IP/port/user extraction.

### 7.2 Normalized Shape

```rust
struct SyslogRow {
    time_ns: i64,
    host_id: u32,
    process_id: u32,
    pid: Option<u32>,
    src_ip: Option<u32>,
    src_port: Option<u16>,
    action_id: u16,
    raw_offset: u32,
    raw_len: u32,
}
```

Intern repeated values:

- host;
- process;
- username;
- action.

### 7.3 Tokenization

Generic message tokens for the example:

```text
failed
password
invalid
user
root
10.0.0.5
ssh2
```

Better Sigma/security shape:

- classify `sshd_failed_password`;
- extract `user=root`;
- extract `src_ip=10.0.0.5`;
- store event kind as enum;
- keep generic tokens as fallback.

### 7.4 Challenges and Optimizations

Challenges:

- timestamp missing year;
- locale/month parsing;
- timezone correction;
- forwarded syslog headers;
- multiline logs;
- process names with odd characters;
- IPv6;
- daemon-specific message variation.

Optimizations:

- separate RFC3164 and RFC5424 fast paths;
- detect daemon before message parser;
- specialized parsers for `sshd`, `sudo`, `kernel`, `cron`;
- byte-level integer parsing for pid/port;
- batch dictionary merges;
- avoid regex on every line; use regex only after cheap classifier hit.

---

## 8. Apache Combined Log Profile

Example:

```text
10.0.0.5 - frank [29/Apr/2026:12:34:56 -0700] "GET /admin/login?x=1 HTTP/1.1" 404 512 "https://example.com/" "Mozilla/5.0"
```

Fields:

```text
remote_ip   = 10.0.0.5
ident       = -
user        = frank
time        = 29/Apr/2026:12:34:56 -0700
method      = GET
path        = /admin/login?x=1
protocol    = HTTP/1.1
status      = 404
bytes       = 512
referer     = https://example.com/
user_agent  = Mozilla/5.0
```

### 8.1 Parse Computation

Apache combined format is delimiter-friendly:

```text
1. scan space -> remote_ip
2. scan next spaces -> ident, user
3. find '[' and ']'
4. find first quote pair -> request
5. parse request: method SP target SP protocol
6. parse status and bytes
7. parse quote pair -> referer
8. parse quote pair -> user_agent
```

Most fields can be slices into the raw line until interned or stored.

### 8.2 Normalized Shape

```rust
struct ApacheRow {
    time_ns: i64,
    remote_ip: u32,
    user_id: Option<u32>,
    method: HttpMethod,
    path_offset: u32,
    path_len: u32,
    status: u16,
    bytes: u64,
    referer_offset: u32,
    ua_id: Option<u32>,
    raw_offset: u32,
    raw_len: u32,
}

enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Other(u16),
}
```

### 8.3 Tokenization

Path:

```text
/admin/login?x=1
```

Tokens:

```text
admin
login
```

Also store:

```text
path_prefix=/admin
path_ext=None
query_keys=[x]
```

Status is a typed column, not a token. User-agent is high-cardinality and expensive; store raw/interned first, parse family only if queries require it.

### 8.4 Challenges and Optimizations

Challenges:

- escaped quotes;
- malformed request lines;
- `bytes` may be `-`;
- IPv6;
- custom proxy formats;
- timezone parse cost;
- user-agent cardinality;
- bot-generated path weirdness.

Optimizations:

- strict fast path plus tolerant slow path;
- common method byte comparisons;
- three-digit status parser;
- byte parser for IPv4;
- avoid URL decoding unless rule requires it;
- path tokenization with delimiter scanner;
- batch dictionary inserts for paths/user-agents.

---

## 9. Tokenization and Index Construction

Tokenize only useful fields by default:

- `Image`;
- `CommandLine`;
- `ParentImage`;
- `ParentCommandLine`;
- `TargetFilename`;
- `User`;
- `Computer`;
- network IP/port fields;
- hashes;
- Apache path;
- selected syslog message classes.

Do not tokenize every JSON key/value by default.

Typical event estimate:

```text
CommandLine:         50–300 bytes
Image path:          30–180 bytes
Parent image:        30–180 bytes
tokens/event:        5–40 typical
posting entry:       4–12 bytes before compression
```

For 1M events:

```text
20 tokens/event × 1M = 20M token occurrences
raw posting size ≈ 80–240 MB
compressed postings lower if sorted/delta encoded
```

Build indexes batch-local first:

```text
parse batch
  -> collect tokens into per-worker local map
  -> sort by token_id, row_id
  -> delta encode postings
  -> merge into bucket sidecar
```

Avoid global concurrent hash maps in the hot path. They destroy cache locality and introduce lock/atomic pressure exactly where throughput matters.

---

## 10. Search-Shaped Storage

For Sigma-style searches, optimize predicates before raw scan.

Bucket metadata:

```text
product/category/service
time min/max
event_id set
field presence set
row count
raw byte count
```

Hot columns:

```text
_time
event_id
host_id
user_id
image_id
parent_image_id
src_ip
dst_ip
dst_port
result/action
raw_offset
raw_len
```

Side indexes:

```text
EventID -> bitmap
field_value_id -> bitmap
token_id -> postings
field Bloom filters
raw offset table
Tantivy segment or custom term dictionary
```

Sigma query plan:

```text
Sigma rule
  -> select matching buckets
  -> bitmap field filters
  -> token prefilter
  -> raw/regex verification
```

Example Sigma-like predicate:

```yaml
selection:
  EventID: 1
  Image|endswith: '\powershell.exe'
  CommandLine|contains: '-enc'
```

Fast path:

```text
1. skip buckets without product=windows
2. bitmap filter EventID=1
3. token/suffix index for powershell.exe
4. token filter for -enc
5. raw-row verification only on candidates
```

---

## 11. Durable Commit Modes

Durable commit means the acknowledgment boundary is tied to a persistence boundary.

| Level | Ack after | Survives process crash | Survives power loss |
|---|---|---:|---:|
| memory accepted | channel enqueue | no | no |
| file write no fsync | `write` returned | usually maybe | no guarantee |
| SQLite WAL commit NORMAL | transaction commit | yes | usually; fsync timing depends on WAL/checkpoint |
| SQLite FULL/fsync | fsync barrier complete | yes | strongest |
| custom WAL fdatasync | append + `fdatasync` | yes | strong |

Recommended default:

```text
ack after durable local ingest commit
search prep lag exposed as metric
```

Metrics:

```text
ingest_commit_latency_ms
ingest_uncommitted_batches
ingest_committed_events_total
search_prep_lag_events
search_prep_lag_seconds
search_prep_oldest_unindexed_time
```

SQLite notes:

- use `BEGIN IMMEDIATE` to acquire write lock up front;
- reuse prepared statements;
- commit 10K–100K rows per transaction;
- `WAL` + `synchronous=NORMAL` for normal ingest durability;
- `FULL` only for strongest local durability mode;
- bucket-per-file creates parallelism by avoiding one global SQLite writer lock.

---

## 12. Skip Rate and Query Selectivity

Skip rate measures avoided data:

```text
bucket_skip_rate = skipped_buckets / candidate_buckets
byte_skip_rate   = skipped_bytes / candidate_bytes
```

Example:

```text
1000 buckets in time range
EventID metadata/bitmap excludes 940
token index excludes 50 more
raw scan only 10 buckets

bucket skip rate = 99%
```

Selectivity measures predicate narrowness:

```text
selectivity = matching_rows / total_rows
```

Example:

```text
EventID = 1                 maybe 5%
Image endswith powershell   maybe 0.5%
CommandLine contains -enc   maybe 0.05%
combined                    maybe 0.001%
```

Planner rules:

- apply time/logsource bucket pruning first;
- use bucket metadata before opening data files;
- intersect smallest posting lists first;
- use bitmaps before raw string checks;
- delay regex until candidate set is tiny;
- choose column scan when predicate is broad;
- choose index lookup when predicate is selective;
- report skipped buckets/bytes in query profile output.

The target is not simply faster raw scan. The target is to avoid raw scan for 95–99.9% of irrelevant data.

---

## 13. SIMD, `memchr`, and CPU Dispatch

### 13.1 `memchr`

`memchr` finds one byte in a byte slice:

```rust
memchr::memchr(b'\n', buffer)
```

Use cases:

- newline framing;
- finding spaces, tabs, quotes, `]`, `:`, `=`;
- cheap candidate discovery before deeper parsing.

Current locked version in this repo: `memchr 2.8.0` in `Cargo.lock`.

Local source path after fetch:

```text
~/.cargo/registry/src/*/memchr-2.8.0/src/
```

Architecture dispatch:

- module selection in `src/arch/mod.rs`;
- `aarch64` path in `src/arch/aarch64/memchr.rs`;
- `x86_64` path in `src/arch/x86_64/memchr.rs`.

For ARM Mac (`aarch64`), the crate chooses NEON at compile time when `target_feature = "neon"`, otherwise fallback.

For x86 Linux (`x86_64`), the crate does runtime dispatch: first call detects CPU support, chooses AVX2 if available, otherwise SSE2, otherwise fallback, then caches the function pointer.

Commands:

```bash
cargo tree -i memchr
rustc -vV
rustc --print cfg | rg 'target_arch|target_feature'
```

Assembly/profiling:

```bash
cargo install cargo-asm
cargo asm memchr
objdump -d target/release/spank | rg -i 'memchr|avx|sse|neon'
perf record ./target/release/spank ...
perf report
```

### 13.2 SIMD JSON and String Scanning

SIMD parser candidates:

- `serde_json`: stable baseline;
- `simd-json`: SIMD structural scanner, best when input can be mutable bytes;
- `sonic-rs`: very fast JSON parser, less established than `serde_json`.

SIMD/string opportunities:

- line framing via `memchr`;
- fixed literal matching via `aho-corasick`;
- JSON structural scanning;
- UTF-8 validation;
- ASCII lowercase/compare for Windows paths;
- vectorized delimiter scans;
- compression libraries such as zstd/zlib-ng.

Avoid handwritten assembly first. Prefer crates that already dispatch to AVX2/AVX-512/NEON. Hand assembly is justified only after profiles show one tiny loop dominates.

---

## 14. Code Structure and Optional Optimized Paths

Use traits and profile modules so optimized implementations are drop-in replacements.

```rust
trait LineParser {
    type Row;

    fn parse_line<'a>(
        &self,
        line: &'a [u8],
        scratch: &mut Scratch,
    ) -> Result<Self::Row, ParseError>;
}

trait Normalizer<R> {
    fn normalize(&self, row: R, batch: &mut NormalizedBatch);
}

trait Tokenizer {
    fn tokenize(&self, batch: &NormalizedBatch, out: &mut TokenBatch);
}

trait StoreWriter {
    fn append_batch(&mut self, batch: &NormalizedBatch) -> Result<CommitId>;
}

trait SearchPrep {
    fn prepare(&mut self, batch: &NormalizedBatch) -> Result<IndexDelta>;
}
```

Suggested crate/module shape:

```text
spank-ingest
  framing/
    newline.rs
    hec.rs
  parsers/
    syslog.rs
    apache.rs
    json.rs
  normalize/
    dictionaries.rs
    schema.rs
  store/
    sqlite_hot.rs
    raw_log.rs
  search_prep/
    tokenize.rs
    bloom.rs
    bitmap.rs
    tantivy.rs
  optimized/
    simd_json.rs
    memchr_scan.rs
    avx2_ascii.rs
```

Feature-selected implementations:

```rust
#[cfg(feature = "simd-json")]
mod json_simd;

#[cfg(not(feature = "simd-json"))]
mod json_serde;
```

Runtime CPU dispatch:

```rust
if is_x86_feature_detected!("avx2") {
    parse_avx2(input)
} else {
    parse_scalar(input)
}
```

Rules:

- same parser output;
- same error semantics;
- same metrics;
- same golden tests;
- optimized path must be a drop-in replacement;
- scalar path remains the correctness reference.

Testing:

- golden parse fixtures;
- malformed line corpus;
- property tests comparing scalar vs optimized;
- fuzz parser inputs;
- per-profile benchmarks;
- query-result equivalence tests across index backends.

---

## 15. Vector Comparison

Vector carries a broad stack because it solves a broad problem: many sources, transforms, sinks, cloud integrations, Kubernetes, GraphQL API, gRPC, WebSockets, OpenSSL enterprise compatibility, VRL, on-disk buffers, and broad parser coverage.

Spank's smaller-stack advantage is not raw Rust minimalism. It is a product constraint:

- one or few input protocols first;
- one local store design;
- one search-shaped data model;
- one query planner;
- fewer protocol versions and framework migrations;
- fewer optional enterprise/cloud connectors in the core binary.

Spank can beat Vector on:

- ingest-to-local-index latency for known log profiles;
- Sigma/SPL query latency over local buckets;
- dependency surface;
- durability semantics tied to local searchable storage;
- operator reasoning and auditability.

Vector wins on:

- connector breadth;
- mature routing/transform graph;
- ecosystem integrations;
- production hardening across many topologies.

Correct framing: use Vector when routing/transforms/sinks are the product. Build Spank when local Splunk-shaped indexing/search is the product.

---

## 16. Benchmark Protocol

Every performance claim should eventually point to a fixture and command.

Datasets:

- 10 GB syslog RFC3164/RFC5424 mix;
- 10 GB Apache combined logs;
- 100 GB Sysmon/EDR NDJSON;
- malformed corpus for slow path;
- Sigma-derived query set.

Metrics:

```text
ingest_events_per_sec
ingest_bytes_per_sec
parse_ns_per_event
normalize_ns_per_event
tokenize_ns_per_event
commit_latency_ms p50/p95/p99
RSS
allocations/event
bytes_written/input_byte
search_prep_lag_seconds
query_latency_ms p50/p95/p99
bucket_skip_rate
byte_skip_rate
rows_verified/query
```

Commands to standardize:

```bash
cargo build --release
cargo bench
perf stat -d ./target/release/spank bench
perf record ./target/release/spank ...
flamegraph ./target/release/spank ...
```

Benchmark matrix:

| Dimension | Values |
|---|---|
| input | file, HEC batch stream |
| parser | scalar, SIMD/optimized |
| storage | raw file, SQLite hot, Parquet warm |
| commit | memory, WAL NORMAL, FULL/fsync |
| index | none, metadata only, bitmap/token, Tantivy |
| query | broad scan, selective Sigma, regex final verification |

Regression gates:

- hot ingest regression >20% requires investigation;
- query skip-rate regression >10 percentage points requires investigation;
- p95 commit latency regression >25% requires investigation;
- correctness failures between scalar/optimized paths block merge.

---

## Appendix A. Schema, Interpretation, and Replay

This appendix keeps schema and field-organization decisions beside the performance plan until they deserve a separate `Schema.md`. The core rule: raw input is the immutable source of truth; every richer field, token, alias, and search index is a replayable interpretation of that raw input.

### A.1 Separate Interpretation and Index Workers

The ingest path should commit raw events before expensive parsing, tokenization, decoding, enrichment, or search-index construction.

```text
input reader
  -> event breaker
  -> bounded raw staging
  -> raw chunk writer
  -> raw manifest commit
  -> interpretation queue
  -> parser/analyzer workers
  -> field dictionaries + column builders
  -> token/posting builders
  -> searchable segment manifest publish
```

This separates two durability moments:

- **raw-durable** means the original event bytes and minimum metadata are safely committed;
- **search-visible** means derived fields, tokens, and indexes have been built and published.

For HEC-style acknowledgement semantics, Spank must decide which moment counts as committed. Internally it should track both because raw durability protects against data loss while search visibility protects user expectations.

### A.2 Bounded Staging

Bounded staging is the finite buffer between input and durable/raw processing. It is a deliberate pressure valve, not an unbounded queue.

Useful bounds:

- maximum staged bytes;
- maximum staged events;
- maximum per-source backlog;
- maximum oldest-event age;
- maximum interpretation lag behind raw commit.

Behavior when a bound is hit must be explicit:

- file tailing can pause reads or record lag;
- HEC can return busy/retry;
- batch import can throttle or fail visibly;
- search can report `indexing_lag_seconds` for recently ingested raw chunks.

The goal is to avoid the worst failure mode: accepting data, dropping it silently, and still reporting success.

### A.3 Raw Event Table and Chunk Format

The minimal durable store should keep `_raw` bytes in append-only chunks and keep event metadata as offsets into those chunks.

Example metadata tables:

```sql
raw_chunks(
  chunk_id          INTEGER PRIMARY KEY,
  path              TEXT NOT NULL,
  codec             TEXT NOT NULL,
  min_event_id      INTEGER NOT NULL,
  max_event_id      INTEGER NOT NULL,
  min_time_ns       INTEGER,
  max_time_ns       INTEGER,
  byte_len          INTEGER NOT NULL,
  crc32c            INTEGER NOT NULL,
  created_at_ns     INTEGER NOT NULL
);

raw_events(
  event_id          INTEGER PRIMARY KEY,
  chunk_id          INTEGER NOT NULL,
  raw_offset        INTEGER NOT NULL,
  raw_len           INTEGER NOT NULL,
  ingest_time_ns    INTEGER NOT NULL,
  event_time_ns     INTEGER,
  host_id           INTEGER,
  source_id         INTEGER,
  sourcetype_id     INTEGER,
  parser_version_id INTEGER,
  flags             INTEGER NOT NULL,
  raw_hash64        INTEGER NOT NULL
);
```

Example chunk layout:

```text
SpankRawChunk v1
header:
  magic, version, chunk_id, codec, record_count, payload_len, crc32c
payload:
  [len varint][raw bytes]
  [len varint][raw bytes]
  ...
offset sidecar:
  event_ordinal -> payload byte offset
```

`codec` names how the payload is encoded: `raw`, `zstd`, `lz4`, or another compression/framing choice. Store it per chunk because different tiers can use different codecs: hot chunks may stay raw or lightly compressed; warm chunks may use stronger compression.

`crc32c` is a cheap corruption check for chunks and manifests. It is not a security hash. It catches torn writes, disk corruption, bad copies, and codec bugs before search trusts a chunk. Keep a stronger optional hash, such as BLAKE3, if tamper evidence or cross-machine content identity becomes important.

#### A.3.1 CRC Failure Model, Mitigation, and Validation

CRC belongs at the raw chunk layer because raw chunks are the replay source for every future parser, decoder, token index, and schema version. If raw bytes or raw offsets are wrong, every downstream index can become confidently wrong. The corruption check should therefore sit before interpretation, before replay, and before search-time raw verification.

Realistic CRC-detected failure instances:

| Failure instance | Where it can occur | What CRC catches | Mitigation |
|---|---|---|---|
| Torn chunk write | process crash, OS crash, power loss during append | payload length or checksum mismatch on reopen | write to temporary chunk, `fsync`, then atomically publish manifest |
| Partial copy or restore | backup, rsync, object-store upload/download, local file copy | copied file shorter or different from manifest length/checksum | verify chunks after copy; quarantine bad chunk; retry from source |
| Wrong manifest points to wrong file | manifest update bug, operator copy, stale symlink | `chunk_id`, length, or CRC mismatch | include chunk id/version in header; validate manifest-to-header consistency |
| Codec/decodec bug | zstd/lz4 upgrade, wrong frame options, implementation mistake | decompressed payload does not match stored checksum | checksum uncompressed canonical payload or store both compressed and uncompressed checks |
| Offset sidecar mismatch | writer bug, compaction bug, crash between payload and offsets | event offsets point outside payload or into wrong record lengths | checksum sidecar separately; validate monotonic offsets and record count |
| Silent disk or filesystem corruption | storage media, controller, filesystem, VM image | chunk bytes differ from recorded CRC | periodic scrub; mark affected segments unavailable; rebuild from replica or source |
| Memory scribble before flush | unsafe code bug, buffer reuse bug, FFI bug | bytes written differ from expected finalized buffer | compute CRC after final buffer assembly; avoid mutating sealed buffers |
| Concatenated/truncated segment during compaction | compactor crash or bug | merged chunk checksum or record count mismatch | build compacted chunk under temp name; verify fully before manifest switch |
| Network transfer corruption | remote ingest, replication, backup transport | received chunk checksum mismatch | length-prefixed transfer plus end-to-end chunk checksum; retry |
| Stale cache or stale mmap view | mmap reuse, file replacement, filesystem cache edge cases | header/checksum mismatch at open or scrub | immutable filenames; no in-place overwrite; reopen by manifest generation |

CRC does not solve:

- malicious tampering by an attacker who can rewrite both payload and CRC;
- semantic parser bugs that extract the wrong fields from valid bytes;
- incomplete but internally consistent data accepted before a crash boundary;
- duplicate events or missing source-side lines before Spank receives them.

Use stronger hashes where the requirement changes:

- `crc32c` for fast corruption detection inside a trusted local store;
- `xxh3` or similar for very fast non-cryptographic fingerprints;
- `blake3` for content identity, replica verification, and tamper-evident manifests;
- `sha256` when interop or compliance demands a conventional cryptographic hash.

Validation plan:

1. Unit-test chunk round trips for every codec: write, close, reopen, verify header, CRC, count, and offsets.
2. Flip one byte in the payload and assert open/replay fails with `chunk_corrupt`.
3. Truncate the final byte, middle record, and header and assert distinct diagnostics.
4. Corrupt the offset sidecar and assert offset validation fails before returning raw bytes.
5. Corrupt the manifest path, `chunk_id`, `byte_len`, and checksum independently.
6. Crash-injection test: kill writer before manifest publish; orphan temp chunk must not become visible.
7. Crash-injection test: kill after chunk fsync but before manifest switch; reopen must either see old manifest or complete new manifest, never half state.
8. Codec test: compress/decompress each chunk and verify the canonical uncompressed payload checksum.
9. Compaction test: merge small chunks, verify merged count/ranges/checksum, then compare replayed raw events byte-for-byte.
10. Scrub command: periodically scan manifests and chunks, reporting corrupt, missing, orphaned, and stale files.

### A.4 EAV, Column Families, and Per-Field Segments

There are two useful implementation shapes.

EAV means entity-attribute-value: each field value is a row keyed by event and field.

```sql
field_values(
  event_id   INTEGER NOT NULL,
  field_id   INTEGER NOT NULL,
  value_id   INTEGER,
  typed_i64  INTEGER,
  typed_f64  REAL,
  typed_text TEXT
);
```

EAV advantages:

- easy to add fields without SQL migrations;
- simple to inspect and debug;
- good for sparse, early-stage parser output;
- compatible with SQLite prototypes.

EAV disadvantages:

- many rows per event;
- poor cache locality for hot predicates;
- index size grows quickly;
- query speed depends heavily on secondary indexes.

Per-field segment files store each hot field in its own typed layout.

```text
segment-000042/
  manifest.toml
  raw.ref
  columns/event_time_ns.i64
  columns/status.u16
  columns/source_ip.u32
  dictionaries/user.dict
  exact/status.postings
  exact/process.postings
  token/message.postings
  token/url_path.postings
```

Per-field segment advantages:

- contiguous arrays for SIMD scans and cache locality;
- field-specific compression;
- easy skip metadata per field;
- adding a field adds sidecar files rather than altering one giant table;
- well matched to query planners and Sigma-style selective predicates.

Per-field segment disadvantages:

- more manifest/version engineering;
- harder manual inspection;
- segment publish and compaction must be correct.

Recommended path: use EAV/SQLite for correctness prototypes and tests, but design the Rust performance path around immutable per-field segment files.

### A.5 Field Registry, Aliases, and Views

Use one internal canonical field registry and expose aliases for different ecosystems. Do not duplicate physical storage for every naming convention.

Example canonical fields:

```text
event.time
host.name
source.path
sourcetype
process.name
process.pid
user.name
source.ip
source.port
destination.ip
destination.port
http.method
url.original
url.path
url.query
http.status_code
http.user_agent
message
```

Example view mapping:

| Concept | Internal | Splunk/CIM-ish | ECS | OTel | Sigma-oriented aliases |
|---|---|---|---|---|---|
| Source IP | `source.ip` | `src`, `src_ip` | `source.ip` | `network.peer.address` | `SourceIp`, `src_ip` |
| Destination IP | `destination.ip` | `dest`, `dest_ip` | `destination.ip` | `network.local.address` | `DestinationIp`, `dst_ip` |
| Source port | `source.port` | `src_port` | `source.port` | `network.peer.port` | `SourcePort` |
| Destination port | `destination.port` | `dest_port` | `destination.port` | `network.local.port` | `DestinationPort` |
| HTTP method | `http.method` | `http_method`, `method` | `http.request.method` | `http.request.method` | `cs-method`, `HttpMethod` |
| URL path | `url.path` | `uri_path` | `url.path` | `url.path` | `cs-uri-stem`, `UrlPath` |
| HTTP status | `http.status_code` | `status` | `http.response.status_code` | `http.response.status_code` | `sc-status`, `StatusCode` |
| User | `user.name` | `user` | `user.name` | `enduser.id` | `User`, `TargetUser` |
| Process | `process.name` | `process` | `process.name` | `process.executable.name` | `Image`, `ProcessName` |
| Message | `message` | `message` | `message` | `Body` | `Message` |

The registry should record:

- field id;
- canonical name;
- aliases;
- type;
- source sourcetypes;
- parser/analyzer producer;
- index policy;
- supported operators;
- replay version;
- deprecation status.

### A.6 Common Field Lists by Log Type

Minimum common fields for every event:

- `event.id`;
- `event.time`;
- `ingest.time`;
- `host.name`;
- `source.path`;
- `sourcetype`;
- `_raw` reference;
- `parse.status`;
- `parser.version`.

Syslog fields:

- `process.name`;
- `process.pid`;
- `message`;
- `syslog.priority`;
- `syslog.facility`;
- `syslog.severity`;
- `systemd.unit`.

Linux auth fields:

- `event.action`;
- `auth.method`;
- `user.name`;
- `source.ip`;
- `source.port`;
- `pam.service`;
- `pam.type`;
- `sudo.user`;
- `sudo.runas_user`;
- `process.command_line`;
- `session.id`;
- `disconnect.reason`.

Apache access fields:

- `client.ip` or canonical `source.ip`;
- `user.name`;
- `http.method`;
- `url.original`;
- `url.path`;
- `url.query`;
- `network.protocol.version`;
- `http.status_code`;
- `http.response.bytes`;
- `http.referrer`;
- `http.user_agent`;
- `event.duration_us`.

Apache error fields:

- `apache.module`;
- `log.level`;
- `process.pid`;
- `apache.error_code`;
- `client.ip`;
- `client.port`;
- `http.method`;
- `url.original`;
- `message`.

### A.7 Replayable Deep Parsing

Deeper parsing is replayable when derived facts can be rebuilt from durable raw chunks after a parser, analyzer, decoder, or schema changes.

Replayable examples:

- URL percent-decoding;
- Log4Shell lookup-obfuscation detection;
- Apache `LogFormat` parser upgrades;
- richer `auth.log` SSH/PAM/sudo extraction;
- user-agent parsing;
- GeoIP enrichment;
- Sigma-specific field normalization;
- tokenization changes.

Replay requires:

- raw bytes retained;
- sourcetype and source metadata retained;
- parser/analyzer versions recorded;
- schema registry versions recorded;
- old and new segment manifests coexisting during rebuild;
- atomic manifest switch when rebuild completes.

Search can then choose old or new interpretations explicitly, or default to the newest complete manifest.

### A.8 Capability Metadata

Capability metadata tells the query compiler, Sigma compiler, and user interface what is cheap, expensive, or unsupported.

Example:

```toml
[sourcetype.access_combined]
parser = "apache_access@3"
schema_view = "spank@1"

[field.url.path]
type = "string"
aliases = ["uri_path", "url.path", "cs-uri-stem"]
operators = ["eq", "contains", "prefix", "suffix", "regex"]
indexes = ["exact", "token", "raw_verify"]
replayable = true

[field.http.status_code]
type = "u16"
aliases = ["status", "http.response.status_code", "sc-status"]
operators = ["eq", "in", "range"]
indexes = ["numeric", "bitmap"]
replayable = true

[field.source.ip]
type = "ip"
aliases = ["src", "src_ip", "source.ip", "SourceIp"]
operators = ["eq", "in", "cidr"]
indexes = ["ip_exact", "ip_cidr"]
replayable = true
```

This supports clear rule planning:

```text
Sigma rule references url.path contains "../"
  -> supported by token/raw verification

Sigma rule references process.command_line on apache_access
  -> unsupported for this sourcetype

Sigma rule references source.ip in CIDR range
  -> supported by IP typed index
```

### A.9 References to Keep Attached

Useful external references for this appendix:

- Splunk default fields and sourcetypes;
- Splunk Common Information Model field lists;
- Elastic Common Schema;
- OpenTelemetry logs and semantic conventions;
- Sigma rule specification, taxonomy, and backend mappings;
- Vector VRL parser functions;
- Apache LogFormat directives;
- RFC 3164 and RFC 5424 syslog;
- Log4j lookup documentation for Log4Shell-style obfuscation.
