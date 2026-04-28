# Indust — Storage, Query, and Industry Context

`Focus: research` — analytical input covering storage backend alternatives, the SPL
query model, the missing Partition layer, Splunk's on-disk formats, and a curated
reading list of external resources. The audience is a developer working on the
ingest pipeline, storage rotation, or SPL execution, and any model session picking
up that work. This document does not receive work items (those belong in `Plan.md`)
or subsystem contracts (those belong in the Reference docs under `docs/`).

This document is updated when session findings introduce new analytical input that
is not yet embodied in code — specifically when a discussion surfaces a design
constraint, a terminology clarification, or an external reference that would
otherwise have to be re-derived next session. The companion research documents are
`research/Pyst.md` (Python/Rust comparison), `research/Stast.md` (coding standards),
and `research/Infrust.md` (infrastructure). The fresh implementation proposal is at
`docs/Sparst.md`.

---

## Table of Contents

1. [Terms that required follow-up in past sessions](#1-terms-that-required-follow-up-in-past-sessions)
2. [The architectural gap: Partition layer](#2-the-architectural-gap-partition-layer)
3. [SQLite concurrency and the bucket-per-file pattern](#3-sqlite-concurrency-and-the-bucket-per-file-pattern)
4. [Storage backend alternatives](#4-storage-backend-alternatives)
5. [Splunk on-disk formats](#5-splunk-on-disk-formats)
6. [SPL functional requirements](#6-spl-functional-requirements)
7. [Industry and technology reading list](#7-industry-and-technology-reading-list)

---

## 1. Terms that required follow-up in past sessions

This section expands the concepts that recurred across sessions and needed
definition before the surrounding analysis could proceed. The intent is to
front-load these so future sessions do not spend time re-deriving them.
"Follow-up" means a reader asked for clarification or the discussion stalled until
the term was unpacked. Terms that are obvious (JSON, HTTP, SQL, gzip, async/await,
Mutex) are not listed here.

**arc-swap.** A Rust crate (`arc-swap`) providing a lock-free `Arc<T>` replacement
for values that are read on every hot-path request. `ArcSwap<T>::load()` costs one
atomic memory load — no system call, no kernel involvement, no Mutex contention.
The return type is `Guard<Arc<T>>`; two `*` dereferences are needed to reach the
inner `T`: the first unwraps `Guard`, the second unwraps `Arc`. In spank-rs `HecPhase`
is stored in an `ArcSwap` so that every inbound HEC request can read the current
phase without any locking overhead. A writer calling `.store()` swaps the pointer
atomically; readers racing with the swap see either the old or new value, both
valid. See `spank-core/src/phase.rs` and `Pyst.md §3` for the axum context.

**`BEGIN IMMEDIATE`.** One of three SQLite transaction-start modes (the others being
`DEFERRED` and `EXCLUSIVE`). `BEGIN IMMEDIATE` acquires the write lock at transaction
start rather than at the first write statement. The practical effect: a writer
discovers lock contention immediately and can fail fast or retry, rather than
discovering it mid-transaction after staging partial work. In spank-rs
`SqliteWriter::append()` uses `BEGIN IMMEDIATE` so that a second concurrent writer on
the same `.db` file fails at transaction open, not mid-insert. This is critical for
the bucket-per-file pattern: each bucket has its own `.db` file and its own WAL,
so `BEGIN IMMEDIATE` contention only occurs between writers targeting the same bucket.

**`prepare_cached`.** The rusqlite method that returns a cached prepared statement
handle from the connection's internal cache, keyed by the SQL string. Without it,
each call to `append()` would re-parse, re-plan, and re-compile the INSERT SQL on
every invocation. With it, the parse and plan cost is paid once; subsequent calls
retrieve the compiled form. The difference on a 100k-row bulk insert is measurable:
prepare overhead on each row shifts the bottleneck from the disk write to the
SQL round-trip. See `SqliteWriter::append()` in `spank-store/src/sqlite.rs`.

**WAL (Write-Ahead Log).** SQLite's `journal_mode = WAL` changes the durability
mechanism from rollback-journal to write-ahead log. The key operational property:
readers never block writers and writers never block readers. Multiple concurrent
readers can proceed while one writer is active. However, only one writer can hold
the WAL write lock at a time — WAL does not give SQLite MVCC (see below). For
spank-rs the implication is: WAL mode enables high read throughput during
concurrent ingest, but does not solve the single-writer bottleneck per `.db` file.
The solution is multiple files, not WAL mode alone.

**MVCC (Multi-Version Concurrency Control).** The mechanism PostgreSQL and other
RDBMS use to allow concurrent writers without blocking. Each row carries version
metadata (`xmin`/`xmax` in Postgres); a transaction sees a consistent snapshot of
the data as of its start time, ignoring rows written by concurrent transactions.
Writers do not acquire row locks for reads. SQLite WAL does not implement MVCC in
this sense: it provides reader-writer non-blocking, but a second writer blocks until
the first commits.

**Bucket (Splunk term).** A directory on disk that holds event data for one
continuous time range. Not a SQL concept. In Splunk a bucket contains: a compressed
event stream (`rawdata/journal.gz`), a byte-offset table (`rawdata/slices.dat`), an
inverted term index (`tsidx` files), bloom filters, and a metadata file
(`metadata/default.meta`). In spank-rs a bucket is mapped to one SQLite `.db` file.
The Splunk bucket has four lifecycle states: hot (currently written), warm (complete,
searchable), cold (moved to slower storage, still searchable), frozen (archived or
deleted, not searchable). Rotation from hot to warm is triggered by age, size, or
event count.

**Partition (architectural term).** The layer above a single bucket that manages
the set of hot, warm, and cold buckets for one named index. The Partition is
responsible for: routing incoming writes to a hot bucket, rotating a hot bucket to
warm when a threshold is crossed, fanning out a search query to all buckets whose
time range overlaps the query window, and recovering from crash by scanning the
bucket directory at startup. In spank-py this is `IndexPartition`; in spank-rs the
`PartitionManager` trait exists but no implementation coordinates multiple buckets.
See `§2` for a full treatment.

**tsidx.** Splunk's per-bucket inverted term index. Maps each term (word from the
raw event text) to a sorted list of `(event_time, rawdata_byte_offset)` pairs. At
query time, a keyword search first looks up the term in the tsidx to get a candidate
offset list, then reads only those offsets from `rawdata/journal.gz` via the byte
positions in `slices.dat`. Searching without tsidx means a full scan of the
compressed event stream. Bloom filters (one per bucket) allow a keyword search to
skip the tsidx lookup entirely when the term is definitely absent from the bucket.
There is no public Splunk specification for the tsidx binary format; the description
above is from reverse-engineering references cited in `§7`.

**`_cd` field.** A cursor field added to each indexed event by spank-py's bucket
implementation. Its value is `bucket_id:sqlite_rowid` — for example `"bench:4217"`.
This allows stable pagination across SPL result pages: a client supplies the last
`_cd` seen, and the next query starts from `rowid > 4217` in bucket `bench`. Without
a cursor field, offset-based pagination (`LIMIT N OFFSET M`) is unstable when new
rows are inserted between pages.

**`dc` in SPL stats.** Distinct Count. `dc(field)` returns the number of unique
values of `field` in the result set, equivalent to SQL `COUNT(DISTINCT field)`. Not
obvious from the abbreviation in Splunk documentation.

**`BytesMut::split_to`.** The bytes crate operation that splits a `BytesMut` buffer
at a given index, returning the bytes before the index as a new `BytesMut` and
leaving the buffer starting at the index. The key property is zero-copy: no memory
is copied; only a pointer and length are adjusted. This is how spank-rs's TCP
receiver (`spank-tcp/src/receiver.rs`) extracts complete lines from the read buffer
without copying: `buf.split_to(pos + 1)` gives the line bytes, and the buffer now
starts at the next line.

**`from_utf8_lossy`.** The standard library function `String::from_utf8_lossy(&[u8])`
that converts a byte slice to a string, replacing any invalid UTF-8 sequences with
the Unicode replacement character (U+FFFD) rather than returning an error. The return
type is `Cow<str>` — a borrowed `&str` if the input is valid UTF-8 (no allocation),
or an owned `String` if replacement characters were inserted (one allocation). In
the TCP receiver this is called on every line; for well-formed UTF-8 log data the
`Cow::Borrowed` path is taken and `.into_owned()` converts it to a `String` for
sending. The cost of that final allocation is unavoidable because the line must
outlive the buffer it was split from.

**Lateral join (SQL).** A join form where the right-side expression can reference
columns from the current left-side row — effectively a correlated subquery in the
`FROM` clause. The relevance to SPL: the `fields` column in spank-rs stores a JSON
object (`fields_json TEXT`). Expanding those fields into columns for a `stats` or
`eval` command requires either a lateral join against a JSON-parsing table function,
or application-level expansion. SQLite 3.38+ supports the `json_each()` and
`json_extract()` functions; full lateral join support (`LATERAL` keyword) is not
present in SQLite as of 3.45.

---

## 2. The architectural gap: Partition layer

The most significant structural gap in spank-rs relative to both spank-py and
Splunk's own architecture is the absence of a Partition layer. This section
describes what the Partition layer is, what spank-py has, and what spank-rs needs.

### 2.1 What the Partition layer does

A Partition manages one named index (e.g., `main`, `security`, `_internal`). It
owns the set of bucket files for that index and is the single point through which
writes and reads pass. Its responsibilities are:

Write routing: all inbound rows for an index go to the current hot bucket. When
the hot bucket crosses a threshold (age, byte size, or event count), the Partition
closes it, promotes it to warm, and opens a new hot bucket.

Scatter-gather reads: a search with a time range may overlap multiple buckets. The
Partition fans the query out to every bucket whose time range intersects the query
window, collects the results, and merges them (typically by `time_event_ns`).

Crash recovery: on startup, the Partition scans the bucket directory, opens each
`.db` file, reads the min and max `time_event_ns` values, and rebuilds the in-memory
bucket registry. No separate catalog file is required; the bucket files are the
ground truth.

Retention enforcement: the Partition deletes or archives cold/frozen buckets that
have exceeded their configured retention age.

### 2.2 What spank-py has

`spank-py/src/spank/indexing/indexer.py` has `IndexPartition` (the Partition) and
`IndexWriter` (a thread that owns one hot bucket and holds the batch accumulation
state). `IndexWriter` batches rows by size and timeout, then calls
`IndexPartition.add_events()`, which routes to the current hot bucket and rotates
when thresholds are crossed. Multiple `IndexWriter` threads target different indexes;
each index has its own `IndexPartition` and its own hot bucket. Concurrency is
achieved by thread isolation, not by locking.

### 2.3 What spank-rs needs

The `PartitionManager` trait in `spank-store/src/traits.rs` provides `create_hot`,
`open_reader`, and `list`. That is a bucket-level interface, not a Partition-level
interface. What is missing is the coordinator that:

- Holds one hot `BucketWriter` per active index.
- Rotates the hot writer when `cfg.bucket.max_events` or `cfg.bucket.max_age_secs`
  is exceeded.
- On search, calls `list()`, opens each matching reader, calls `scan_time_range`,
  and merges.
- On startup, scans the directory and rebuilds the registry.

Until this layer exists, the HEC ingest path uses `FileSender` (JSON-lines files,
not SQLite), because `FileSender` requires no Partition coordination. The
`SqliteBackend` is wired only in the `bench` subcommand. This is tracked as an open
item: `docs/Sparst.md §4` describes the target persistence design.

---

## 3. SQLite concurrency and the bucket-per-file pattern

SQLite allows exactly one writer at a time per `.db` file, even in WAL mode. The
WAL write lock is a process-wide mutex per file. This is not a defect; it is a
deliberate design choice that keeps the implementation simple and the correctness
guarantees strong.

The implication for spank-rs: if all indexes share one `.db` file, all ingest
threads block behind each other. The solution used by spank-py — and the target for
spank-rs — is one `.db` file per hot bucket. Each bucket has its own WAL lock;
ingest threads for different indexes never contend. Within a single index there is
one hot bucket at a time, so there is at most one writer per index.

The write throughput target from the bench subcommand is approximately 200k–500k
inserts per second on a laptop-class machine (release build, `BEGIN IMMEDIATE`,
`synchronous = NORMAL`, `journal_mode = WAL`, no `fsync` per row). This is
sufficient for most single-node ingest workloads. If higher throughput is needed,
the path is batch accumulation (stage rows in memory, flush in bulk), not switching
backends.

The three alternative backends considered are discussed in `§4`. The conclusion from
that analysis is that the bucket-per-file pattern solves the concurrency problem for
the current scale target; switching to DuckDB or PostgreSQL solves a different
problem (analytics query performance) and introduces new constraints.

---

## 4. Storage backend alternatives

This section records the analysis of DuckDB, PostgreSQL, and MySQL as alternatives
to SQLite, and the conclusion for each.

### 4.1 DuckDB

DuckDB is an embedded OLAP database designed for analytical queries on large
datasets. Its strengths are vectorized column-wise execution, native Parquet and
Arrow support, and the ability to query across multiple files with glob patterns
(`FROM 'warm/**/*.parquet'`). Its constraint is a single-writer process: DuckDB
uses a file-level write lock, not a per-row MVCC. For concurrent ingest, DuckDB has
the same per-file single-writer limitation as SQLite.

DuckDB is not a replacement for SQLite in the hot-write path. It is a strong
candidate as a read-query layer over warm and cold buckets if those are stored as
Parquet files rather than SQLite files. The workflow would be: hot bucket = SQLite
(fast append), warm bucket rotation = convert to Parquet, analytics queries = DuckDB
over the Parquet files. The Rust bindings are the `duckdb` crate.

### 4.2 PostgreSQL

PostgreSQL provides full MVCC, concurrent writers, and a mature extension
ecosystem. Its strengths are correctness under concurrent load and the `json` /
`jsonb` types for structured field storage. Its constraints for spank-rs are
operational complexity (an external process, connection pooling, schema migrations)
and per-row insert overhead that is an order of magnitude higher than SQLite for
bulk ingest workloads. The `tokio-postgres` and `sqlx` crates provide async Rust
bindings.

PostgreSQL becomes attractive if spank-rs needs to be deployed in a horizontally
scaled ingest cluster where multiple writer processes share one storage backend.
For single-node deployment the operational overhead is not justified.

### 4.3 MySQL

MySQL has similar operational profile to PostgreSQL. Its InnoDB storage engine
provides row-level MVCC. The Rust ecosystem has `sqlx` with MySQL support. The
analytical query capabilities are weaker than PostgreSQL and far weaker than
DuckDB. MySQL is not recommended for spank-rs.

### 4.4 Parquet as a first-class storage format

Parquet is a columnar, immutable, compressed file format designed for analytics.
Its key properties: column pruning (a query that reads only `time_event_ns` and
`source` does not decompress `raw`), row-group min/max statistics (a time-range
query can skip entire row groups without decompression), and broad ecosystem support
(DataFusion, DuckDB, Spark, BigQuery all read Parquet natively).

The target architecture for spank-rs warm/cold storage is: SQLite hot bucket
rotates to a Parquet file. Analytical SPL queries (`stats`, `timechart`, `dedup`)
run against Parquet via DataFusion or DuckDB. The Rust crates are `arrow` and
`parquet` (Apache Arrow Rust implementation), `datafusion` (SQL query engine over
Arrow/Parquet), and optionally `duckdb` for ad-hoc queries. This is a roadmap item;
no implementation exists yet.

---

## 5. Splunk on-disk formats

This section records what is known about Splunk's internal storage formats. There
is no public specification; the description below is assembled from reverse-engineering
references listed in `§7.4`.

### 5.1 Bucket directory layout

A Splunk bucket is a directory on the indexer's filesystem. The directory name
encodes the bucket's time range and sequence number. Inside, the canonical layout is:

```
db_<earliest>_<latest>_<sequence>/
  rawdata/
    journal.gz          # compressed, concatenated raw events
    slices.dat          # byte-offset table: event N starts at offset M in journal.gz
  <name>.tsidx          # inverted term index
  bloomfilter           # per-bucket Bloom filter for the term index
  metadata/
    default.meta        # plaintext key-value: minTime, maxTime, eventCount, etc.
```

The `<earliest>` and `<latest>` fields in the directory name are Unix timestamps in
seconds. The `metadata/default.meta` file is a plain-text KV store with fields like:

```
minTime = 1700000000
maxTime = 1700003600
eventCount = 42317
isReadOnly = 0
```

### 5.2 rawdata format

`rawdata/journal.gz` is a gzip stream containing raw event text, one event per
logical record, separated by null bytes or record markers (the exact separator
varies by Splunk version and is not public). `rawdata/slices.dat` is a binary table
where entry N contains the compressed byte offset in `journal.gz` where event N
begins. This allows random access to a specific event by decompressing a slice of
`journal.gz` starting at the given offset.

### 5.3 tsidx format

The tsidx (time-series index) file is Splunk's inverted term index. For each term
extracted from raw event text (after tokenization: splitting on whitespace and
punctuation, case-folding, stripping common stop words), the tsidx stores a posting
list: a sorted array of `(time, rawdata_offset)` pairs. At search time, the query
engine looks up each keyword in the tsidx, intersects the posting lists for
multi-keyword queries, and uses the offsets to retrieve matching raw events from
`journal.gz` via `slices.dat`.

The SQLite FTS5 extension provides equivalent functionality within a single SQLite
`.db` file. FTS5 creates an inverted index over a designated TEXT column, supports
prefix queries, phrase queries, and BM25 relevance ranking. For spank-rs this is the
lowest-friction path to keyword search: add a `CREATE VIRTUAL TABLE events_fts USING
fts5(raw, content=events, content_rowid=id)` to the schema and populate it alongside
the main `events` table. The trade-off is that FTS5 roughly doubles the storage
footprint of the events table.

### 5.4 Bloom filter

Each bucket contains a Bloom filter that covers all terms in that bucket. At search
time, the query engine tests each search term against the Bloom filter before
consulting the tsidx. A negative result (term definitely absent) allows the entire
bucket to be skipped without reading the tsidx. A positive result (term possibly
present) triggers the tsidx lookup. False positives cause unnecessary tsidx reads;
false negatives are impossible. The false-positive rate is controlled by the filter
size relative to the number of terms indexed.

---

## 6. SPL functional requirements

SPL (Search Processing Language) is the pipe-based query language used by Splunk.
A pipe expression reads left-to-right: the first command produces an initial result
set, and each subsequent command transforms it. Commands are categorized here by
the storage and execution requirements they impose on an implementation.

### 6.1 Tier 1: basic usability

These commands are required before SPL is useful for any operational purpose. They
can be implemented with the existing `scan_time_range` storage interface plus
simple in-memory post-processing.

| Command | SQL equivalent | Storage requirement |
| --- | --- | --- |
| time-range filtering | `WHERE time_event_ns >= ?1 AND time_event_ns < ?2` | B-tree index on `time_event_ns` (present) |
| `search <term>` | `WHERE raw LIKE '%term%'` or FTS5 | Full scan or FTS5 virtual table |
| `fields <f1> <f2>` | `SELECT f1, f2` | Column projection, no storage change |
| `table <f1> <f2>` | same as `fields` with tabular output | Same |
| `head <N>` | `LIMIT N` | Limit on result set, no storage change |
| `tail <N>` | `ORDER BY time_event_ns DESC LIMIT N` | Requires full scan or reverse index |
| `sort <field>` | `ORDER BY field` | In-memory sort of result set |
| `where <expr>` | `WHERE <expr>` | Application-layer filter after storage fetch |

### 6.2 Tier 2: analytics

These commands require either SQL aggregation or significant application-layer
computation. `scan_time_range` must return a full result set that is then processed
in memory, or the storage queries must be extended to support aggregation.

| Command | SQL equivalent | Implementation note |
| --- | --- | --- |
| `stats count by field` | `SELECT field, COUNT(*) GROUP BY field` | Requires GROUP BY and agg functions |
| `stats sum(f) by g` | `SELECT g, SUM(f) GROUP BY g` | Same; `f` must be numeric or extracted |
| `stats dc(field)` | `SELECT COUNT(DISTINCT field) GROUP BY ...` | Distinct count; SQLite supports this |
| `stats values(f)` | `SELECT GROUP_CONCAT(DISTINCT f)` | Array aggregation; SQLite `group_concat` |
| `eval f=expr` | Per-row expression evaluation | Application-layer expression engine |
| `rex field=f "pattern"` | Per-row regex extraction | Application-layer; populates virtual fields |
| `timechart span=1h count` | `SELECT time_bucket, COUNT(*) GROUP BY time_bucket` | Requires time-bucketing function |
| `dedup field` | `SELECT DISTINCT ON (field)` or window `ROW_NUMBER` | DISTINCT ON absent in SQLite; use GROUP BY |

### 6.3 Tier 3: power commands

These commands require either window functions, session-grouping logic, or
subsearch execution. Most require either a capable SQL layer (DataFusion, DuckDB,
PostgreSQL) or custom application-layer implementations.

| Command | Mechanism | Implementation note |
| --- | --- | --- |
| `streamstats` | Running window aggregation | SQL window functions: `COUNT OVER (ORDER BY time ROWS N PRECEDING)` |
| `eventstats` | Aggregate joined back per-row | Lateral join or two-pass: aggregate then join |
| `transaction` | Session grouping by field + timeout | Application-layer state machine; no SQL equivalent |
| `lookup` | Enrichment join against CSV/KV store | `LEFT JOIN` against a lookup table |
| `join` | Subsearch join | Nested query or hash join |

---

## 7. Industry and technology reading list

This section presents external resources relevant to spank-rs's storage, query,
and indexing problems. Each entry gives a tight description of what the resource
covers and why it is relevant here, followed by the URL. The URLs are to stable
documentation or specification pages. Entries are organized by topic area.

### 7.1 Log storage formats and engines

**Apache Parquet file format specification.** The normative description of the
columnar layout: row groups, column chunks, dictionary encoding, RLE bit-packing,
and footer statistics (min/max per row group per column). The footer statistics are
what enable predicate pushdown — a time-range query reads the footer to identify
which row groups contain any matching timestamps, then decompresses only those
groups. Essential reading before implementing warm/cold bucket rotation to Parquet.
https://parquet.apache.org/docs/file-format/

**Apache Arrow columnar format specification.** Defines the in-memory layout used
by DataFusion, DuckDB, and `arrow-rs`. The unit of exchange is a `RecordBatch`: a
set of equal-length columnar arrays sharing a schema. Understanding Arrow is
necessary to use `datafusion` or to pass data between `parquet` and `duckdb` without
copying. https://arrow.apache.org/docs/format/Columnar.html

**ClickHouse MergeTree engine documentation.** ClickHouse is a production-scale
column-oriented log store. Its MergeTree storage engine manages concurrent ingest
through immutable "parts" (analogous to Splunk's hot buckets): each insert creates a
small part, and a background thread merges parts asynchronously. The merge process
maintains sorted order and computes per-column min/max statistics. This is the
design pattern spank-rs's Partition layer will converge toward as it scales.
https://clickhouse.com/docs/en/engines/table-engines/mergetree-family/mergetree

**Grafana Loki architecture.** Loki stores logs as compressed chunks indexed by
label sets rather than by term. This trades keyword search flexibility for lower
index storage overhead. Relevant as a contrast to the Splunk tsidx model: Loki
shows what a log store looks like when you deliberately omit a full inverted index
and rely on label cardinality instead.
https://grafana.com/docs/loki/latest/get-started/architecture/

### 7.2 Query engines in Rust

**Apache DataFusion.** A SQL query engine implemented in Rust, built on Arrow. It
reads Parquet and CSV natively, supports a large subset of SQL including window
functions and lateral joins, and can be embedded in a Rust process without a network
round-trip. DataFusion is the practical path to SPL Tier 3 commands (`streamstats`,
`eventstats`) without writing a query engine from scratch. The `datafusion` crate is
the entry point. https://datafusion.apache.org/

**DuckDB Rust bindings.** DuckDB queries Parquet files natively with glob patterns
(`SELECT * FROM 'warm/**/*.parquet' WHERE time_event_ns > ?`). The Rust bindings
expose `duckdb::Connection`, which has the same interface as rusqlite. DuckDB's
OLAP vectorized execution makes it substantially faster than SQLite for aggregation
queries over large result sets. Its single-writer constraint makes it unsuitable as
an ingest target but well-suited as a read-only query layer over Parquet.
https://docs.rs/duckdb/latest/duckdb/

**Splunk SPL2 search reference.** Splunk's second-generation query language, moving
toward SQL-like syntax. Useful as a normative reference for the command semantics
in `§6`: what `stats`, `timechart`, and `dedup` are specified to return, including
edge cases around empty groups, null fields, and multi-value fields.
https://docs.splunk.com/Documentation/SCS/current/SearchReference/Introduction

**Microsoft Kusto Query Language (KQL) documentation.** KQL is structurally
identical to SPL (left-to-right pipe of commands), developed independently for Azure
Data Explorer. KQL's specification is more formally documented than SPL's and covers
the same command vocabulary (`summarize` = `stats`, `extend` = `eval`, `where` =
`where`). Reading KQL's definition of `summarize` clarifies edge cases that SPL's
documentation leaves implicit.
https://learn.microsoft.com/en-us/azure/data-explorer/kusto/query/

### 7.3 Rust crates

**`arrow-rs` and `parquet`.** The official Apache Arrow Rust implementation. The
`arrow` crate defines `RecordBatch` and the Arrow type system; the `parquet` crate
provides `ArrowWriter` (writes a `RecordBatch` stream to a Parquet file) and
`ParquetRecordBatchReader` (reads Parquet back as `RecordBatch` stream). These are
the starting point for bucket-rotation-to-Parquet.
https://docs.rs/arrow/latest/arrow/ and https://docs.rs/parquet/latest/parquet/

**`arc-swap`.** Lock-free `Arc<T>` swap primitive used in spank-rs for `HecPhase`
(see `§1`). The crate documentation explains the `Guard` return type, the
consistency guarantees under concurrent load, and the cases where a plain `Mutex`
is preferable (infrequent writes, complex write logic).
https://docs.rs/arc-swap/latest/arc_swap/

**`tantivy`.** A full-text search engine in Rust implementing an inverted index with
BM25 ranking, analogous to Lucene. `tantivy` is the practical path to a tsidx
equivalent for keyword search without implementing a term index from scratch. Its
index format is not Splunk-compatible, but its query API (`QueryParser`, `Searcher`,
`TopDocs`) covers the SPL `search` command's keyword and boolean query requirements.
https://docs.rs/tantivy/latest/tantivy/

**SQLite FTS5 extension.** SQLite's built-in full-text search. Creates an inverted
index over a TEXT column in a virtual table, with BM25 ranking and prefix query
support. The lowest-friction path to keyword search in spank-rs: no new crate
dependency, no separate process. Usable from rusqlite with `conn.execute("CREATE
VIRTUAL TABLE events_fts USING fts5(raw, content=events, content_rowid=id)", [])`.
https://www.sqlite.org/fts5.html

**`tokio-util` `CancellationToken`.** The structured cancellation primitive used in
spank-rs for all subsystem shutdown. The child-token pattern (`parent.child_token()`)
ensures that cancelling the parent cancels all descendants atomically. Documented at
the crate level with examples of the select-loop pattern used in `spank-tcp/src/receiver.rs`.
https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html

### 7.4 Indexing algorithms

**Bloom filters (Wikipedia).** Probabilistic membership test: given a set of items,
a Bloom filter answers "definitely not in the set" or "possibly in the set" with
tunable false-positive rate. The false-positive rate is a function of the filter bit
width and the number of hash functions; it increases as the filter fills. Splunk
uses one Bloom filter per bucket to allow keyword searches to skip entire buckets
without reading the tsidx. The `bloomfilter` and `bloom` Rust crates provide
implementations. https://en.wikipedia.org/wiki/Bloom_filter

**Roaring Bitmaps.** Compressed bitset data structure used in production inverted
indexes (Lucene, tantivy, Druid). More space-efficient than sorted arrays for
high-cardinality posting lists and supports fast set operations (union, intersection)
without decompression. Relevant if spank-rs implements a custom term index rather
than using FTS5 or tantivy. https://roaringbitmap.org/

**OpenTelemetry Log Data Model.** The CNCF specification for structured log events.
The field vocabulary (`Body`, `Attributes`, `Resource`, `TraceId`, `SpanId`,
`SeverityNumber`) maps partially onto Splunk's `_raw`, `fields`, `host`,
`sourcetype`. Relevant for future HEC-to-OTLP compatibility and for understanding
where the two models diverge. https://opentelemetry.io/docs/specs/otel/logs/data-model/

---

## References

[1] SQLite documentation, "WAL mode", https://www.sqlite.org/wal.html
[2] SQLite documentation, "BEGIN IMMEDIATE", https://www.sqlite.org/lang_transaction.html
[3] SQLite documentation, "FTS5", https://www.sqlite.org/fts5.html
[4] rusqlite documentation, "prepare_cached", https://docs.rs/rusqlite/latest/rusqlite/struct.Connection.html#method.prepare_cached
[5] arc-swap crate documentation, https://docs.rs/arc-swap/latest/arc_swap/
[6] bytes crate documentation, "BytesMut::split_to", https://docs.rs/bytes/latest/bytes/struct.BytesMut.html#method.split_to
[7] Apache Parquet format specification, https://parquet.apache.org/docs/file-format/
[8] Apache Arrow columnar format specification, https://arrow.apache.org/docs/format/Columnar.html
[9] Apache DataFusion documentation, https://datafusion.apache.org/
[10] DuckDB Rust bindings, https://docs.rs/duckdb/latest/duckdb/
[11] tantivy crate documentation, https://docs.rs/tantivy/latest/tantivy/
[12] ClickHouse MergeTree engine, https://clickhouse.com/docs/en/engines/table-engines/mergetree-family/mergetree
[13] Grafana Loki architecture, https://grafana.com/docs/loki/latest/get-started/architecture/
[14] Splunk SPL2 reference, https://docs.splunk.com/Documentation/SCS/current/SearchReference/Introduction
[15] Microsoft KQL documentation, https://learn.microsoft.com/en-us/azure/data-explorer/kusto/query/
[16] OpenTelemetry Log Data Model, https://opentelemetry.io/docs/specs/otel/logs/data-model/
[17] Bloom filter, Wikipedia, https://en.wikipedia.org/wiki/Bloom_filter
[18] Roaring Bitmaps, https://roaringbitmap.org/
[19] tokio-util CancellationToken, https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html
[20] arrow-rs crate, https://docs.rs/arrow/latest/arrow/
[21] parquet crate, https://docs.rs/parquet/latest/parquet/
