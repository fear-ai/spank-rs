# Procesp — Reasoning Framework and Engineering Methodology

`Focus: foundation` — the reasoning framework, working methodology, decision sequence, and engineering lifecycle for spank-rs. Adapts and extends `spank-py/Process.md`, recovering the dual customer-vendor perspective and structural reasoning that informed the original and restating it for the Rust port. Part A is the general methodology; Part B is its direct application to spank-rs covering documentation governance, coding standards, plans, and gates. This document does not receive implementation status (that belongs in `Plan.md`), product strategy conclusions (those remain in `spank-py/Product.md` until the Rust port forks positioning), or subsystem design rationale (that belongs in the Reference docs under `docs/`).

A developer reads Part A once to understand the reasoning framework, then consults Part B when a placement, standards, or structuring question arises. A model session involving document edits, research, or architectural decisions reads the relevant Part B section at the start.

---

## Table of Contents

**Part A — Methodology**

1. [The dual perspective](#1-the-dual-perspective)
2. [Structural position](#2-structural-position)
3. [Methodological underpinnings](#3-methodological-underpinnings)
4. [Decision sequence](#4-decision-sequence)
5. [Product-market fit path](#5-product-market-fit-path)
6. [Retrospective evaluation method](#6-retrospective-evaluation-method)

**Part B — Direct application to spank-rs**

7. [Documentation governance](#7-documentation-governance)
8. [Coding standards](#8-coding-standards)
9. [Plans and gates](#9-plans-and-gates)
10. [Working rules](#10-working-rules)

**Appendices**

- [Appendix A: Comparison with prior and adjacent methodologies](#appendix-a-comparison-with-prior-and-adjacent-methodologies)
- [Appendix B: Audit record](#appendix-b-audit-record)

---

## Part A — Methodology

## 1. The dual perspective

Every decision in this project lives simultaneously in two frames. The customer frame asks: what does this person need, what is their frustration, what workaround have they already built? The vendor frame asks: what can we build, at what cost, with what margin of confidence, and what does shipping it imply for what comes next? Both frames must be present at the same time, from the earliest requirements discussion through architecture, design, implementation, and release.

This is not a statement about user research methodology. It is a structural requirement for sound engineering decisions. A decision made entirely in the vendor frame — what is the cleanest implementation of this interface? — frequently produces correct code for a requirement that turns out to be the wrong requirement. A decision made entirely in the customer frame — users want X — frequently produces a feature whose implementation constraints invalidate the requirement before it ships. The value of maintaining both frames simultaneously is that the tension between them surfaces problems early: if a requirement is technically intractable at the desired cost, the customer frame asks whether the requirement can be restated; if a clean implementation produces something users cannot use, the vendor frame asks whether the clean implementation is actually cheaper.

The four confirmed audiences for Spank — SPL learner, CI fixture user, detection engineer, small-scale deployer — each bring a different blend of needs and workarounds. The vendor translation of each need is a different architectural constraint. Keeping both columns populated — user want/need on the left, vendor feature/constraint on the right — is the discipline that prevents the product from drifting into technical-correctness-without-adoption.

| User | Want | Need | Vendor feature | Vendor constraint |
|------|------|------|----------------|-------------------|
| SPL learner | Run SPL without a Splunk license | Zero-friction execution, no infrastructure | `spank demo`, embedded mode | SPL parser breadth, zero runtime deps |
| CI fixture user | Real HEC endpoint in pytest | Start-in-process, fast, teardown clean | `pytest-spank`, `spank start --test` | Graceful shutdown, deterministic state |
| Detection engineer | Validate Sigma rules in CI | SPL execution without Splunk in the loop | pySigma-backend-spank | SPL correctness on detection-relevant commands |
| Small-scale deployer | Managed log store without ELK overhead | Splunk-compatible ingest+query, one node | Shank bundle, REST API | HEC conformance, persistent storage durability |

The Rust port does not change these audiences or their needs. It changes the vendor column: better resource utilization, sharper performance envelope, native binary distribution, no Python runtime dependency. The dual-frame discipline applies unchanged.

## 2. Structural position

The structural gap Spank occupies is maintained by deliberate choices of the dominant players, not by neglect. Splunk has not published a formal SPL grammar; that absence is a moat. Sigma's execution gap is intentional: Sigma is a conversion tool by design. The HEC mock gap is recognized internally at Splunk across three of its own projects and left unaddressed. These are policy-maintained constraints — stable attractors in the competitive landscape that persist because filling them would contradict the incentive structure of the players who could.

The Rust port reinforces the position. A zero-runtime-dependency binary distributed via `cargo install` or a single static download has lower activation energy than a Python package for the CI fixture use case. A compiled HEC receiver is a more credible production component than a Python server for the small-scale deployer. The port does not change the structural argument; it improves the vendor translation of it.

The flywheel described in `spank-py/Process.md §1` applies without modification. SPL learners become CI users; CI users work adjacent to security teams; security teams encounter Sigma; Sigma users advocate back into the communities where Splunk decisions are made. The Rust port is a better vehicle for the same flywheel — it is not a different flywheel.

## 3. Methodological underpinnings

The decision patterns throughout this project are not ad hoc. They reflect a consistent application of systems theory, quality methodology, lean product development, and first-principles engineering. The complete treatment is in `spank-py/Process.md §2`; the condensed version here names each principle and its application to the Rust port specifically.

**Structural causation** (Meadows, *Thinking in Systems*, 2008): the structural gap is stable because it is maintained by incentive architecture, not neglect. The Rust port's strategic bet is identical to the Python port's — it is a different implementation of the same bridge.

**Increasing returns** (Arthur, *Increasing Returns and Path Dependence in the Economy*, 1994): the grammar publication and pySigma backend submission are infrastructure investments with network-scaled returns. Both apply to the Rust port without modification.

**Validated learning** (Ries, *The Lean Startup*, 2011): the primary product methodology. Form a hypothesis; build the minimum falsifiable test; measure real signal; decide before scaling. The Rust port's MVP is a working HEC endpoint and a usable `spank demo` — not a complete SPL implementation. The measurement loop restarts when real users install and run it.

**Jidoka / kaizen** (Ohno, *Toyota Production System*, 1978): build quality in at the source. The track-based implementation in spank-rs is kaizen: each track is small, bounded, verifiable, and reversible. The nine tracks completed in the first implementation pass are the first kaizen cycle; subsequent cycles refine them.

**Hoshin Kanri** (Akao, *Hoshin Kanri*, 1991): policy deployment cascade with bidirectional traceability. The flywheel is the breakthrough objective. Every task in `Plan.md` should be traceable upward to a phase in `docs/Sparst.md §12`, which is traceable to an audience need in `§1` of this document, which is traceable to the structural position in `§2`.

**First principles / five-step** (Musk / Isaacson, *Elon Musk*, 2023): make the requirement less dumb → delete → simplify → optimize → automate, in that order. The deliberate scope exclusions in spank-rs (no TLS in the binary, no gRPC, no OpenSSL) are first-step deletions.

**Requisite variety** (Ashby, *Introduction to Cybernetics*, 1956): D-prime positioning preserves optionality across all four audiences. The Rust port does not narrow positioning; it adds a deployment option.

**Security engineering** (Shostack, *Threat Modeling*, 2014; STRIDE): security properties designed in, not added after. The TCP line cap, the HEC body length limit, and the `try_send` backpressure boundary are security boundaries, not performance optimizations. STRIDE applied to the current trust boundaries: network inputs (HEC receiver, TCP receiver) are Tampering and DoS surfaces; the auth token store is a Spoofing surface; the file output path is an Information Disclosure surface. These are enumerated, not discovered after an incident.

**Build or buy.** The governing principle is to prefer taking before building — build only what cannot be taken without accepting a structural constraint. The classification for spank-rs components:

| Component | Classification | Reason |
|-----------|---------------|--------|
| SPL parser | Must own | No grammar, no OSS implementation |
| HEC protocol handling | Must own | Wire conformance is the product promise |
| axum / tokio | Lift | Standard; no value in reimplementing |
| SQLite via rusqlite | Should own seam | Interface layer; engine is lifted |
| TLS termination | Use as-is (LB) | Terminate at Envoy/Caddy/nginx |
| Prometheus export | Lift | Pull model, standard format |

## 4. Decision sequence

Every product or architectural decision follows a three-stage gate in order. No stage is skipped. The gate enforces loop order: measuring before building is the core discipline.

```
Stage 1: Research          Stage 2: Strategy          Stage 3: Execution
─────────────────────      ──────────────────────     ─────────────────────────
Hypothesis formed           Research output read        Task created and bounded
Minimum test defined        Verdict rendered            Phase assignment given
Evidence collected          docs/ or Sparst.md updated  Implementation begins
Gate evaluated              Pivot conditions set        Reference docs updated
                            Gate cleared or held
```

The gate between stages is not advisory. A strategy conclusion requires a research finding. An execution task requires a strategy conclusion. Work that begins before the gate has cleared carries the risk of building on an unvalidated assumption.

**Stage 1 — Research.** A question is formed as a falsifiable hypothesis. The minimum search or test is defined — the smallest effort that would produce a confirmatory or disconfirmatory signal. Evidence is collected in a `research/` document using the standard finding format: hypothesis, evidence, verdict. When the evidence threshold is met, a verdict is written and the research document is updated.

**Stage 2 — Strategy.** Research output is synthesized into a positioning, opportunity, or architectural decision. In spank-rs, strategy decisions land in `docs/Sparst.md` (design target), `docs/Network.md` (library choices), `docs/Errors.md` (error and recovery model), or `docs/Observability.md` (metrics and profiling decisions). The conclusion names the evidence it rests on and sets explicit pivot conditions.

**Stage 3 — Execution.** Tasks are created only after the strategy decision that justifies them exists. Each task is bounded (scope, files, test coverage), assigned to a phase in `docs/Sparst.md §12`, and traceable to the strategy conclusion above it. Implementation detail lives in the Reference docs, not in `Plan.md`.

## 5. Product-market fit path

PMF for Spank is not a single gate but a staged path. Each stage has an observable exit criterion and a set of features that are necessary conditions, not sufficient ones.

**Demo.** A new user can install the binary and run `spank demo` against a bundled dataset and receive SPL query results within five minutes. No configuration required. Exit criterion: three real users outside the development context complete the demo without assistance.

**POC.** A CI fixture user can start an in-process HEC endpoint, send events from a real shipper, and query results in a pytest test. Exit criterion: `pytest-spank` works in at least one real CI pipeline not maintained by the developers.

**MVP.** The four-column table in `§1` is satisfied for all four audiences simultaneously: learner can run SPL, CI user has a fixture, detection engineer can validate Sigma rules, small-scale deployer can run a persistent Shank instance. Exit criterion: all four use cases are tested end-to-end in the test suite with real shipper binaries.

**Minimum viable feature set.** HEC conformance passes the full wire-code suite (`test_hec_conformance.py`). The SPL subset in `docs/Sparst.md §3.5` is complete and conformance-tested. The REST management surface at `docs/Sparst.md §5.2` is implemented. The Shank bundle ships as a static binary with a systemd unit. Exit criterion: a real user runs Shank in production for 30 days without a restart forced by a software defect.

**Prioritization and bundling.** Features are prioritized on three axes: strategic value (which confirmed audience does this advance?), unblocking value (how many tasks does this gate?), and reversibility (how costly to redo if the approach is wrong?). The phase sequence in `docs/Sparst.md §12` encodes this prioritization: HEC conformance (Phase 1) is highest unblocking value; pipeline decoupling (Phase 2) is highest reversibility concern; SPL and storage (Phase 3) is highest strategic value for the detection engineer and deployer audiences.

Bundling decisions — which features ship together — are driven by the audience columns in `§1`. The Strap and Shank bundles (defined in `docs/Sparst.md §10`) are the natural bundle units: each is a complete vertical slice for a specific audience, and each can be validated independently. A feature that adds value to Relay but not to Strap does not ship in a Strap release.

## 6. Retrospective evaluation method

Components built without a prior written specification require a different evaluation approach. The risk is circular: reading the code and writing a specification from it produces a specification that ratifies whatever the code does, including its defects. The method below, carried forward from `spank-py/Process.md Appendix C`, breaks that circularity. It applies to any spank-rs crate whose implementation preceded its specification.

The method has five steps applied to one crate or subsystem at a time.

Step 1 — Write the specification without reading the implementation. Sources: the protocol or standard the component is based on (for HEC: Splunk documentation, Vector's HEC sink, OSS implementations); first-principles derivation from the component's stated purpose; the Reference doc that owns the subsystem. The rule: if you have not read the implementation in the past 24 hours, your recollection is close enough to outside-in. Read the external references instead.

Step 2 — Derive evaluation criteria from the specification. Before looking at the code, convert the specification into a checklist of requirements — each one falsifiable against the implementation. Three categories: correctness requirements (must do X); error requirements (must respond to Y with Z); boundary requirements (must behave correctly at scale N or under condition C).

Step 3 — Audit the implementation against the checklist. For each requirement: implemented and tested (pass); implemented but untested (gap — code exists, test absent); not implemented (miss — behavior absent); implemented incorrectly (defect — code is wrong).

Step 4 — Classify and route findings. Pass findings require no action. Gap findings become test tasks in `Plan.md`. Miss findings become implementation tasks if in scope for the current phase, or deferred items if not. Defects become priority fixes.

Step 5 — Size the effort. From the findings, derive: how many miss and defect items exist; which block publication; which block the confirmed use cases; which are quality improvements with no blocking effect. The sizing produces the task estimate for the retrospective work.

The method applies to all nine tracks from the initial implementation pass. The two open gaps documented in `docs/Errors.md §6` (Drain::wait return unchecked; TCP silent drop without counter) are the first two outputs of applying this method to the shutdown and TCP receiver subsystems.

---

## Part B — Direct application to spank-rs

## 7. Documentation governance

The documentation system for spank-rs is defined in full in `Docs.md`. This section states the principles behind the rules; the mechanics are in `Docs.md §2–§7`.

The four document classes — Foundation, Reference, Plan, Research — correspond to the four stages of the decision sequence: Foundation captures structural commitments (methodology, design target, documentation system itself); Reference captures stable technical contracts (error taxonomy, network stack, metrics names); Plan captures in-flight tracking; Research captures pre-verdict investigation. Content placed in the wrong class creates drift because it inherits the wrong update cadence and the wrong audience expectation.

The most common placement failure is writing status text in a Reference document. A Reference document's audience expects it to be stable; a status note tagged to a date or a task ID violates that expectation silently and erodes the document's credibility as a reference. The correct home for status is `Plan.md`; the correct home for a resolved decision is the Reference document, stated as an undated fact with the reasoning that produced it.

The central-map-only rule — only `Docs.md §4` enumerates documents — exists because every document that carries its own "see also" list creates a maintenance surface. When a document is renamed or removed, every such list must be updated. A single authoritative map updated in the same commit as the change is the only sustainable approach.

Cross-references resolve to numbered sections, not to file names. `docs/Errors.md §3` is a reference; `docs/Errors.md` is not. The distinction matters because a file name reference becomes a dead reference the moment the target section moves or is split; a section-numbered reference fails the compliance check immediately.

## 8. Coding standards

The coding standards for spank-rs are derived from `research/Stast.md` (Rust coding standards survey) and `research/Pyst.md` (Python-Rust comparison). The rules stated here are the code-actionable subset; rationale is in those documents.

**Naming.** Public API names use standard Rust conventions: `UpperCamelCase` for types and traits, `snake_case` for functions and variables, `SCREAMING_SNAKE_CASE` for constants. Abbreviations in type names are permitted only when the abbreviation is universally understood in the domain (`Hec`, `Tcp`, `Api`). Generic names (`Worker`, `Handler`, `Manager`, `Processor` used alone as a complete type name) are rejected; every type carries a role-specific name that indicates what it is responsible for. This rule carries forward from the Python port's Standards M03–M05 and M32.

**Error handling.** All library functions return `Result<T, SpankError>`. `unwrap()` and `expect()` are banned in library code. Every I/O error uses the `SpankError::io(syscall, target, source)` constructor — the syscall name and target string are mandatory, not optional. `panic!` in library code is allowed only to enforce a precondition that the caller has already been told is a precondition; it is not a substitute for a `Result` return. See `docs/Errors.md §1` for the full taxonomy.

**Backpressure.** Bounded channels are mandatory; unbounded channels require a design note in the relevant Reference document explaining why the bound cannot be applied. All sends use `try_send`, never `.await` on a `send`. A full channel returns `SpankError::QueueFull` and the caller returns a `503` or sheds load. An `await` on a full channel hides the backpressure signal until the queue drains; by then the information is worthless. See `docs/Errors.md §3`.

**Observability.** Every metric name is a constant from `spank-obs::metrics::names`. No metric name is constructed as a string literal at a call site. Log macros are `ingest_event!`, `lifecycle_event!`, `error_event!`, or `audit_event!` from `spank-obs`; direct `tracing::` calls at call sites without the category tag are a style violation. See `docs/Observability.md §1–§2`.

**Testing.** The process environment is shared across parallel tests; any test that sets an environment variable must either restore it in a `finally`-equivalent or be serialized with tests that read the same variable. The reference fix is in `spank-cfg::lib::defaults_and_validation` (combined sequential test replacing two parallel tests that raced on `SP_HEC__QUEUE_DEPTH`). Dynamic ports, not fixed ports, in integration tests. No `sleep` as the primary synchronization primitive.

**Cargo workspace.** Adding a new crate-level dependency requires a rationale entry in `docs/Network.md` (for network-related crates) or a comment in the relevant `Cargo.toml` (for all others). Adding a workspace dev-dependency (`criterion`, `serial_test`) requires explicit approval — these affect every crate's build and test time. Feature flags for optional backends (DuckDB, Postgres, `tokio_unstable`) are the mechanism for keeping the default build minimal.

**Formatting and lint.** `cargo fmt` and `cargo clippy -- -D warnings` are clean on every commit. Clippy allows are permitted only with a comment explaining why the lint is wrong for this specific case, not as a blanket suppression.

## 9. Plans and gates

`Plan.md` is the single tracking document for all in-flight work. This section defines the schema, the phase assignment model, and the gate criteria for the phases in `docs/Sparst.md §12`.

**Work item schema.** Each work item has: an ID in the form `CODE-L#` (e.g., `TCP-DROP1` for TCP domain, DROP chain, item 1); a status (`open`, `in-progress`, `done`, `deferred`); an owner (developer or model session); a target (the crate, file, or function affected); and a one-line description. The full ID scheme — domain codes, chain letter conventions, and the `ARCH` standalone code — is defined in `Plan.md Appendix A`. Items that cannot be bounded to a single target are a signal that the item needs to be decomposed.

**Open questions.** An open question is a work item whose resolution requires a decision, not an implementation. Its status is `open` until a decision is made; the decision closes the item and the resolution moves to the relevant Reference document. Open questions must not accumulate as undecided text in Reference documents — they belong in `Plan.md` with a one-line stub in the Reference doc pointing to the `Plan.md` item.

**Phase gates.** Each phase in `docs/Sparst.md §12` has a stated exit criterion. A phase does not close until its criterion is met. The current phase (Phase 0 — Spine) exits when CI is red on every current violation of the standards defined in `§8` above. The list of reds at Phase 0 exit is the Phase 1 backlog.

**Deferred items.** A deferred item is a known gap that is explicitly out of scope for the current phase. It has an ID, a one-line description, and a stated condition for re-opening. Deferreds that have no condition for re-opening are deletions waiting to be named.

**Five decisions from the initial implementation pass.** These were flagged in `Tracks.md` and are recorded in `Plan.md §1` as open items for Phase 0 resolution:

1. `SHIP-JIT1` — Jitter for the shipper's exponential backoff. See `docs/Network.md §10`.
2. `TCP-BP1` — TCP receiver backpressure: replace `.await` send with `try_send`, add dropped-line counter. See `docs/Errors.md §6`.
3. `API-STUB1` — Order of 501-stub implementation. Drives Phase 2 and 3 sequencing.
4. `ENG-DEP1` — `criterion` and `serial_test` as workspace dev-dependencies.
5. `STOR-BACK1` — DuckDB and Postgres backend stubs: early feature-gated stubs or land with implementation.

## 10. Working rules

One task per session. The task is stated at the start; the session ends when that task's deliverable is complete or a named blocker is hit.

Developer judgment gates are: anything requiring credentials, external action, publication, or a strategic direction choice. The model does not cross these without explicit instruction. When a gate is reached, the model states it and stops.

Model autonomy scope: document edits, code implementation, research synthesis, and evaluation that are within the current task. The model does not self-assign new tasks, resequence the plan, or initiate a new concern without naming it and waiting for direction.

Publication gates: a Reference document update is required before any implementation is considered complete. Code without a corresponding document update is incomplete, not done. Nothing publishes (crates.io, GitHub release, community post) without developer authorization.

At session start: state the task and the one condition that would require developer input. At session end: state what was done, what was not done, and the next task.

Iteration bounds: a task is allocated a maximum of three implementation-review passes before the model stops and states what is blocking convergence. The developer then decides whether to restate the requirement, change approach, or accept the current state.

---

## Appendix A: Comparison with prior and adjacent methodologies

This appendix exists to orient a developer or model who is familiar with one of these frameworks and wants to understand how the methodology in this document relates. Cross-references are to the specific technique, not to the framework as a whole.

**Lean Startup (Ries, 2011).** The three-stage gate in `§4` maps directly onto the Build-Measure-Learn loop: Stage 1 is Measure (hypothesis → minimum test → evidence), Stage 2 is Learn (verdict → strategy update → pivot conditions), Stage 3 is Build (bounded task → implementation). The PMF path in `§5` corresponds to Ries's engine-of-growth model: Demo activates the viral engine; MVP completes the four-audience bridge; MVF is the sticky-engine threshold. The dual-perspective table in `§1` is a formalization of the user-needs-first starting point that Ries describes as the precondition for a valid experiment.

**BDD / TDD.** Behavior-driven development's "given/when/then" language maps to the retrospective evaluation method's Step 2 (boundary requirements). Test-driven development's red-green-refactor cycle maps to the engineering cycle's implement-test-fix-document sequence. The distinction is that BDD/TDD operate at the test level; the methodology here operates at the requirement level — the specification is written before the tests, and the tests are derived from the specification, not the other way around.

**Shape Up (Basecamp).** The six-week cycle and appetite concept are analogous to the phase model in `docs/Sparst.md §12`. Shape Up's "betting table" and "unscheduled" distinction map to `Plan.md`'s open vs. deferred status. Shape Up rejects backlogs; this methodology maintains an explicit deferred list with re-opening conditions. The difference reflects scale: Shape Up is designed for teams; the spank-rs project is a developer-model pair where the deferred list is not a negotiating surface but a memory aid.

**C4 model (Brown).** The C4 model's four levels (Context, Containers, Components, Code) correspond to the spank-rs document layers: Context is `docs/Sparst.md §3` (Splunk role mapping); Containers is `docs/Sparst.md §7` (Strap, Shank, Relay deployment units); Components is `docs/Sparst.md §11.5` (seed component diagram); Code is the crate-level rustdoc. The C4 separation of levels maps onto the separation between Foundation documents (Context, Containers) and Reference documents (Components, Code).

**Hoshin Kanri (Akao, 1991).** The X-matrix structure of Hoshin Kanri — breakthrough objectives, annual objectives, improvement priorities, resources — maps onto: flywheel (breakthrough), phase exit criteria (annual objectives), `Plan.md` items (improvement priorities), track assignments (resources). The bidirectional traceability requirement (every task traces upward to a breakthrough objective, and every breakthrough objective traces downward to at least one task) is the discipline enforced by the phase-gate model in `§9`.

**ISO 25010.** The quality model in ISO 25010 names eight quality characteristics: functional suitability, performance efficiency, compatibility, usability, reliability, security, maintainability, portability. The coding standards in `§8` address maintainability (naming, error handling, testing) and reliability (bounded channels, backpressure, graceful shutdown). Security (STRIDE analysis in `§3`) and portability (zero runtime deps, static binary target) are addressed at the architectural level. The ISO 25010 vocabulary is useful for stating what a requirement is about; it is not a process framework.

**Agile Manifesto / Scrum.** The agile values (working software over comprehensive documentation, responding to change over following a plan) are partially inverted here: documentation is a first-class deliverable, not an afterthought, because the project's primary consumer of its own documentation is the model session that implements the next increment. The plan is not followed blindly, but changes to the plan are gated — a change to `Plan.md` that adds scope without corresponding authority from a phase exit criterion is a scope creep event, not an agile response. The iteration is bounded by phase, not by a fixed time-box.

**Jidoka / andon.** The gate-as-poka-yoke principle is the direct application: a gate that requires a research verdict before a strategy decision is a device that makes it impossible to ship without validation. The "open question" status in `Plan.md` is an andon cord: pulling it (creating an open question without a corresponding strategy resolution) stops work on the affected task until the question is answered. Ignoring an open question is the equivalent of disabling the andon — the defect propagates downstream.

**RUP (Rational Unified Process).** The RUP phases (Inception, Elaboration, Construction, Transition) are loosely analogous to the Spank phases in `docs/Sparst.md §12`: Phase 0 (Spine) is Elaboration — establishing the architecture and eliminating the highest risks; Phases 1–3 are Construction; Phase 4 is Transition (first real users). RUP's emphasis on use cases as the primary organizing artifact maps onto the audience-need columns in `§1`. RUP's heavyweight artifact model (vision document, software architecture document, risk list, iteration plan) is not adopted; this project uses the minimum set of documents needed to keep the model session consistent with the developer's intent.

---

## Appendix B: Audit record

Structural decisions about this document, retained as a record of why it is shaped as it is.

**2026-04-28** — Procesp.md created for spank-rs. Process.md in spank-py reviewed. The following were recovered and restated: dual customer-vendor perspective (`§1`), structural position and flywheel (`§2`), full methodological underpinnings with citations (`§3`), three-stage gate decision sequence (`§4`), PMF path with demo/POC/MVP/MVF stages and bundling rationale (`§5`), retrospective evaluation method (`§6`). Part B added as direct application: documentation governance (`§7`), coding standards derived from Stast.md and Pyst.md research (`§8`), plan schema and five open decisions from the initial implementation pass (`§9`), working rules (`§10`). Appendix A written to cross-reference and compare Lean Startup, BDD/TDD, Shape Up, C4, Hoshin Kanri, ISO 25010, Agile/Scrum, Jidoka, and RUP. Content that is purely spank-py-specific (document map for the Python project, Python coding standards, per-module routing rules) was left in Process.md and is referenced by section rather than duplicated.

The flywheel diagram in `spank-py/Process.md §1` is the source diagram for `§2` of this document; it is referenced, not reproduced.
