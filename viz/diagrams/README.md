# Diagrams

This directory holds the architectural diagrams for Spank. The formats and maintenance policy are defined in `Spast.md` §11.

## Layout

- `docs/diagrams/*.mmd` — Mermaid flowcharts, sequence diagrams, component diagrams.
- `docs/uml/*.puml` — PlantUML class diagrams and state machines.
- `docs/graphs/*.dot` — Graphviz entity relationships and data-flow graphs.

## Current files

- `components.mmd` — top-level component diagram (Ingest / Control / Egress). Seed from Spast §11.5.
- `ingest-sequence.mmd` — HEC ingest sequence through indexer ack.
- `threads.mmd` — long-lived thread inventory with stop mechanisms.
- `../uml/hec-readiness.puml` — `HECPhase` state machine (file kept for filename stability; `@startuml` id is `hec-phase`). Seed from Spast §11.6.
- `../uml/bucket-lifecycle.puml` — bucket HOT/WARM/COLD lifecycle. Seed from Spast §11.7.
- `../uml/search-job.puml` — SearchJob dispatchState machine; Splunk-compatible names.
- `../graphs/entities.dot` — entity relationships (Index/Bucket/Record, Token/Channel/Ack, Principal/Role).

## Rendering

Not yet wired into CI. Local commands:

```
mmdc -i docs/diagrams/components.mmd -o build/diagrams/components.svg
plantuml -o ../../build/uml docs/uml/hec-readiness.puml
dot -Tsvg docs/graphs/entities.dot -o build/graphs/entities.svg
```

CI rendering is a Phase 0 task in Spast §12.

## Maintenance

Each file carries a header naming the source-of-truth section in `Spast.md` and the owner modules. A PR that changes an owner module reviews the diagram. See Spast §11.3.

State-machine diagrams pair with Python `enum.Enum` declarations in the owner module and an allowed-transition set. The `pytest` state-machine check enforces correspondence. See Spast §11.4.
