# Docs — Documentation System for spank-rs

`Focus: foundation` — this document is the central documentation map of the system.  The audience is the developer and model that edits and reviews documentats.  Every other document gets the rules for content inclusion and presentation from Docs, that solely enumerates other documents.

---

## Table of Contents

1. [Goals and scope](#1-goals-and-scope)
2. [Document classes](#2-document-classes)
3. [Document format](#3-document-format)
4. [The documentation map](#4-the-documentation-map)
5. [Cross-reference rules](#5-cross-reference-rules)
6. [Procedural workflow](#6-procedural-workflow)
7. [Compliance system](#7-compliance-system)
8. [Relationship to spank-py](#8-relationship-to-spank-py)

---

## 1. Goals and Scope

The documentation system has three goal:

1) Enumerate and define the rules for all project documents.

2) Stable reference separate from frequently updated planning material.

3) Keep decisions visible and distinct.

The scope of this document is the documentation system. The reasoning framework, methodology, and project lifecycle that govern will live in `Procesp.md`.

## 2. Document classes

Documents in the tree belong to one of four classes declared by a `Focus:` line in the intro paragraph.

The **foundation** class holds project framing and the definition of the system itself: what the project is, who it is for, how decisions are made, and where each kind of content lives. Foundation documents change rarely — typically when the project's framing changes or when the documentation system itself is revised. The audience is a first-time reader, reviewer, and Model orienting at start. Examples include this file (`Docs.md`), `Procesp.md`, and the future `README.md`. Foundation documents are deliberately short and cross-reference others; the other document classes do not paraphrase Foundation content.

The **reference** class holds stable technical truths: how a subsystem is designed and implemented, what industry patterns and standards are used, contracts and protocols, error taxonomy and the network library choice and operating mode. Reference documents change when a design decision changes. The audience is a developer working on the subsystem the document owns. Reference documents accrete domain detail but not intermediate status or dated annotations.

The **plan** class holds active tracking: tasks, open issues, questions awaiting decision, deferred items, in-flight work logs. Plan documents change frequently, even daily during active work. The audience is the Developer and the Model resuming work.  Here `Plan.md` at the root is the singular planning document that has a strict internal structure and schema (work item ID format, status vocabulary, fixed columns).  It does not contain design rationale that lives in the Reference doc.

The **research** class holds investigatory and analytical material like: surveys of alternatives, Python-to-Rust comparisons, infrastructure option analyses, storage backend evaluations. Research documents become stable buti are not contracts — they are the analytical input that produced Reference docs. They will change when new investigations occur or when a prior analysis proves wrong. The audience is Developer or Model seeking context on a design choice or the alternatives considered. Research documents make no authoritative claims about the implementation state; those live in Reference docs. In this tree the research class documents live under `research/` at the project root.

The four classes form a flow: Foundation is the static anchor for the system.  Research produces design choices that resolve into Reference; open questions and definition and development status go into Plan.

## 3. Document format

Every document in the tree follows the same shape so that a reader knows where to look without reading the file end to end. The format has top matter, a body of numbered sections, and bottom matter. Each part has a fixed structure.

### 3.1 Top matter

The top matter has four elements in this order. The H1 title states the document name and a short tagline separated by an em dash, for example `# Procesp — Reasoning Framework and Documentation System`. The first paragraph is the scope and audience: it states the focus label (`Focus: foundation` / `reference` / `plan` / `research`), the subject the document owns, the audience that reads it, and what the document does *not* receive — naming the correct home for the most likely misdirected content. The second paragraph names what triggers an update to the document and (for Reference and Plan) what its sibling documents are.

A horizontal rule (`---`) separates the intro from the Table of Contents. The Table of Contents follows immediately. Every numbered section in the body appears in the ToC; appendices and the References, Glossary, and Index sections appear after the numbered list under their own headings.

### 3.2 Body

The body is a sequence of numbered top-level sections (`## 1.`, `## 2.`, …) introduced by a single horizontal rule. Subsections are numbered (`### 1.1`, `### 1.2`, …) and may nest one level further (`#### 1.1.1`) when the topic warrants it; deeper nesting is a decomposition signal that the section should be split rather than nested.

Every section opens with a lead-in sentence or paragraph before any list, table, or code block. A section that begins with a bullet list is a structural failure — the lead-in is what tells the reader why the list is there and what it enumerates. Subsections appear only when a topic has at least three substantive sentences beneath them; fewer means the heading is decoration and the content belongs in its parent.

Tables are introduced by a sentence that names the columns and what comparison the table supports. Code blocks are introduced by a sentence that names the language and what the snippet illustrates. Diagrams (ASCII or fenced ` ```mermaid ` blocks) are introduced by a sentence that names what they show and from whose perspective.

### 3.3 Bottom matter

The bottom matter has up to four sections in this order, separated from the body and from each other by horizontal rules.

The **References** section is mandatory for Foundation and Reference documents and optional for Plan and Research documents. It enumerates external sources cited in the body — books, papers, RFCs, standards, OSS projects whose code or design is invoked — in a numbered list. Each entry is `[N] Author, *Title*, year, identifier or URL`. The body cites references by `[N]` rather than by parenthetical author-year, so the body remains scannable and the citation list remains the single source of truth for sources.

The **Appendices** section is optional. Appendices hold material that supports the body but would dilute it if inlined: long worked examples, derivations, evaluation procedures, session records, historical notes. Each appendix is `## Appendix A — <Title>`, with internal subsections numbered `A.1`, `A.2`, and so on. Appendices are referenced from the body by `Appendix A.2` or `Appendix B`; never as "see below" or "see appendix".

The **Glossary** section is optional and recommended for Foundation and Reference documents that introduce non-obvious terminology. Each entry is a bolded term followed by one or two sentences. Terms appear in the order in which they first occur in the body, not alphabetically; this lets a reader scan the Glossary as a reading order.

The **Index** section is optional and only worthwhile for documents over ~1500 lines. When present, it lists significant terms with their section numbers.

### 3.4 Style and voice

The voice is analytical and expository: state the scope and purpose first, give exact values and code paths, cross-reference inline. Explain "why" alongside "how"; offer trade-offs and alternatives. Direct, factual, professional. No emoji, no Unicode symbols (checkmarks, warning triangles, status circles) anywhere in the prose, tables, or status fields — use words. No filler ("we will see that", "as mentioned earlier"); no decorative repetition; no commentary or parenthetical remarks in headings.

Cross-references inside a section use file-and-section form (`Procesp.md §3.2`); cross-references inside the same document use the section form alone (`§3.2`). Vague references ("see above", "as discussed", "in the relevant section") are forbidden — the compliance check in `§7` flags them.

## 4. The documentation map

The documentation map is the authoritative list of every document in the tree. No other document in this tree carries a list of documents; references to peers go to specific sections, not to file names with paraphrased descriptions. When a document is added, renamed, or removed, this section is updated in the same change.

The tree is laid out as follows. The top-level directory holds Foundation and Plan documents at the project root. Reference documents live under `docs/`. Research documents live under `research/` at the project root. Source code lives under `crates/` and is governed by coding standards, not by this map.

```
spank-rs/
  README.md                 (foundation; orientation, quick start)
  Docs.md                   (foundation; this file — system definition and map)
  Procesp.md                (foundation; reasoning framework and methodology)
  Plan.md                   (plan; work tracking)
  docs/
    Observability.md        (reference; logs, metrics, profiling baseline)
    Errors.md               (reference; error taxonomy, recovery, shutdown)
    Network.md              (reference; network library stack)
    Sparst.md               (foundation; fresh implementation proposal for the Rust port)
    ExpRust.md              (research; Rust syntax and codebase study transcript)
  research/
    Stast.md                (research; Rust coding standards survey)
    Pyst.md                 (research; Python-to-Rust comparison and gap analysis)
    Infrust.md              (research; Rust infrastructure counterpart to spank-py Infra.md)
    Indust.md               (research; storage backend analysis, industry reading list)
  crates/                   (source; not part of the documentation system)
```

Each entry below names what the document owns, who reads it, and what it does *not* receive. Once a document exists, its rules of placement are these — not its own intro paragraph, which paraphrases this map. If the description below conflicts with a document's intro, this map wins and the document is updated.

**`README.md`** — orientation only. Quick start, development commands, license, pointer to `Docs.md`. Does not receive design rationale, status, or any list of documents beyond the pointer to this one. *(Foundation; not yet written; deferred until the project ships.)*

**`Docs.md`** (this file) — the documentation system: classes, format, map, cross-reference rules, compliance. Does not receive project methodology (that is `Procesp.md`) or work tracking (that is `Plan.md`).

**`Procesp.md`** — the reasoning framework, working methodology, decision sequence, and engineering lifecycle for spank-rs. Reviews and adapts the upstream `spank-py/Process.md`, restoring the dual customer/vendor perspective and recovering material lost in summarization. Carves out a "Direct application to Spank" part covering coding standards, plans, and gates. Does not receive product strategy conclusions (those live in upstream `spank-py/Product.md` until and unless the Rust port forks positioning), implementation detail (Reference docs), or work items (`Plan.md`).

**`Plan.md`** — the single tracking document for active work: tasks, open issues, open questions, deferred items. Schema is fixed (work item ID, status, owner, target). Does not receive design rationale or resolved decisions; rationale belongs in the relevant Reference doc, and decisions are recorded there or in the relevant research file.

**`docs/Observability.md`** — the contract between the runtime and the operator: log macros, metric names, baseline workload, profiler choices. Does not receive task lists or status text.

**`docs/Errors.md`** — the `SpankError` taxonomy, the four recovery classes, the backpressure path, and shutdown composition (`Lifecycle`, `Drain`, `Sentinel`). Does not receive work items or per-call-site status.

**`docs/Network.md`** — the rationale for every network-library choice in the tree (axum, tokio, mpsc, tokio::net, flate2, rusqlite), what we deliberately omitted, and the inspection points for an external reviewer. Does not receive code snippets that belong inside the source crates' rustdoc.

**`docs/Sparst.md`** — the fresh implementation proposal for the Rust port: wins to preserve from spank-py, misses to overcome, Splunk alignment, persistence and durability, configuration and SPL, composability, terminology map, phased evolution, and open questions. Does not receive implementation status; status lives in `Plan.md`. *(Foundation because it defines the design target, not investigatory material.)*

**`research/Stast.md`** — a survey of Rust coding standards, style conventions, and toolchain best practices relevant to the port. Informs `Procesp.md Part B` without duplicating it.

**`research/Pyst.md`** — a detailed Python-to-Rust comparison covering data models, storage, networking, observability, concurrency, and the resulting gaps and reduced functionality in the current Rust port. The primary reference for understanding what spank-py does that spank-rs does not yet do.

**`research/Infrust.md`** — the Rust infrastructure counterpart to `spank-py/Infra.md`: deployment topology, configuration management, packaging, and operational considerations in the Rust port.

**`research/Indust.md`** — storage backend analysis (SQLite, DuckDB, Parquet, S3), the missing Partition layer design, Splunk on-disk formats, SPL functional requirements by tier, and a curated industry and technology reading list.

**`docs/ExpRust.md`** — study transcript from a session exploring `main.rs` structure (`Cli`, `Cmd`), Rust syntax patterns, and design decisions in the binary crate. Belongs in `docs/` rather than `research/` because it is tied to the current codebase, not to external analysis. Does not receive implementation tasks or rationale that belongs in `docs/Network.md`, `docs/Errors.md`, or `docs/Observability.md`.

**`crates/`** — source code, governed by `Procesp.md Part B` (coding standards), not by this map.

## 5. Cross-reference rules

Cross-references are the connective tissue of the system. They allow each document to stay short by deferring to the document that owns a topic, but only if they are precise. The rules below are mechanical — the compliance check in `§7` enforces them.

The first rule is that every cross-reference resolves to a specific section or appendix, never to a bare file name. `Procesp.md §3.2` is permitted; `Procesp.md` alone is not, because it imposes on the reader the cost of finding the relevant section. The exception is when the reference is genuinely to the document as a whole — for example, "the documentation system is defined in `Docs.md`" — and even then the section number is preferred when one applies.

The second rule is that no document other than this one (`Docs.md §4`) lists peer documents. A document that needs to refer to another document does so by section reference at the point of use; it does not carry a "see also" block, a "related documents" list, or a paraphrased description of what the other document contains. This is the single most important rule for keeping the map authoritative.

The third rule is on direction. Reference documents may cite research files for the rationale that produced their content. Reference documents do not cite `Plan.md` task IDs except as a one-line `*Open question:* see Plan.md §X.Y` stub for an unresolved question; once the question resolves, the stub is replaced by the resolved content. `Plan.md` may cite Reference documents and research files freely.

The fourth rule is on quoting. A document quoting more than three consecutive sentences from another document is doing so at the wrong level — it should be referencing that section, not paraphrasing it. The compliance check does not catch this directly; the cultural rule `DOC-R3` in `§7.2` does.

## 6. Procedural workflow

The system is applied through a fixed sequence of operations that ensure changes do not break the rules. The order matters: out of order, several of the steps either fail their own checks or produce empty artifacts.

When **adding a new document**, first decide its class (`§2`); then add its row to the documentation map in `§4` of this file *in the same change* that creates it; then write its top matter (`§3.1`) including the focus line; then its body. The doc is incomplete until the map row exists and the focus label is present.

When **adding a new section to an existing Reference doc**, update the document's ToC in the same change. If the new section depends on an open question, the section either carries the resolved content or it carries the one-line stub form. Drafting the section *and* its open-question discussion in the same file is forbidden by `DOC-C2`.

When **resolving an open question**, the resolution is a commit that removes the question from `Plan.md` and updates the relevant Reference or research section with the resolved content. Both changes are in the same commit; partial resolution is what produces the drift the system is designed to prevent.

When **renaming or moving a document**, update `§4` of this file, update every cross-reference in the tree (the compliance check finds dangling references), and leave a one-line stub at the old path for one release cycle if the file was published or referenced externally.

When **deleting a document**, update `§4` of this file (remove the row), confirm the compliance check finds no references to the removed file, and commit the removal. The deletion of `Tracks.md`, `docs/errors.md`, `docs/network-libraries.md`, and `docs/observability.md` in the change that creates the current file set is the worked example.

## 7. Compliance system

The compliance system is a small set of mechanical checks (`§7.1`) backed by a small set of cultural rules (`§7.2`). The mechanical checks fence the failure modes that grow silently — status drift, paraphrase duplication, dangling cross-references — and the cultural rules cover the failure modes that depend on judgment. Together they are sufficient; either alone is not.

### 7.1 Mechanical checks

Five rules numbered `DOC-C1` through `DOC-C5`. Each is stated as the *check* (what a script looks for), the *violation* (what fails), and the *remediation* (what the author does). The intended implementation is a single shell script `scripts/docs-check.sh` running `rg` against a fixed pattern set; the script has not yet been written, but the rules are stated such that any text-search tool produces the same answer.

**DOC-C1 — Focus label present.** Check: every Markdown file under the project root that is part of the documentation system carries a line matching `` `Focus: (foundation|reference|plan|research)` `` within its first thirty lines. Violation: missing label or label outside the allowed set. Remediation: add the label; if the file does not fit any class, split it.

**DOC-C2 — No status text in Reference.** Check: files labeled `Focus: reference` contain none of these case-insensitive patterns: `queued for review`, `\bTODO\b`, `to be decided`, `\btbd\b`, `\(20[0-9]{2}-[0-9]{2}-[0-9]{2}\)` (dated annotations), `^### Open Questions?$`, `^### Work Items?$`, `^## Current State and Next Steps$`. Violation: any match. Remediation: move the content to `Plan.md` and replace with a `*Open question:* see Plan.md §X.Y` stub, or, if the content is a resolved decision, incorporate it into the Reference section with a dated note.

**DOC-C3 — No design rationale in Plan.** Check: files labeled `Focus: plan` do not contain headings matching `^## (Architecture|Design|Rationale|Why|Trade-?offs?)$` or `^### (Architecture|Design|Rationale|Why|Trade-?offs?)$`. Violation: such a heading. Remediation: move the rationale to the relevant Reference doc; leave a one-line cross-reference in Plan.

**DOC-C4 — Cross-references resolve.** Check: every cross-reference of the form `<File>.md §<N>(.<M>)*` resolves — the file exists in the tree and a heading numbered `N` (or `N.M`) exists in it. The check also looks for vague reference patterns: `see above`, `as discussed`, `in the relevant section`, `(see [^§][^)]*\)` without a section anchor. Violation: dangling reference or vague reference. Remediation: fix the reference to a numbered section, or fix the section if the reference was right and the section moved.

**DOC-C5 — Forbidden glyphs and patterns in any class.** Check: no file contains emoji, Unicode symbols (checkmarks, warning triangles, status circles, fancy bullets like `→` or `•` outside fenced code blocks where they belong to ASCII diagrams), or H1 titles missing the em-dash tagline form. Violation: any match. Remediation: replace with words; ASCII art and Mermaid blocks are exempt because they use Unicode legitimately for diagram lines.

A document that adds content covered by these rules without violating them is in compliance regardless of the cardinality of the change. The checks operate on file content, not on commit metadata.

### 7.2 Cultural rules

Three rules that the script cannot enforce, numbered `DOC-R1` through `DOC-R3`.

**DOC-R1 — When you write a status note in a Reference doc, you have made a mistake.** The mechanical check `DOC-C2` catches the obvious patterns; the rule catches the subtle ones that read as well-formed prose. If you find yourself writing "this is currently…", "we plan to…", "this section is queued for review", or any present-progressive description of project state inside a Reference doc, stop and write it in `Plan.md` instead.

**DOC-R2 — Resolving a question records the decision.** When an open question in `Plan.md` is closed by a decision, the closing commit moves the resolution from `Plan.md` into the relevant Reference or research section as resolved content. The Plan entry is removed (not edited to "resolved"); the Reference or research file is the new home. Without this discipline `Plan.md` accretes resolved-decision content and the supposed Plan document becomes a hybrid.

**DOC-R3 — Quote a paragraph, reference a section.** A document quoting more than three consecutive sentences from another document is paraphrasing instead of referencing. Replace the paraphrase with `<File>.md §X.Y` and let the reader follow the link. The exception is when the quoted content is genuinely small enough that the reference would cost the reader more than the inline copy — generally a single short sentence that defines a term.

The combination of `DOC-C1` through `DOC-C5` (mechanical) and `DOC-R1` through `DOC-R3` (cultural) is the full compliance system. It is summarized in `Appendix A` for quick reference at review time.

## 8. Relationship to spank-py

The Python project (`spank-py`) is the upstream of this Rust port. Its `Process.md`, `Product.md`, `HEC.md`, `Architecture.md`, `Infra.md`, `Standards.md`, and the rest of its document tree are the source material from which the Rust port's framing decisions derive. This file does not enumerate the Python tree — that is the upstream's concern — but it acknowledges three points of contact.

The first is that positioning, audience definition, and the methodology that produced them are *inherited* from `spank-py/Product.md` and `spank-py/Process.md` until and unless the Rust port forks. If the Rust port forks positioning, a new `Product.md` is added to this tree at that time. Until then the Foundation document `Procesp.md` carries the methodology, citing the Python source where it adapts upstream content.

The second is that the research documents in `research/` (`Stast.md`, `Pyst.md`, `Infrust.md`, `Indust.md`) and the fresh implementation proposal at `docs/Sparst.md` were authored in the Python project's worktree or in model sessions against this tree, and live here because their content is Rust-specific. They retain their original form; they were not rewritten for this map. Research documents are stable but not contracts.

The third is that the diagnosis driving this system is documented in `Procesp.md Appendix A` ("Review of Process.md") and the migration recommendations for the Python project are in `Procesp.md Appendix B` ("Migration recommendations for spank-py"). They are recommendations, not commitments; the Python project may or may not adopt them.

---

## References

[1] Eric Ries, *The Lean Startup*, Crown Business, 2011.
[2] Donella Meadows, *Thinking in Systems: A Primer*, Chelsea Green, 2008.
[3] Yoshio Akao, *Hoshin Kanri: Policy Deployment for Successful TQM*, Productivity Press, 1991.
[4] Taiichi Ohno, *Toyota Production System: Beyond Large-Scale Production*, Productivity Press, 1978.
[5] W. Ross Ashby, *An Introduction to Cybernetics*, Chapman & Hall, 1956.
[6] Simon Brown, *The C4 Model for Software Architecture*, c4model.com, 2018.
[7] ISO/IEC 25010:2011, *Systems and software Quality Requirements and Evaluation (SQuaRE) — System and software quality models*.
[8] Tom Preston-Werner, *Semantic Versioning 2.0.0*, semver.org.

---

## Appendix A — Compliance rules summary

This appendix collects the rules from `§7` in a single table for quick reference at review time. The body of `§7` is the authoritative statement; this table is an index.

| Rule | Class | Check |
| - | - | - |
| DOC-C1 | mechanical | Focus label present in first 30 lines |
| DOC-C2 | mechanical | No status text in Reference docs |
| DOC-C3 | mechanical | No design rationale in Plan docs |
| DOC-C4 | mechanical | Cross-references resolve and are not vague |
| DOC-C5 | mechanical | No emoji or fancy glyphs outside diagram blocks |
| DOC-R1 | cultural | Status notes do not belong in Reference docs |
| DOC-R2 | cultural | Resolving a question records the decision |
| DOC-R3 | cultural | Quote a paragraph, reference a section |

---

## Appendix B — Document templates

This appendix gives the minimum skeleton for each document class. A new document copies the relevant skeleton and fills in the placeholders. The skeletons enforce the format from `§3` so an author cannot omit top or bottom matter by accident.

### B.1 Foundation document skeleton

```markdown
# <Name> — <Tagline>

`Focus: foundation` — <one-paragraph scope, audience, what it does not receive>.

<one-paragraph update trigger and sibling documents>

---

## Table of Contents

1. [Section name](#1-section-name)
...

---

## 1. Section name

<lead-in sentence>. <body>.

---

## References

[1] ...

---

## Appendix A — <Title>

<body>
```

### B.2 Reference document skeleton

```markdown
# <Name> — <Tagline>

`Focus: reference` — <scope, audience, what it does not receive>.

<update trigger>

---

## Table of Contents

1. [Section name](#1-section-name)
...

---

## 1. Section name

<lead-in>. <body>.

---

## References

[1] ...
```

### B.3 Plan document skeleton

```markdown
# <Name> — <Tagline>

`Focus: plan` — <scope, audience>.

---

## 1. Active work

| ID | Title | Status | Owner | Target |
| - | - | - | - | - |
| ... | ... | ... | ... | ... |

## 2. Open questions

| ID | Question | Blocking |
| - | - | - |

## 3. Deferred

| ID | Title | Reason |
| - | - | - |
```

### B.4 Research document skeleton

```markdown
# <Name> — <Tagline>

`Focus: research` — <scope, audience, what it does not receive>.

<one-paragraph update trigger and relationship to the Reference docs it informs>

---

## Table of Contents

1. [Section name](#1-section-name)
...

---

## 1. Section name

<lead-in>. <body>.

---

## References

[1] ...
```

---

## Glossary

**Focus.** The document class declared in a file's top matter, one of `foundation`, `reference`, `plan`, or `research`. The focus governs which content is allowed in the file and is enforced by `DOC-C1` and `DOC-C2`.

**Reference doc.** A document with `Focus: reference` — stable subsystem design and contract material under `docs/`.

**Plan doc.** A document with `Focus: plan` — active tracking material. Currently a single document, `Plan.md`.

**Foundation doc.** A document with `Focus: foundation` — project framing and the documentation system itself. Read by every reader.

**Research doc.** A document with `Focus: research` — investigatory and analytical material under `research/`. Stable but not a contract; the input that produced Reference docs and design decisions.

**Stub.** A one-line cross-reference left in a Reference doc when content moved out, of the form `*Open question:* see Plan.md §X.Y`. Stubs prevent dangling links and signal to a reader that the discussion is alive elsewhere.

**Supersession.** The act of replacing accepted design content with new content that explicitly notes what it supersedes. For research documents this is an in-place update that records the prior position and why it changed; for Reference docs the new content stands and the prior position is noted in the body.
