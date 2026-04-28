# Errors, Recovery, and Shutdown

Scope: how errors flow through `spank-rs`, what each recovery class costs, and
how the shutdown path is composed. The taxonomy lives in
`spank-core::error::SpankError`; this document explains the *why* behind each
class so call sites pick the right variant rather than reaching for `Internal`.

## 1. Error type

Libraries return `SpankError`. The binary `main` collapses to `anyhow::Error`
purely for the convenience of `Context::context` chains; nothing inside the
library tree depends on `anyhow`. The variants are organized by domain so a
single `match` at the top of a subsystem can route every error it might surface:

- `Config { message }` — figment failed, or `validate()` rejected the merged
  tree. Always fatal at the component boundary.
- `Io { syscall, target, source }` — every I/O failure carries the *syscall name*
  and the *target* (path or peer address) it was operating on. Constructors
  `SpankError::io(...)` and `SpankError::io_path(...)` enforce the convention; do
  not build the variant by hand.
- `Hec { code, text, http_status }` — Splunk HEC wire errors. `code` is the
  Splunk numeric code; `text` is the human string; `http_status` is what the
  receiver should return. Producing this variant is the contract for converting a
  protocol failure into an HTTP response.
- `Storage { message }` — wrapped backend errors (rusqlite, future Duck/PG).
- `Auth { message }` — token unknown, principal denied, channel rejected.
- `Lifecycle { message }` — startup failure or shutdown timeout. Component-fatal.
- `QueueFull { queue }` — bounded `mpsc::Sender::try_send` returned `Full`. The
  *only* path for backpressure; do not paper over it with `await`.
- `Internal { message }` — invariant violation. Should not happen; if it does,
  the process aborts and supervision restarts.

## 2. Recovery classes

`SpankError::recovery()` returns one of four classes. The class is the *only*
thing a generic caller needs to act on:

| Class | Meaning | Caller behavior |
| - | - | - |
| `Retryable` | Transient. Peer reset, timeout, gzip retry, store contention. | Back off and retry; emit `error_event!`. |
| `Backpressure` | Bounded resource exhausted. | Shed load — HEC code 9 / HTTP 503; do not retry in-process. |
| `FatalComponent` | Subsystem cannot continue. | Drop the component, log, signal lifecycle, let the Commander decide. |
| `FatalProcess` | Invariant broken. | Abort. Supervisor restarts the process. |

The classification is structural, not vibes-based. `Io` errors map their
`io::ErrorKind`: `ConnectionReset`, `ConnectionAborted`, `BrokenPipe`,
`TimedOut`, `Interrupted`, `WouldBlock` are `Retryable`; everything else is
`FatalComponent`. `AddrInUse` on bind is fatal (you cannot retry into a port
you don't own); `ConnectionReset` on a TCP read is retryable (the next
connection might land). Sites that want a different class should construct a
different variant rather than override the classification at the call site.

## 3. Backpressure path

The HEC ingress channel is a `tokio::sync::mpsc` with `cfg.hec.queue_depth`
slots. Receivers always go through `try_send`. On `Full`, the receiver returns
`SpankError::QueueFull { queue: "hec" }`, which `recovery()` maps to
`Backpressure`, which `routes()` maps to HTTP 503 with Splunk HEC code 9
("server-busy"). Crucially the request is *rejected*, not parked: a parked
request consumes a tokio task slot and produces tail latency that scales with
the queue depth instead of the backlog. The metric `spank.queue.full_total`
counts these rejections; the gauge `spank.queue.depth_current` tracks the
backlog. An operator alerting on the *gauge* sees pressure building; an
operator alerting on the *counter* sees pressure exceeded.

## 4. Shutdown

Shutdown is composed from three pieces:

1. `Lifecycle` — a tree of `tokio_util::sync::CancellationToken`s rooted at the
   process. Children are produced with `lifecycle.child(name)`. Cancelling a
   parent cancels every descendant.
2. `Drain` — a tag-keyed wait/signal primitive built on `tokio::sync::Notify`
   with a latched signaled-set so a wait that arrives after a signal still
   completes. Used to flush in-flight work for a specific batch tag.
3. `Sentinel` — an `End` or `Checkpoint` marker that travels through the
   pipeline alongside `Rows`. The downstream consumer treats `End` as
   "everything for this tag has arrived" and signals the corresponding `Drain`.

The `serve` path orchestrates these as follows. `ctrl_c` triggers
`lifecycle.shutdown()`, which propagates cancellation. The API server's
`graceful_shutdown` future is bound to the lifecycle token, so axum stops
accepting new connections and finishes in-flight ones. The HEC consumer task
drains its receiver until the channel closes. The optional TCP receiver's
listener and per-connection tasks observe the same token and unwind.
`tokio::time::timeout(cfg.runtime.shutdown_seconds, ...)` caps the wait on each
join handle so a misbehaving subsystem cannot hold the process forever; the
budget defaults to the value in `RuntimeConfig`.

The shipper side mirrors this: `FileMonitor` in `OneShot` mode emits
`FileOutput::Done(Sentinel::end(path))` when the file is exhausted; the bridge
forwards that as a sentinel through the `TcpSender`, which flushes its writer
and exits. Cancellation hits both halves through the same lifecycle root.

## 5. Anti-patterns

- Do not `unwrap()` an I/O error in a library. Use the `SpankError::io`
  constructor; the syscall name and target are the difference between a
  diagnosable incident and a 3am pager.
- Do not `await` on a backpressured send. The whole point of bounded channels
  is that the producer learns about the backlog *now*; an `await` hides the
  signal until the queue drains, which is exactly when you no longer need it.
- Do not invent new recovery classes at call sites. If `Retryable` is wrong for
  your case, the variant is wrong, not the classification.
