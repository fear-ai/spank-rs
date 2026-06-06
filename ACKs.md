# ACKs.md — Discussion Record: ACK Content Placement and Document Design

Session: 2026-03-26. Participants: Developer, Model.

This document records the complete back-and-forth on ACK content placement, including the sequence of decisions, corrections, disagreements, positions taken and revised, and points that remain unsettled between Developer and Model.

---

## The Starting Condition

Product.md §24.5 existed as "HEC Ingest Reuse Inventory — LOG-C1". It contained:
- A component inventory table mapping `receiver.py` components to their Strap fitness
- An ACK protocol gap paragraph giving shipper-specific configuration guidance (Vector: `acknowledgements.enabled = false`; Fluent Bit: ACK not used by default)
- A pytest-spank reuse verdict
- A drain/synchronization note (`indexer.await_committed()`)

The Developer requested: "Review Product and other design and implementation documents for opportunities to move to HEC.md directly related functional, integration, deployment, and product concerns and decisions."

---

## First Move: Model Migrates to HEC.md

The Model moved the component inventory, ACK gap paragraph, drain primitive, and pytest-spank verdict into HEC.md:
- Component inventory + ACK gap + drain primitive → new HEC.md §6.8
- pytest-spank verdict → HEC.md §11

Product.md §24.5 was replaced with a pointer: "Component inventory... are in `HEC.md §6.8` and `HEC.md §11`."

**Developer response:** "Reconsider heavily referencing material that was moved out of a document as not belonging there, according to the project documentation design and inclusion rules."

This was the first clear statement of the anti-pattern: a section whose only function is to point to where its content went is a dead section. The Model had created exactly that.

---

## Second Move: Model Removes the Back-Pointer

The Model recognized the anti-pattern and revised §24.5 to contain only the content that belongs in Product.md — path status and the deployer ACK constraint — without pointing back to HEC.md §6.8.

The ACK gap paragraph (shipper config guidance) was removed from HEC.md §6.8 because that content belongs in Product.md deployer reference, not in an architecture section. It was already present in the §7 shipper compatibility table.

---

## Third Move: Product.md §7 ACK Appears Twice

Developer: "Product.md §7 ACK issue repeated twice."

The two locations:
1. Con row: "no native acknowledgment without ACK protocol extension"
2. Open questions row: "ACK protocol: shippers configured with `enable_ack=true` require a poll endpoint; without it events may be silently lost on restart. Implement after TLS is in place. ACK design detail: `HEC.md §15`."

Model position: The Con row is a protocol tradeoff appropriate at product decision table level. The Open questions row is implementation status masquerading as an open question — the question is answered (ACK is deferred, tracked in Plan.md DEF-1), and the `HEC.md §15` cross-reference is implementation depth not needed in a product interface table.

Action taken: Open questions row deleted. Con row retained, reworded to include deferred status: "at-least-once delivery requires ACK protocol extension (deferred — Plan.md DEF-1)".

Developer accepted this.

---

## Fourth Move: Delete §24.5

Developer: "Delete Product.md §24.5."

The section at that point contained: path status (complete/tested) and the ACK constraint for deployers. The Model's position was that both items had homes elsewhere:
- Path status is implementation tracking → Plan.md, not a deployer reference section
- ACK constraint and shipper config is already in §7 shipper compatibility table

Section deleted. TOC updated. §24 lead-in updated to remove the reference to "HEC component reuse map."

---

## Fifth Move: "A Dozen ACK Mentions is Terrible"

Developer: "A dozen ACK mentions in HEC.md is terrible, makes working through the topic difficult. There is no goal to slice the HEC implementation every possible way effectively creating MxN matrix of topics and components."

The Model initially accepted this framing and wrote DOC-J2 as: "HEC.md has grown by accretion into a cross-cutting topic matrix (ACK alone appears in §4, §5, §6.1–6.8, §12, §15, §17). Restructure by topic rather than by component."

Developer then challenged: "DOC-J2 description of the two dimensions incorrect — review actual mentions of ACK in the document and identify what all those sections are about and why they include ACK."

---

## Model Self-Correction: The MxN Diagnosis Was Wrong

After reading every ACK mention with line numbers, the Model revised its position.

**What the mentions actually are:**

ACK has a complete vertical slice in HEC.md — wire spec (§4.7), requirements (§5.6), architecture (§6.4), design (§7.3). Each section adds a distinct layer. This is not fragmentation; it is the correct structure for a protocol feature that must be specified, implemented, and tested.

The remaining mentions are incidental appearances of ACK within sections covering other topics:
- §6.1–6.3, §6.5–§6.7: ACKTracker appears as one component in sections covering all HEC components/threads/flows. Unavoidable.
- §6.8, §7.1: ACK appears in implementation status and testability design. Contextual.
- §8, §9, §10: One row each in error, config, and metrics sections. Correct placement.
- §11, §12: Prioritization and evaluation. Correct placement.
- §13: External findings about ACK behavior. Correct placement.

**Revised Model position:** The problem is not MxN fragmentation. HEC.md was authored in sequential passes — wire protocol, then requirements, then architecture, then design, then evaluation, then external findings. Each pass correctly placed ACK content within its layer. What looks like twelve ACK sections is actually one feature covered at six distinct depths, plus incidental one-line appearances in sections about other things.

**The actual structural problems are narrower:**
1. Open design questions left inline in §6.6.5 Q1–Q7 instead of Plan.md §4 (Q6, callback vs. poll for ACK commitment, is clearly still open)
2. §7 Design subsections of uncertain added value over §6 Architecture (to be audited in DOC-J2)
3. §13.4 discrepancies not applied back to §4/§5

DOC-J2 was rewritten to reflect this corrected diagnosis.

---

## Points Settled

| Point | Resolution |
|-------|-----------|
| Product.md §7 ACK — two mentions | Open questions row deleted; Con row retained with deferred pointer |
| Product.md §24.5 | Deleted entirely |
| Back-pointer anti-pattern | Identified and corrected: sections that only point to extracted content are dead sections |
| ACK shipper config guidance | Belongs in Product.md §7 deployer table, not HEC.md §6 architecture |
| pytest-spank verdict | Belongs in HEC.md §11 (Prioritization), not Product.md |
| Drain primitive (`await_committed()`) | Belongs in HEC.md §6.8 (Architecture), not Product.md |
| "MxN fragmentation" diagnosis | Wrong. HEC.md has correct layered coverage, not topic scatter. Corrected in DOC-J2. |
| DOC-J2 scope | Not a consolidation pass. A structural review: §7 vs §6 overlap, §6.6.5 Q migration, §13.4 discrepancy application. |

---

## Points Unsettled or Incompletely Resolved

**1. Where does current implementation status live?**

HEC.md §6.8 is titled "Component Inventory and Integration Reuse" and contains a table of current implementation state for receiver.py components. This is implementation tracking content that could equally live in Plan.md §3 (issues), Plan.md §5 (deferred), or Infra.md. The Model placed it in HEC.md §6 because the Developer requested HEC implementation content moved into HEC.md. Whether a current-state snapshot table belongs in an architecture section was not directly challenged or resolved. It may be revisited when DOC-J2 is executed.

**2. §7 Design vs §6 Architecture layer distinction**

The question of what §7 adds over §6 was identified as a task (DOC-J2 step a) but not yet answered. The Model's working hypothesis is that §7.1 (HECHandler Extraction) and §7.3 (ACKTracker Design) clearly add implementation detail not in §6. §7.8 (Connection Handling Design) was flagged as potentially overlapping §6.2. This remains to be read and decided.

**3. HEC.md §6.6.5 Q6 — callback vs. poll for ACK commitment**

Q6 asks whether ACK commitment notification from the indexer to ACKTracker should be callback-driven or poll-driven. The Model assessed this as clearly open and flagged it for Plan.md §4. The Developer has not confirmed this assessment. It is possible Q6 was resolved in earlier sessions and the answer exists in the code or in a prior document.

**4. How many of §13.4 discrepancies are already applied to §4/§5?**

§13.4 contains discrepancy rows including "R25 already scoped correctly." The Model identified the pattern of discrepancy rows that say "already correct" as cleanup candidates. Whether all of them are truly already correct, or whether some require §4/§5 edits first, was not checked in this session.

**5. ACKs.md file — original intent**

The Developer asked for "the entire ACK argument" documented in ACKs.md — meaning this session's discussion, Q&A, disagreements, Model final position, and points likely unsettled. This document is that record. Whether it becomes a standing reference, a session closure artifact only, or feeds into a Process.md section on document design discipline was not specified.

---

## The Document Design Principle Demonstrated

The ACK discussion surfaced a general principle that was applied several times:

**A section that only points to where its content went is a dead section.** When content is extracted from a document because it belongs elsewhere, the source section either retains content that genuinely belongs there or is deleted. A residual pointer section adds navigation overhead without adding information — the reader who wanted the content follows the pointer; the reader who wanted the source document's own content finds nothing.

This was applied to Product.md §24.5 (deleted) and to the back-reference pattern the Model initially created after the migration.

A second principle that emerged but was not explicitly stated until corrected:

**"Appears in many sections" is not the same as "fragmented."** A feature with a complete specification, requirements, architecture, and design has correct multi-section coverage. The test is whether each section adds a distinct layer. If yes, the coverage is correct. If a section restates another without adding a layer, that is the fragmentation to fix — not the count of sections.
