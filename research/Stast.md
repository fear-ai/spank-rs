# Stast — Rust Coding Standards and Canonical Patterns

`Focus: research` — a code-actionable rulebook for the Rust port: toolchain, workspace layout, error handling, concurrency, trait design, documentation, tests, and component composability. The audience is a developer writing or reviewing Rust code in this tree. Does not receive work items (`Plan.md`) or subsystem contracts (`docs/`).

## Scope

Stast is the Rust counterpart to `Standards.md`. It is a code-actionable rulebook for a hypothetical Rust implementation of Spank: what the toolchain does, how the workspace is laid out, how errors and concurrency and traits are shaped, what the documentation and tests must include, and how the codebase composes from configurable parts. Material is drawn from Rust API guidelines, the Rust 2018+ idioms, and patterns observed in the Rust OSS projects inventoried in `Pyst.md` Appendix A — Vector, Quickwit, Parseable, tokio, axum, hyper, rusqlite, tracing.

Stast is prescriptive where the community has converged and offers explicit alternatives where it has not. Each section closes with a one-line "rule" suitable for a checklist; alternatives appear in their own subsection so they cannot be confused for the rule.

## Table of Contents

1. Toolchain Baseline
2. Workspace and Crate Layout
3. Naming
4. Error Handling
5. Async Patterns
6. Lifetime, Ownership, API Surface
7. Trait Design
8. Documentation
9. Testing
10. Unsafe
11. Composition and In-Flight Configuration
12. Compatibility with Stable Rust and Edition
13. Mandate Index

---

## 1. Toolchain Baseline

The Rust ecosystem has converged on a small set of tools that every project runs. Skipping them is unusual; deviating from their default settings is fine and common.

**rust-toolchain.toml** at the workspace root pins the toolchain channel, the edition, and the components. Pin the channel (`stable` plus a precise minor version, e.g. `1.86.0`) for production builds; pin `nightly-YYYY-MM-DD` only when a feature requires it and document why in the file's comment. The components list always includes `rustfmt`, `clippy`, and on CI machines `rust-src` if a build dependency consumes the standard-library source.

**rustfmt** is mandatory and enforced. `cargo fmt --check` is a CI gate. The configuration is `rustfmt.toml`; settings beyond the defaults are added only with a one-line justification comment per setting. Avoid stylistic-only knobs (e.g. `tab_spaces = 2`); the cost of disagreeing with the rest of the ecosystem outweighs any benefit.

**clippy** is mandatory and enforced. The lint profile is set in `Cargo.toml`'s `[workspace.lints.clippy]` section (workspace-wide) or `[lints.clippy]` (per crate, on Rust 1.74+). Recommended baseline:

```toml
[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }
cargo = { level = "warn", priority = -1 }

# Selective relaxations — each documented inline.
module_name_repetitions = "allow"   # Rust API guidelines tolerate this in practice.
must_use_candidate = "allow"
missing_errors_doc = "warn"          # Required for lib crates only.
```

CI runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`. Local pre-push runs the same command. Disable individual lints inline with `#[allow(clippy::xxx)] // reason: ...` and require the reason text in code review.

**cargo-deny** scans dependencies for license, advisories, banned crates, and duplicate-dependency drift. Configuration in `deny.toml`. CI gates: `cargo deny check advisories licenses bans sources`. License allow-list is explicit; new licenses require a PR to `deny.toml`.

**cargo-audit** is the older RustSec advisory scanner; cargo-deny's `advisories` subsystem subsumes it. Use one, not both. Recommendation: cargo-deny for everything in one tool.

**cargo-nextest** replaces `cargo test` for parallelism and JUnit output. Faster, better failure isolation, mature. CI uses nextest; local `cargo test` remains fine for quick iteration.

**miri** runs unsafe code under interpretation to find UB. Required only for crates containing `unsafe` blocks; if any crate has unsafe, that crate has a CI job `cargo +nightly miri test`. If `forbid(unsafe_code)` covers the crate, no miri job is needed.

**cargo-geiger** measures the unsafe footprint across the dependency graph. Useful as a tracking metric in `README.md` ("Unsafe blocks: 0 in our code, N in deps"). Not a CI gate.

**Mandate.** Pin the toolchain via `rust-toolchain.toml`. CI gates: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo deny check`, `cargo nextest run`. miri runs only on crates containing `unsafe`.

### 1.1 Alternatives

- **Pinning to `stable` without a minor version**: simpler, breaks under MSRV drift. Acceptable for pure-library crates with explicit MSRV; rejected for production binaries.
- **clippy without `pedantic`**: less noise, less learning. Acceptable for early prototypes; rejected for the production codebase because pedantic catches real readability problems (`must_use_candidate`, `redundant_closure_for_method_calls`).
- **`cargo test` instead of nextest**: builtin, no extra tool. Slower and produces less useful CI output; the migration cost is one line in the workflow file.

## 2. Workspace and Crate Layout

A non-trivial Rust project is a Cargo workspace. Vector ships 31 internal crates under `lib/`; Quickwit and Parseable do similar splits. The single-crate layout is fine for a library that does one thing and a small CLI; everything else benefits from being a workspace.

**Workspace shape**:

```
Cargo.toml                    # workspace manifest, [workspace.dependencies], [workspace.lints]
rust-toolchain.toml
deny.toml
rustfmt.toml
crates/
  spank-core/                 # ABCs, types, errors used by everyone
  spank-cfg/                  # configuration parsing and validation
  spank-store/                # bucket, partition, SQLite specifics
  spank-hec/                  # HEC receiver
  spank-api/                  # REST API server
  spank-tcp/                  # TCP receiver
  spank-files/                # File tailer (FileMonitor)
  spank-shipper/              # TCP egress (TcpSender / Forwarder)
  spank-obs/                  # tracing init, metric constants
  spank/                      # binary; depends on all the above
xtask/                        # cargo xtask helpers (release, lint summary)
docs/
tests/                        # workspace-level integration tests
```

The layout above reflects the actual workspace as of the initial implementation pass. Crates for SPL (`spank-search`), UDP input, and additional storage backends are not yet present; they will follow as phases land per `docs/Sparst.md §12`.

**Cargo.toml at the workspace root** defines `[workspace]`, `[workspace.dependencies]` (pinned, used by all member crates), `[workspace.lints]`, `[profile.release]`, `[profile.dev]`. Member crates import dependencies via `dep = { workspace = true }`. This is the single source of truth for versions.

**lib.rs vs main.rs.** Library crates have only `src/lib.rs`. Binary crates have only `src/main.rs` (which is small — typically `tokio::main` and a call into a library `run` function). A crate that is both a library and a binary has `src/lib.rs` and `src/main.rs` where main.rs depends on the library. Test the library; the binary is a thin shim.

**Module file form.** Use the post-2018 form — `src/foo.rs` and `src/foo/` for submodules — not `src/foo/mod.rs`. The latter is allowed but deprecated by community convention; new code uses the flat form.

**Re-export discipline.** A crate's `lib.rs` `pub use`s types intended for external consumption. Anything not re-exported is internal even if `pub`. Document the re-export surface; treat it as the API.

**Visibility.** Default to private. `pub(crate)` for crate-internal API. `pub` only for items in the crate's `lib.rs` re-export list.

**Feature flags as composition.** Cargo features compose at compile time: `spank-cli` enables `tls`, `fts5`, `forwarder`, etc. Default features are minimal — what every user needs. Optional features are additive (`tls`, `prometheus-metrics`); features that change behavior breakingly (e.g. choosing one of two crypto backends) are documented as mutually exclusive and tested with `cargo hack` to verify each combination compiles.

**Mandate.** Cargo workspace, post-2018 module form, `[workspace.dependencies]` pinning, default-private visibility, re-export only at `lib.rs`, additive features only.

### 2.1 Alternatives

- **Single mega-crate.** Simpler at the start; compile-time scales poorly past ~50k lines. Vector's 31-crate split is partly a compile-time decision (parallel `cargo build`).
- **Per-feature crates published separately to crates.io.** Maximum modularity; substantial release-coordination cost. Reasonable when external integrators need to depend on a subset (e.g. `spank-storage` could be a library for other tools); not necessary just for internal organization.
- **`mod.rs` everywhere.** Older form, still legal. Avoid in new code.

## 3. Naming

Rust naming is governed by the Rust API Guidelines (https://rust-lang.github.io/api-guidelines/naming.html). Every Rust developer reads it; deviations are visible.

**Cases.**

- `snake_case`: functions, methods, modules, variables, fields, lifetimes (`'a`), Cargo crate names.
- `UpperCamelCase`: types (structs, enums, type aliases), traits, type parameters, enum variants.
- `SCREAMING_SNAKE_CASE`: constants, statics.

**Conversion methods.** The prefix communicates the cost.

- `as_*`: free, borrows. `String::as_str` returns `&str`.
- `to_*`: cheap or moderate, allocates. `String::to_owned`.
- `into_*`: consumes the receiver. `Vec::into_boxed_slice`.

**Constructors.** `new` for the canonical constructor; `with_*` for builders; `from_*` for type conversions; `try_new`/`try_from` for fallible constructors. A type with multiple constructors uses `from_<source>` (`Path::from_string`, `Path::from_bytes`).

**Iterator methods.** A type with iteration exposes:

- `iter(&self) -> Iter<'_>`: borrowing iterator.
- `iter_mut(&mut self) -> IterMut<'_>`: mutable-borrow iterator.
- `into_iter(self) -> IntoIter`: consuming iterator (impl `IntoIterator`).

**Predicates.** `is_<noun>` for boolean state (`is_empty`, `is_dir`); `has_<noun>` for boolean possession (`has_root`).

**Domain naming.** Avoid stutter — `spank_core::error::Error` not `spank_core::error::SpankError`. Module name plus item name reads naturally at the call site (`error::Error::Config { .. }`). Apply Standards.md M03 spirit: project brand stays out of internal type names; the crate name carries the brand.

**File names** match the module they declare: `src/receiver.rs` declares `mod receiver`; `src/inputs/tcp.rs` declares `mod tcp` inside `mod inputs`.

**Type-name brevity.** Long names indicate a missing module. `HecRequestProcessorBuilder` becomes `hec::request::ProcessorBuilder` — the module path supplies what the name elides.

**Mandate.** Follow Rust API Guidelines naming. No project brand inside type names. Conversion-cost semantics in the prefix (`as_`/`to_`/`into_`).

### 3.1 Alternatives

- **Hungarian-ish prefixes** (`m_field`, `s_static`): explicitly rejected by the API Guidelines and by every reviewed project.
- **Underscore-prefix for unused** (`_var`): allowed and idiomatic; the compiler treats `_x` as intentionally unused.
- **Single-letter type parameters** (`T`, `K`, `V`, `E`): idiomatic. Multi-letter type parameters (`Item`, `Error`) are common in standard-library traits and fine when the role isn't obvious.

## 4. Error Handling

The Rust convention is `Result<T, E>` for all fallible operations and `?` for propagation. Two libraries dominate the error-type ecosystem and they have distinct uses.

**Libraries use `thiserror`.** Each crate defines its own error enum with `#[derive(thiserror::Error)]`. Variants name the failure class; `#[from]` attributes auto-implement `From` for chained errors.

```rust
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("bucket {0:?} not found")]
    BucketNotFound(BucketId),

    #[error("sqlite write failed")]
    SqliteWrite(#[from] rusqlite::Error),

    #[error("io error on {path:?}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
}
```

The variant name is grep-able; the `Display` impl is the user-facing message; `#[source]` chains errors for `Error::source()` traversal. No `String`-only variants for domain conditions — they erase the type and break call-site discrimination.

**Applications use `anyhow` or `eyre`.** A binary's main function returns `anyhow::Result<()>`. `?` propagates anything that implements `std::error::Error`. `.context("descriptive string")` adds layers. Backtraces appear when `RUST_BACKTRACE=1`.

```rust
fn run() -> anyhow::Result<()> {
    let config = config::load(&args.config_path)
        .with_context(|| format!("loading config from {}", args.config_path.display()))?;
    let commander = Commander::new(config)
        .context("constructing commander")?;
    commander.run()
        .context("commander main loop")?;
    Ok(())
}
```

`eyre` is API-compatible with `anyhow` and supports custom report handlers (`color-eyre` for colored output, `miette` for rustc-style diagnostics). Pick one; do not mix.

**Where the boundary is.** `thiserror` types are public API — they cross crate boundaries and external code matches on variants. `anyhow::Error` is internal — it crosses internal function boundaries inside the binary crate. The binary crate's `main` is the only place `anyhow::Error` becomes a process exit. A library crate must not return `anyhow::Error` because consumers cannot match on it.

**`Box<dyn Error + Send + Sync>` as escape hatch.** Acceptable in a private fn signature when the error type is genuinely heterogeneous and a custom enum would be premature. Use `anyhow` instead in application code; use a real enum in library code.

**Panic discipline.**

- `panic!`: reserved for "we have a bug" — invariant violations the type system cannot express.
- `unreachable!()`: matches and `if let` arms that genuinely cannot execute. Each one carries a `// reason: ...` comment.
- `unwrap()`: never in library code. In application code, only on values proven non-`None`/`Ok` by surrounding logic, with a comment.
- `expect("reason")`: preferred over `unwrap()` because the message survives in the panic and is grep-able. Use for "this fn is called only after init has succeeded" and similar.
- Tests may `unwrap()` freely. Examples in doctests `unwrap()` for brevity.

`panic = "abort"` in `[profile.release]` is recommended for production binaries: smaller binary, no unwind tables, `panic!` immediately terminates. Library crates should be agnostic; they tolerate either.

**Mandate.** Library crates use `thiserror`. Application binaries use `anyhow` (or `eyre`, project-wide consistent). No string-only error variants for domain conditions. No `unwrap()` in library code. `expect("reason")` over `unwrap()` everywhere else.

### 4.1 Alternatives

- **`failure` crate.** Deprecated; do not use.
- **Custom error types without `thiserror`.** Acceptable but verbose; loses `#[from]` ergonomics. Rare in modern code.
- **`miette` instead of `anyhow`.** Better diagnostics for tools that present errors to humans (compilers, linters). Overkill for a server's structured logs.
- **Returning `Result<T, String>`.** Strongly discouraged. Strings are not patterns.

## 5. Async Patterns

Rust async is opinionated; the prevailing runtime is `tokio`. Project-internal consistency matters more than cross-project comparison.

**Runtime.** Use `tokio` with the `multi-thread` runtime for servers; `current-thread` for CLI tools and small fixtures. `async-std` and `smol` exist; if you adopt either, the choice is project-wide and irreversible without rewriting every dependency.

**`async fn` in traits.** Stable since Rust 1.75. Use it directly; avoid the `async-trait` crate unless you need object-safety (`Box<dyn Trait>`). When object-safety is required, use `async-trait` and pay the per-call `Box::pin` allocation.

```rust
// Prefer:
trait Source {
    async fn run(&self) -> Result<(), SourceError>;
}

// When object-safe is required:
#[async_trait::async_trait]
trait DynSource: Send + Sync {
    async fn run(&self) -> Result<(), SourceError>;
}
```

**`Send` discipline.** Tasks spawned with `tokio::spawn` require `Send + 'static`. Most futures are `Send` automatically; the exception is futures holding `!Send` state (`Rc`, raw pointers, MutexGuard across `.await`). The compiler's error message is loud and accurate; fix at the source, do not paper over with `Arc`/`Mutex` reflexively.

**Cancellation safety.** A future is cancellation-safe if dropping it mid-`.await` leaves no torn state. `tokio::select!` requires every branch to be cancellation-safe — the canonical resource on this is the `tokio::select!` documentation, which lists which `tokio::*` futures are safe and which are not. As a rule:

- `tokio::time::sleep` is cancellation-safe.
- Channel `recv` is cancellation-safe.
- A custom future that holds a `MutexGuard` across a `.await` is not.

If a future must complete, do not use `tokio::select!` against it; spawn it and wait on its `JoinHandle`.

**Shared state.** The hierarchy of preference:

1. **Message passing**: `tokio::sync::mpsc` / `tokio::sync::broadcast` / `tokio::sync::watch`. Each task owns its state; communication is explicit.
2. **`Arc<X>` immutable shared state**: the config object, lookup tables, anything read-only after construction.
3. **`Arc<RwLock<T>>` for read-mostly**: `tokio::sync::RwLock` (async-aware) or `parking_lot::RwLock` (sync, faster, never poisoned). The token registry under a read lock during validation.
4. **`Arc<Mutex<T>>`**: `tokio::sync::Mutex` only when you must hold across `.await`; `parking_lot::Mutex` otherwise. Hold time as short as possible.
5. **`arc_swap::ArcSwap<T>` for hot configuration**: lock-free read, atomic swap on update. Use for items read on every request and updated rarely (config, token list).

**Structured concurrency.** Prefer `tokio::task::JoinSet` over loose `tokio::spawn` for a group of tasks that must complete together or be cancelled together. `CancellationToken` from `tokio_util::sync::CancellationToken` carries cancellation through hierarchies.

**Escaping async.** Heavy CPU work runs on `tokio::task::spawn_blocking`. JSON parse of a 1 KB body is not heavy; SHA-256 of a 100 MB body is. Calibrate by measurement.

**Mandate.** tokio multi-thread runtime. Native `async fn` in traits except where object-safety demands `async-trait`. Message passing first; `ArcSwap` for hot read-mostly state. `JoinSet` for grouped tasks.

### 5.1 Alternatives

- **`async-std`**: API resembles `std`; smaller community; declining momentum. Avoid for new work unless an essential dependency forces it.
- **`smol`**: minimal, embeddable. Fine for libraries; does not compete with tokio for application servers.
- **`glommio`**: thread-per-core io_uring runtime. Excellent fit for storage-bound workloads; small ecosystem; locks you in.
- **`monoio`**: similar shape, ByteDance project. Same tradeoffs.

## 6. Lifetime, Ownership, API Surface

API choices about ownership are ergonomics decisions visible to every caller. The defaults are simple; the alternatives have specific niches.

**Argument types.**

- `&str` over `&String`. `&[T]` over `&Vec<T>`. The slice type accepts both owned and borrowed callers.
- `impl AsRef<Path>` for path-like arguments (accepts `&str`, `&Path`, `String`, `PathBuf`).
- `impl Into<String>` when ownership is sometimes needed and sometimes not.
- `Cow<'_, str>` in return types when the function might return a borrowed slice or an owned string depending on input — avoid in argument types.

**Return types.**

- Owned return (`String`, `Vec<T>`) is the default unless the borrow has obvious lifetime.
- `impl Iterator<Item = T>` rather than concrete `Iter<'_>` for return types — keeps the iterator type opaque and substitutable.
- `impl Trait` in return position for "I return some type satisfying this trait, you don't need to know which" — common for closure-returning factories and async functions returning `impl Future`.

**`Box<dyn Trait>` versus generics.** Generics (monomorphized at compile time) are faster and impose no heap allocation; pay with code size and longer build. `Box<dyn Trait>` (dynamic dispatch) is the right choice when:

- The set of implementations is open or runtime-determined (plugins).
- The same type appears in heterogeneous collections (`Vec<Box<dyn Source>>`).
- Compile time dominates (a deeply generic type instantiated 100 ways).

In Spank's shape: input subsystems (`Source` trait) use `Box<dyn Source>` — they form a heterogeneous collection. The storage backend is a generic parameter on the indexer because it is chosen once at startup and exercised in hot loops.

**Builder pattern.** Two flavors:

- **Mutable builder** (`X::builder().with_y(y).with_z(z).build()`). Simple, used everywhere (hyper, reqwest, axum). Constructor errors surface in `build()`.
- **Typestate builder** (`X::builder().with_y(y).with_z(z).build()` where the type encodes which fields are set). Stronger compile-time guarantee that `build()` cannot fail on missing fields. Verbose; reserved for cases where the constructor failure is genuinely intolerable.

Default to mutable builder. Use typestate when the constructor genuinely cannot return `Err`.

**Newtypes for invariants.** A `u64` is not an `IndexId`. Wrap distinct identifiers:

```rust
#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BucketId(u64);

#[derive(Copy, Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AckId(u64);
```

The compiler refuses to mix them. Add `Display`, `FromStr`, and `serde::{Serialize, Deserialize}` only when the wire shape demands.

**Mandate.** Slices over owned in arguments; `impl AsRef<Path>` for paths. Generics for hot internal seams; `Box<dyn Trait>` for heterogeneous or plugin collections. Mutable builders; newtypes for distinct identifiers.

### 6.1 Alternatives

- **`String` and `Vec<T>` everywhere.** Simpler; allocates more; common in early Rust code. Fine for a CLI; rejected for a server.
- **Lifetime-elided functions only.** Avoiding explicit lifetimes is a goal, not a rule. Sometimes `fn x<'a>(s: &'a str) -> &'a str` is clearer than relying on elision.

## 7. Trait Design

**Object-safety.** A trait is object-safe if it can become `dyn Trait`. The full rules are in the Rustnomicon; the practical checklist:

- All methods have `&self` or `&mut self` (no generic `self`).
- No methods are generic over types (generic over lifetimes is fine).
- No `Self: Sized` bound on methods (or those methods are explicitly opted out).

If a trait is intended for dynamic dispatch (plugin point, heterogeneous collection), make it object-safe and assert it: `const _: Option<&dyn MyTrait> = None;` is a compile-time check.

**Sealed traits.** When a trait is implemented only inside the crate but exposed publicly so users can refer to it as a bound, use the sealed-trait pattern:

```rust
mod private {
    pub trait Sealed {}
}

pub trait MyTrait: private::Sealed {
    fn method(&self);
}
```

External crates cannot impl `MyTrait` because they cannot impl `Sealed`. This preserves freedom to add methods without breaking SemVer.

**Extension traits.** Adding methods to a foreign type goes through an extension trait — a trait you define and `impl` for the foreign type, plus a `use` at the call site. Convention: name with `Ext` suffix (`SliceExt`, `ResultExt`).

**Marker traits.** Empty traits used only as type bounds (`Send`, `Sync`, `Unpin`). Use sparingly; each is a permanent commitment to backward-compatible behavior.

**Trait method defaults.** A default implementation lets the trait grow with backward compatibility — implementors who don't override see the new method automatically. Use this for traits with many implementations.

**Mandate.** Object-safe traits at every plugin point; assert it. Sealed traits when the trait is closed but used as a bound. `Ext`-suffixed extension traits.

### 7.1 Alternatives

- **`enum`-based dispatch instead of `dyn Trait`.** Faster (no vtable), closed (no third-party extension). Good for a fixed small set; bad for plugins.
- **Static parameter on the consumer (`fn f<S: Source>(s: S)`)**: no vtable; can't store heterogeneous instances together. Use for one-call-site cases.

## 8. Documentation

`rustdoc` is mandatory and CI-rendered.

**Item doc comments.** `///` above every public item. Required structure for non-trivial items:

````rust
/// Brief one-line description.
///
/// Longer prose explaining what the item does and why.
///
/// # Examples
///
/// ```
/// use spank_core::record::Record;
/// let r = Record::new("hello");
/// assert_eq!(r.raw(), "hello");
/// ```
///
/// # Errors
///
/// Returns `Error::InvalidUtf8` if `bytes` is not valid UTF-8.
///
/// # Panics
///
/// Panics if `capacity` is zero. (Avoid documenting panics — prefer `Result`.)
///
/// # Safety
///
/// Required if and only if the function is `unsafe fn`. Document every invariant
/// the caller must uphold.
pub fn parse(bytes: &[u8]) -> Result<Record, ParseError> { ... }
````

`//!` at the top of `lib.rs` and each module documents the module itself.

**Doctests** are integration tests that run `cargo test`. A failing example in the docs is a failing build. Use `no_run` for examples that compile but don't run (e.g. binding a port); use ignore for examples that don't compile. Avoid both unless necessary.

**`#[doc(hidden)]`** for items that are technically `pub` but not part of the API (e.g. macro internals). Document the reason in a comment above.

**README.md** at each crate root summarizes purpose and links to the rustdoc. The crate root's `lib.rs` `//!` re-uses the same prose via `#![doc = include_str!("../README.md")]` — single source.

**CI gate**: `cargo doc --no-deps --workspace -- -D warnings`. Broken intra-doc links fail the build.

**Mandate.** rustdoc on every public item with the four-section structure where applicable. CI gate on `cargo doc -- -D warnings`. README is the crate-root module doc.

### 8.1 Alternatives

- **Skipping rustdoc on `pub` items.** `missing_docs` lint catches it. Acceptable for crate-internal types that happen to be `pub` for visibility reasons; reject for the documented API surface.
- **Manual examples instead of doctests.** Examples drift out of date; doctests do not.

## 9. Testing

The Rust testing layout is fixed by Cargo and well-conventionalized.

**Unit tests** live in the same file as the code, in a `#[cfg(test)] mod tests { ... }` block. The block has access to private items.

**Integration tests** live in `tests/` at the crate root, one file per test module. They exercise the public API only.

**Doctests** are unit-of-documentation tests; see §8.

**Workspace integration tests** at the workspace `tests/` directory exercise multiple crates together (HEC + Indexer + APIServer).

**Test naming.** `test_<unit>_<scenario>_<expected>` works well for grep-ability. `test_parse_invalid_utf8_returns_error`. Avoid bare `test_works`.

**Property-based tests.** `proptest` is the prevailing crate. Use for parsers, codecs, anything with broad input domains. `quickcheck` is older and still used; pick one project-wide.

**Snapshot tests.** `insta` for output that's hard to assert structurally (formatted text, generated SQL, JSON shapes). Snapshots are checked into git; review on update.

**Benchmark.** `criterion` is the de-facto standard. A `benches/` directory with `criterion::benchmark_group!` files. Run `cargo bench`; CI runs them on a schedule with regression alerts, not per-PR (variance is high).

**HTTP mocking.** `wiremock` for outbound HTTP under test. `axum::body::Body` constructed directly for inbound under test (no real socket needed).

**Container-based integration.** `testcontainers-rs` for tests that need a real Postgres, real Kafka. Slow; gated behind a feature flag (`cargo test --features integration`).

**Test isolation.** Each test gets its own tempdir via `tempfile::TempDir`. Each test gets its own bound port via `std::net::TcpListener::bind("127.0.0.1:0")` and reading the assigned port. Never hardcode ports.

**No `tokio::time::sleep` as primary sync.** Same rule as Standards.md M27: use `tokio::sync::Notify`, `Barrier`, or `oneshot` channels to coordinate.

**Mandate.** Colocated unit tests, `tests/` for integration, `proptest` for properties, `criterion` for benchmarks, `tempfile` and bound-to-zero for isolation, no sleep for sync.

### 9.1 Alternatives

- **`mockall` for mocks.** Generates mock impls of traits. Acceptable; the community trends toward fakes (real implementations against in-memory backends) over mocks because mocks couple tests to implementation details. Vector uses fakes throughout `lib/vector-core/src/event/test_util.rs`.
- **`rstest` for parameterized tests.** Compact; good for table-driven tests. Use it.
- **`test-case` macro.** Similar to `rstest`; less feature-rich.

## 10. Unsafe

**`#![forbid(unsafe_code)]`** at every crate root that does not need unsafe. The lint is at the crate root for visibility — adding any `unsafe` requires removing the attribute, which surfaces in review.

**Justified unsafe.** Each `unsafe` block carries a `// SAFETY:` comment explaining the invariants the writer is upholding. Each `unsafe fn` carries a `# Safety` rustdoc section explaining what callers must ensure.

```rust
// SAFETY: `ptr` is non-null and aligned because we just allocated it via
// `alloc::alloc(layout)` with a non-zero layout, and we have not deallocated
// it. The cast preserves provenance.
let r = unsafe { &mut *ptr };
```

**miri.** Crates with unsafe run `cargo +nightly miri test` on CI for those tests. miri detects undefined behavior — out-of-bounds, use-after-free, data races.

**cargo-geiger.** Track the unsafe count over time. Treat it as a metric, not a gate.

**Common unsafe patterns we accept.** FFI bindings to C libraries (`libsqlite3-sys`, `libc`). Cases where they appear in our code, we wrap them in safe abstractions and place those wrappers in a single module.

**Mandate.** `#![forbid(unsafe_code)]` at each crate root unless explicitly waived. `// SAFETY:` on every unsafe block. `# Safety` rustdoc on every unsafe fn. miri job on crates with unsafe.

### 10.1 Alternatives

- **`#![deny(unsafe_code)]`**: identical effect to `forbid` for compilation; allows local override with `#[allow(unsafe_code)]`. Use `forbid` for stronger signal.
- **No policy.** Loses the audit trail. Reject.

## 11. Composition and In-Flight Configuration

The Rust mechanisms for composition span compile time and runtime. Each has its place.

**Compile-time composition: cargo features.**

Features add functionality without changing existing behavior. `default = ["tls", "fts5"]`; users opt out with `default-features = false`. Mutually exclusive features (`crypto-aws-lc` xor `crypto-ring`) are rare and require a compile-error for the wrong combination.

`cargo hack --feature-powerset check` exercises every combination; CI runs it on PRs touching feature-gated code.

**Compile-time composition: trait objects vs generics.** See §6 — generics for hot internal paths, `Box<dyn Trait>` for heterogeneous collections.

**Runtime composition: trait-object registries.**

A factory pattern indexed by string name. Used by Vector for sources, transforms, sinks. Pattern:

```rust
pub trait Source: Send + Sync {
    fn name(&self) -> &str;
    async fn run(&self) -> Result<(), SourceError>;
}

pub struct SourceRegistry {
    factories: HashMap<String, Box<dyn Fn(&Config) -> Box<dyn Source> + Send + Sync>>,
}

impl SourceRegistry {
    pub fn register<F>(&mut self, name: &str, factory: F)
    where F: Fn(&Config) -> Box<dyn Source> + Send + Sync + 'static
    {
        self.factories.insert(name.to_string(), Box::new(factory));
    }
}
```

Configuration names a source by string; registry constructs by name. Inputs, outputs, parsers, search commands all fit this shape.

**Distributed slice via `linkme` or `inventory`.** Compile-time registration: each subsystem declares its factory in its own crate; `linkme::distributed_slice` collects them at link time. Avoids a central registration list. Used by some plugin-like systems; modest learning curve.

**Hot reload.** Configuration that changes at runtime sits behind `arc_swap::ArcSwap<Config>`. Readers do `let cfg = config.load();` (cheap, lock-free). Updates do `config.store(Arc::new(new_config))`. The previous `Arc` outlives any in-flight reader because `Arc`'s refcount handles it.

```rust
use arc_swap::ArcSwap;
use std::sync::Arc;

pub struct ConfigHandle(ArcSwap<Config>);

impl ConfigHandle {
    pub fn current(&self) -> Arc<Config> { self.0.load_full() }
    pub fn install(&self, new: Config) { self.0.store(Arc::new(new)); }
}
```

A SIGHUP handler reads the file, parses, validates, and `install()`s on success. On failure it logs and leaves the running config intact.

**`notify` crate** watches the config file directly; on modify it triggers reload. Useful for config-server-pushed updates; gratuitous for operator-edited files where SIGHUP is the natural trigger.

**Plugin systems.** Three flavors, ranked by ecosystem fit:

1. **Trait-object registry + cargo features.** Plugins are crates; building them in or out is a feature flag. No dynamic loading. This is what Vector does.
2. **wasm modules.** Vector has experimented (`vrl-wasm`); no production deployment we know of. Cost: wasm runtime, capability sandboxing, build complexity. Benefit: third-party plugins without recompilation. Defer.
3. **Dynamically-loaded `.so`.** Brittle ABI, no Rust stable C-ABI for traits. Avoid except for FFI wrappers around C libraries.

**Mandate.** Cargo features for compile-time composition with `cargo hack` exercising the powerset. Trait-object registries for runtime composition. `arc_swap::ArcSwap<Config>` for hot reload. SIGHUP triggers reload; failure leaves running config intact.

### 11.1 Alternatives

- **No plugin system.** All inputs/outputs/parsers built in. Simplest. Loses third-party extension. Acceptable if the surface is small and stable.
- **Dynamic dispatch everywhere via `Box<dyn Trait>`.** Removes the generics learning curve; loses inlining and adds heap allocations on hot paths.
- **Configuration loaded once at startup, no reload.** Matches Spank's current Python policy. Simpler; restart on config change. Acceptable; document as a deliberate non-feature.

## 12. Compatibility with Stable Rust and Edition

**MSRV (minimum supported Rust version).** Document in `Cargo.toml` via `rust-version`. Bump only with cause; bumping is a minor-version change for a library, a patch for a binary.

**Edition.** The current workspace uses `edition = "2021"`. The specific MSRV value, whether to target `edition = "2024"`, and the migration plan are deferred pending a dedicated review — the version and edition choices interact with transitive dependencies and are not purely internal decisions.

**`#![warn(rust_2018_idioms)]`** at every crate root: catches old-form module declarations, anonymous lifetimes, etc.

**Deprecation policy.** Public APIs that change are marked `#[deprecated(since = "0.x.0", note = "use ...")]` for one minor version before removal.

**Mandate.** Pin MSRV; document the chosen edition; `rust_2018_idioms` warn lint at every crate root.

## 13. Mandate Index

A compact list of every "Mandate" line above for use as a review checklist.

| # | Mandate | Section |
|---|---|---|
| S01 | `rust-toolchain.toml` pins toolchain. | §1 |
| S02 | CI: `cargo fmt --check`, `clippy -D warnings`, `deny check`, `nextest run`. | §1 |
| S03 | miri only on crates with `unsafe`. | §1 |
| S04 | Cargo workspace; `[workspace.dependencies]` for pinning. | §2 |
| S05 | Post-2018 module form (`foo.rs` + `foo/`). | §2 |
| S06 | Default-private visibility; `pub` only at re-export surface. | §2 |
| S07 | Additive features only; mutually exclusive features compile-error. | §2 |
| S08 | Rust API Guidelines naming. | §3 |
| S09 | No project brand inside type names. | §3 |
| S10 | `as_/to_/into_` carry conversion-cost semantics. | §3 |
| S11 | Library crates: `thiserror`. Application crates: `anyhow` (or `eyre`, project-wide). | §4 |
| S12 | No string-only error variants for domain conditions. | §4 |
| S13 | No `unwrap()` in library code. `expect("reason")` over `unwrap()` elsewhere. | §4 |
| S14 | tokio multi-thread runtime. | §5 |
| S15 | Native `async fn` in traits except where object-safety demands `async-trait`. | §5 |
| S16 | Message passing first; `ArcSwap` for hot read-mostly state. | §5 |
| S17 | Slices over owned in arguments; `impl AsRef<Path>` for paths. | §6 |
| S18 | Newtypes for distinct identifiers. | §6 |
| S19 | Object-safe at every plugin point; assert with compile-time check. | §7 |
| S20 | Sealed traits when closed but used as bound. | §7 |
| S21 | rustdoc on every public item with Examples/Errors/Panics/Safety as applicable. | §8 |
| S22 | CI gate `cargo doc -- -D warnings`. | §8 |
| S23 | Colocated unit tests; `tests/` for integration. | §9 |
| S24 | `proptest` for properties; `criterion` for benchmarks. | §9 |
| S25 | `tempfile` and bind-to-zero for test isolation. | §9 |
| S26 | No `tokio::time::sleep` as primary sync in tests. | §9 |
| S27 | `#![forbid(unsafe_code)]` unless waived. | §10 |
| S28 | `// SAFETY:` on every unsafe block; `# Safety` rustdoc on every unsafe fn. | §10 |
| S29 | Cargo features for compile-time composition; `cargo hack` powerset. | §11 |
| S30 | Trait-object registries for runtime composition. | §11 |
| S31 | `arc_swap::ArcSwap<Config>` for hot reload; SIGHUP triggers; failure preserves running config. | §11 |
| S32 | Pin MSRV; document edition (version/edition choice deferred for review); `rust_2018_idioms` warn. | §12 |

These thirty-two mandates parallel Standards.md's M01–M33 in spirit: each is checkable, each is grep-able, each is the kind of rule that survives review without restating the rationale.
