# HEC — Protocol Reference and Design Decisions

`Focus: reference` — the authoritative record of how spank-rs implements the Splunk HTTP Event Collector protocol: what the wire protocol requires, what the major receiver implementations actually do (verified from source), where implementations diverge, and how spank-rs resolves each divergence. The audience is a developer adding a feature to `spank-hec`, reviewing a protocol conformance question, or tracing the reasoning behind a specific code structure. Implementation status and task tracking belong in `Plan.md`; infrastructure library choices belong in `docs/Network.md`; error taxonomy belongs in `docs/Errors.md`.

This document changes when a protocol behavior is confirmed from a new source, when a design decision is revised, or when a deferred item in `Plan.md` is resolved and the resolution changes behavior here. Cross-references use document section and subsection.

---

## Table of Contents

1. [Wire protocol](#1-wire-protocol)
   - [1.1 Endpoints](#11-endpoints)
   - [1.2 Authentication](#12-authentication)
   - [1.3 Content-Encoding and body framing](#13-content-encoding-and-body-framing)
   - [1.4 Event endpoint — JSON envelope format](#14-event-endpoint--json-envelope-format)
   - [1.5 Raw endpoint](#15-raw-endpoint)
   - [1.6 Health endpoint](#16-health-endpoint)
   - [1.7 ACK endpoint](#17-ack-endpoint)
   - [1.8 Channel header](#18-channel-header)
   - [1.9 Error codes](#19-error-codes)
2. [Vendor survey](#2-vendor-survey)
   - [2.1 Sources examined](#21-sources-examined)
   - [2.2 Event field — null vs. absent vs. empty string](#22-event-field--null-vs-absent-vs-empty-string)
   - [2.3 Channel header — presence, format, enforcement](#23-channel-header--presence-format-and-enforcement)
   - [2.4 time field — numeric, string, and unit detection](#24-time-field--numeric-string-and-unit-detection)
   - [2.5 Tag derivation — channel, source, and fallback](#25-tag-derivation--channel-source-and-fallback)
   - [2.6 Content-Type handling](#26-content-type-handling)
   - [2.7 Endpoint path variants](#27-endpoint-path-variants)
   - [2.8 Indexed fields validation](#28-indexed-fields-validation)
   - [2.9 Health check body and status codes](#29-health-check-body-and-status-codes)
   - [2.10 ACK semantics](#210-ack-semantics)
   - [2.11 Metadata field carry-forward across envelopes](#211-metadata-field-carry-forward-across-envelopes)
   - [2.12 Host field derivation priority](#212-host-field-derivation-priority)
   - [2.13 No-token mode](#213-no-token-mode)
   - [2.14 Splunk SDK ingest paths](#214-splunk-sdk-ingest-paths)
   - [2.15 Shipper retry branches on HTTP status, not code](#215-shipper-retry-branches-on-http-status-not-code)
3. [spank-rs design decisions](#3-spank-rs-design-decisions)
   - [3.1 event field: null mapped to code 12](#31-event-field-null-mapped-to-code-12)
   - [3.2 Channel handling without ACK](#32-channel-handling-without-ack)
   - [3.3 Empty channel header treated as absent](#33-empty-channel-header-treated-as-absent)
   - [3.4 time field: fractional seconds only](#34-time-field-fractional-seconds-only)
   - [3.5 Channel UUID validation deferred](#35-channel-uuid-validation-deferred)
   - [3.6 Channel from query parameter deferred](#36-channel-from-query-parameter-deferred)
   - [3.7 Token comparison — constant-time gap](#37-token-comparison--constant-time-gap)
   - [3.8 Tag derivation: channel then token id](#38-tag-derivation-channel-then-token-id)
4. [Code structure](#4-code-structure)
   - [4.1 Module map](#41-module-map)
   - [4.2 Request pipeline](#42-request-pipeline)
   - [4.3 HecEnvelope deserialization — absent vs. null](#43-hecenvelope-deserialization--absent-vs-null)
   - [4.4 TimeField — number and string coercion](#44-timefield--number-and-string-coercion)
   - [4.5 Channel extraction in receiver](#45-channel-extraction-in-receiver)
   - [4.6 Queue and consumer](#46-queue-and-consumer)
5. [Deferred items](#5-deferred-items)
6. [References](#6-references)

---

## 1. Wire Protocol

The Splunk HEC wire protocol is HTTP/1.1. All endpoints accept plain HTTP; TLS termination is handled at the load balancer layer (see `docs/Network.md §2`). All responses use `Content-Type: application/json`.

### 1.1 Endpoints

The following endpoints are specified in Splunk documentation and confirmed by at least one implementation surveyed in §2.

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/services/collector/event` | JSON envelope ingest |
| `POST` | `/services/collector/event/1.0` | JSON envelope ingest, versioned alias |
| `POST` | `/services/collector` | JSON envelope ingest, base path alias |
| `POST` | `/services/collector/raw` | Raw line-delimited ingest |
| `POST` | `/services/collector/raw/1.0` | Raw ingest, versioned alias |
| `GET`  | `/services/collector/health` | Readiness probe |
| `GET`  | `/services/collector/health/1.0` | Readiness probe, versioned alias |
| `POST` | `/services/collector/ack` | ACK poll (deferred; see §1.7) |

The `/1.0` versioned aliases are in active use by shippers — confirmed by Fluent Bit `in_splunk` (`splunk_prot.c:842–866`) and OTel issue #2025. spank-rs currently handles the base paths only; versioned aliases are tracked as Phase 2 work. The base path `/services/collector` alias for the event endpoint is used by at least one Python HEC client (`spylunking/splunk_publisher.py`).

### 1.2 Authentication

The Authorization header carries the HEC token. Two scheme prefixes are accepted:

- `Splunk <token>` — the canonical Splunk HEC form
- `Bearer <token>` — accepted for compatibility with tooling that uses OAuth-style headers

Both prefixes are compared case-insensitively. The token value is compared as-is (case-sensitive), matching Splunk Enterprise behavior. Fluent Bit issue #9517 documented a case-sensitivity defect in an earlier Fluent Bit receiver; the real Splunk behavior is case-sensitive token values with case-insensitive prefix matching.

When the Authorization header is absent, the response is HTTP 401 with code 2 ("Token is required"). When the header is present but the token is unknown, the response is HTTP 401 with code 4 ("Invalid token"). A malformed Authorization header — present but not matching either scheme — is treated the same as absent (code 2), because the header conveys no credential.

### 1.3 Content-Encoding and Body Framing

Gzip-compressed bodies are supported. The sender signals gzip with `Content-Encoding: gzip`; the header value is compared case-insensitively. Fluent Bit `splunk_prot.c:588` and OTel `receiver.go:384` both check the same header. A malformed gzip body returns HTTP 400 with code 6.

Content-Type is used as a routing hint on the event endpoint and is not validated. A body sent to `/services/collector/event` is always parsed as JSON regardless of Content-Type. Absent Content-Type is not an error. This matches Fluent Bit's explicit note ("Not necessary to specify content-type for Splunk HEC", `splunk_prot.c:551–568`) and Vector issue #23022 (real Splunk accepts `application/json; profile=...; charset=utf-8`).

### 1.4 Event Endpoint — JSON Envelope Format

The event endpoint body is one or more JSON objects concatenated without a delimiter between them. Each object is one envelope. Whitespace between objects is ignored. The parser uses `serde_json::Deserializer::from_slice` in streaming mode, which handles concatenated objects natively.

An envelope may contain the following fields:

| Field | Type | Required | Default |
|-------|------|----------|---------|
| `event` | any JSON value | yes | — |
| `time` | number or decimal string | no | server time at receipt |
| `host` | string | no | `""` |
| `source` | string | no | `""` |
| `sourcetype` | string | no | `"_json"` |
| `index` | string | no | `"main"` |
| `fields` | flat JSON object | no | — |

The `event` field is the only required field. Its treatment on missing, null, and empty-string values is covered in §3.1.

A parse failure on any envelope rejects the entire request. Splunk does not skip-and-continue on malformed input; a multi-envelope body where the second envelope is malformed returns the same error as a single malformed envelope, and no events from the body are indexed.

### 1.5 Raw Endpoint

The raw endpoint treats the body as line-delimited text. Each non-empty line becomes one record. `\r\n` is normalized to `\n` before splitting; a trailing `\r` without a following `\n` is a sender defect and the `\r` is included in the raw text. Empty lines (blank or whitespace-only after `\r\n` normalization) are discarded.

Event metadata — `host`, `source`, `sourcetype`, `index` — is taken from query parameters for the raw endpoint, not from the body. The event body is the line text itself; there is no JSON envelope. The `sourcetype` default for raw events is `_raw`.

A body with no newlines is a valid single event. Vector issue #22969 documents a sender-side defect (missing newline delimiter) that was only caught against a real server, not a stub — confirming that raw endpoint line-splitting must be strict and tested with real payloads.

### 1.6 Health Endpoint

The health endpoint reports server readiness. spank-rs maps its internal `HecPhase` state to three response states:

| `HecPhase` | HTTP status | Response body |
|------------|-------------|---------------|
| `SERVING` | 200 | `{"text":"HEC is available","code":0}` |
| `DEGRADED` | 200 | `{"text":"HEC is degraded","code":0}` |
| `STARTED`, `STOPPING` | 503 | `{"text":"HEC is unavailable","code":9}` |

`DEGRADED` returns 200 because the node still admits work and must stay in load-balancer rotation. Splunk Enterprise responds 200 for degraded indexers while reporting reduced capacity. `STARTED` and `STOPPING` return 503 so the load balancer stops routing before the node is ready or after it begins draining.

The real Splunk health response uses `{"text":"HEC is healthy","code":17}`. spank-rs uses `code:0` for both `SERVING` and `DEGRADED`, because the zero code is the success code used in all other OK responses and the `text` field carries the state distinction. OTel issue #20871 was filed specifically because OTel returned no body on health check GET; Fluent Bit `in_splunk` returns `{"text":"Success","code":200}`, a known non-compliance. The exact Splunk text string `"HEC is healthy"` and code 17 are specific to Splunk Enterprise and do not map cleanly to the phase model — using `code:0` with descriptive text is the correct approach for a server with a multi-state phase model.

### 1.7 ACK Endpoint

The `POST /services/collector/ack` endpoint is not implemented. spank-rs returns HTTP 501 with `{"text":"ack not yet implemented","code":14}`. ACK requires three features to be present simultaneously before any part is useful: `ackId` assignment on ingest, state tracking through the indexer commit boundary, and the poll endpoint. A partial implementation produces incorrect behavior — ackIds issued but never confirmable. The full design is tracked as Plan.md HEC-ACK1.

ACK protocol overview for future reference: when a token is ACK-enabled, each successful ingest response includes an `ackId` integer (not a UUID — the channel is the UUID, the ackId is a per-channel monotonically increasing integer starting from 0). The client polls `/services/collector/ack` with a list of ackIds; the server responds with a map of `ackId -> true/false` indicating whether each event has been durably committed. Once a client retrieves `true` for an ackId, the server may delete that entry; querying again returns `false`. The `Drain::wait` primitive in `spank-core` is designed for this pattern and is ready; no caller exists yet (see `docs/Errors.md §6`).

### 1.8 Channel Header

The `X-Splunk-Request-Channel` header carries a channel identifier. Its role differs depending on whether ACK is enabled on the token:

- **ACK enabled:** the channel is required. It is the namespace for ackId sequences — ackIds are per-channel, not global. A missing channel when ACK is enabled returns code 10 ("Data channel is missing"). A channel that fails format validation returns code 11 ("Invalid data channel").
- **ACK disabled:** the channel is optional. If present, spank-rs uses it as the routing tag for the submitted batch; if absent, the token's principal name serves as the tag. The channel value is not validated for format when ACK is disabled.

This scoping of channel requirements to ACK mode is confirmed by Vector discussion #22642, Splunk documentation, and OTel `receiver.go` (validates channel only when `ackExt != nil`). Vector issue #22653 documents a bug in Vector's HEC source where it required the channel header even when ACK was disabled — spank-rs does not replicate this behavior.

An empty string channel header (header present but zero-length after trimming) is treated as absent. See §3.3 for rationale and §3.2 for the full channel handling strategy.

The channel may also be carried as a query parameter (`?channel=UUID`). spank-rs currently reads the header only; query parameter support is tracked as Plan.md HEC-CHAN1, deferred until HEC-ACK1 because the ack poll client may use the query parameter form.

### 1.9 Error Codes

All response bodies have the shape `{"text":"...", "code":N}`. The HTTP status code and the Splunk code are both meaningful; shippers branch on both.

| Code | HTTP | Text | Condition | spank-rs status |
|------|------|------|-----------|-----------------|
| 0 | 200 | Success | Accepted | Implemented |
| 1 | 403 | Token disabled | Token administratively disabled | Not implemented (no disabled-token concept yet) |
| 2 | 401 | Token is required | Authorization header absent or malformed | Implemented |
| 3 | 401 | Invalid authorization | Header present but scheme not `Splunk` or `Bearer` | Not implemented — currently collapsed into code 2 |
| 4 | 403 | Invalid token | Token value unknown | Implemented, but see note below on HTTP status |
| 5 | 400 | No data | Body empty after decoding | Implemented |
| 6 | 400 | Invalid data format | JSON unparseable, malformed gzip, or invalid time string | Implemented |
| 7 | 400 | Incorrect index | Named index does not exist in configured set | Not implemented |
| 9 | 503 | Server is busy | Queue full, or phase does not admit work | Implemented |
| 10 | 400 | Data channel is missing | ACK enabled, channel header absent | Deferred (HEC-ACK1) |
| 11 | 400 | Invalid data channel | ACK enabled, channel format invalid | Deferred (HEC-UUID1) |
| 12 | 400 | Event field is required | `event` key absent or value is null | Implemented |
| 13 | 400 | Event field cannot be blank | `event` key present with empty string | Implemented |
| 14 | 400 | Ack is disabled | ACK endpoint called; ACK not enabled on token | Currently 501; correct behavior is 400 code 14 |
| 15 | 400 | Error in handling indexed fields | `fields` value is not a flat JSON object | Not implemented |
| 16 | 400 | Query string authorization not enabled | Token supplied via query parameter, but not permitted | Not implemented |
| 17 | 200/503 | HEC is healthy / unhealthy | Health check only; code 17 in both states | spank-rs uses code 0 (see §1.6) |
| 19 | 503 | HEC is initializing | Health check during startup | spank-rs maps to code 9 via STARTED phase |
| 20 | 503 | HEC is shutting down | Health check during graceful shutdown | spank-rs maps to code 9 via STOPPING phase |
| 27 | 400 | Request entity too large | Body exceeds `max_content_length` | Currently returns code 6; should be code 27 |

**Notes on HTTP status divergences.** spank-py/HEC.md §4.6 and Splunk documentation assign HTTP 403 (Forbidden) to codes 1 and 4; spank-rs currently returns HTTP 401 for code 4. The distinction matters: 401 means "credentials needed or wrong", 403 means "credentials valid but access denied". Code 4 (unknown token) is correctly 401 in Vector's implementation (`StatusCode::UNAUTHORIZED`), which conflicts with spank-py's 403. The Splunk documentation is ambiguous; this is tracked but not yet resolved.

**Code 3 vs. code 2.** A syntactically present Authorization header that uses neither `Splunk` nor `Bearer` as the scheme should return code 3 ("Invalid authorization"), not code 2 ("Token is required"). spank-rs currently returns code 2 for both absent and malformed headers because `extract_credential` returns `None` in both cases. Code 3 requires distinguishing "header absent" from "header present but malformed".

**Code 14 response.** The ACK endpoint currently returns HTTP 501. The correct Splunk behavior is HTTP 400 with code 14 ("Ack is disabled") when the endpoint is called but ACK is not configured. Vector implements this exactly (`ApiError::AckIsDisabled → 400 code 14`). The 501 is an acceptable placeholder but differs from the Splunk wire format.

**Code 27.** Body-too-large should return code 27 with text "Request entity too large". spank-rs currently returns the generic code 6 with a message string. Correcting this requires adding a `RequestOutcome::body_too_large()` constructor.

The text strings for codes 0, 2, 5, 6, 9, 10, 12, 13, 14 are exact Splunk wire strings confirmed from Vector source (`splunk_response` module, line 1187). Must not be changed.

---

## 2. Vendor Survey

### 2.1 Sources Examined

All sources below are present in `/Users/walter/Work/Spank/sOSS/` or in `/Users/walter/Work/Spank/spank-py/`. Line numbers reference the specific files examined.

| Implementation | Source path | Language | Notes |
|----------------|-------------|----------|-------|
| OTel splunkhecreceiver | `opentelemetry-collector-contrib/receiver/splunkhecreceiver/receiver.go` | Go | Full source read |
| OTel splunk_to_logdata | `opentelemetry-collector-contrib/receiver/splunkhecreceiver/splunk_to_logdata.go` | Go | Full source read |
| Vector splunk_hec source | `vector/src/sources/splunk_hec/mod.rs` | Rust | Full source read |
| Fluent Bit in_splunk | `fluent-bit/plugins/in_splunk/splunk_prot.c` | C | Full source read |
| spylunking SplunkPublisher | `spylunking/spylunking/splunk_publisher.py` | Python | Sender; directly read |
| Splunk documentation | public | — | Error codes, endpoint paths, ACK protocol |
| Vector issues and discussions | github.com/vectordotdev/vector | — | Issues #22653, #22969, #23022; discussion #22642 |
| OTel issues | github.com/open-telemetry/opentelemetry-collector-contrib | — | Issues #2025, #19219, #20871 |
| Fluent Bit issues | github.com/fluent/fluent-bit | — | Issue #9517 |

### 2.2 Event Field — Null vs. Absent vs. Empty String

This is the most consequential divergence across implementations. The JSON type system distinguishes three states: key absent, key present with value `null`, and key present with value `""`. Splunk's error codes assign distinct meanings to two of the three states (12 = absent, 13 = blank), but the specification does not explicitly define how `null` maps.

**Splunk Enterprise (inferred from behavior):** Returns code 12 for `null`. The distinction between absent and null is erased — both represent "no event payload".

**OTel `splunk_to_logdata.go`:** In Go, JSON `null` deserializes to a nil pointer. The check `msg.Event == nil` catches both absent and null (both produce a nil pointer after deserialization); returns code 12 for both. The Go type system makes absent-vs-null indistinguishable after deserialization.

**Vector `mod.rs` — legacy namespace (`build_log_legacy`):** `None` (absent) returns code 12. `Some(Value::Null)` returns code 6. Empty string and object return code 13. The code 6 return for null is a product of how Vector's match arm is structured, not a deliberate protocol decision — null falls through to the "other invalid type" arm.

**Vector `mod.rs` — vector namespace (`build_log_vector`):** `None` (absent) returns code 12. `Some(Value::Null)` is accepted and used as the event body. This is an intentional relaxation for the vector-native path.

**Fluent Bit `in_splunk`:** No event field validation. All body content is passed through without checking for the `event` key.

**spank-rs decision:** Null maps to code 12, the same as absent. Rationale: the event field is absent semantically — `null` conveys "no value" in JSON, and the distinction between absent and null is an artifact of the JSON encoding, not a meaningful protocol state. OTel and Splunk Enterprise both agree on this. Vector legacy's code 6 return is an implementation artifact. The code 13 path is reserved for the empty string case, which is meaningfully different: the key is present and the value is explicitly the empty string — the sender chose to send an empty event.

### 2.3 Channel Header — Presence, Format, and Enforcement

**Splunk documentation:** Channel is required only when the token is ACK-enabled. Format is a UUID. Missing channel with ACK returns code 10; invalid format returns code 11.

**OTel `receiver.go:344–358`:** `extractChannel()` checks `X-Splunk-Request-Channel` header first, then the `?channel=` query parameter, then returns empty string. The result is uppercased. `validateChannelHeader()` rejects empty string (code 10) and non-UUID (code 11). Channel validation runs only when `ackExt != nil` — i.e., when the ACK extension is configured on the token.

**Vector `mod.rs` — `required_channel()`:** Reads the `X-Splunk-Request-Channel` header. No trimming, no UUID validation. An empty string header passes the filter and is accepted as a channel value. This is a latent bug: clients that send a syntactically present but empty channel header will have it silently accepted and used as a routing key.

**Vector issue #22653:** Vector's HEC source required the channel header even when ACK was disabled, rejecting valid requests. This is acknowledged as a bug. The correct behavior is to require the channel only when ACK is enabled.

**Fluent Bit `in_splunk`:** No channel header reading. No ACK support. Tag comes from the URI path segment.

**spank-rs decision:** Channel is optional when ACK is disabled. When present (non-empty after trimming), it is used as the routing tag. An empty string channel header is treated as absent — the header is meaningless if it carries no value, and accepting it as a tag would route all such requests to a single empty-string bucket. UUID format validation is deferred (Plan.md HEC-UUID1) because Splunk Enterprise does not validate format; UUID validation is only warranted if a specific deployment requires it or if ACK interop requires it.

### 2.4 Time Field — Numeric, String, and Unit Detection

**Splunk documentation:** `time` is a Unix epoch float in seconds, e.g. `1234567890.123`. The documented type is number.

**OTel `splunk_to_logdata.go`:** Accepts both `serde_json::Number` (via Go's numeric JSON type) and string representation. Parses string to float. No unit detection.

**Vector `mod.rs`:** Accepts both JSON number and string (parses to `serde_json::Number`). Unit detection by cutoff: if the value is less than `SEC_CUTOFF` (year ~2400 in seconds), it is seconds; if less than `MILLISEC_CUTOFF` (year ~10000 in milliseconds), it is milliseconds; otherwise it is nanoseconds. This allows Vector to accept integer millisecond timestamps (common from some shippers) without configuration.

**Fluent Bit `splunk_prot.c`:** Sets `time` from the record timestamp as a Unix float. Expects seconds.

**spank-rs decision:** Accepts both JSON number and decimal string (via the `TimeField` untagged enum). Always interprets the value as fractional seconds — `1234567890.5` is 1,234,567,890.5 seconds since epoch. Integer values like `1234567890` are treated as whole seconds, not milliseconds. The Vector unit-detection cutoff approach is more tolerant but introduces ambiguity: a value of `1000000000000` could be a year-2001 nanosecond timestamp or a far-future second timestamp. The seconds-only interpretation is unambiguous for the documented Splunk protocol. Clients sending integer millisecond timestamps will receive wrong event times; this is tracked as Plan.md HEC-TIME1 and re-opens if a specific client is targeted.

An invalid string (non-numeric, e.g. `"not-a-number"`) silently falls back to server time at receipt. The `TimeField::as_f64` method returns `None` for unparseable strings; the caller substitutes `now_ns()`. This matches the lenient behavior in OTel and Vector — a sender that misconfigures the time field should not have its events rejected.

### 2.5 Tag Derivation — Channel, Source, and Fallback

Tag is the routing key used to associate a batch of rows with a `Drain::wait` gate, enabling per-channel delivery confirmation. It is also used as the principal name fallback and as the log field `tag` in `ingest_event!`.

**Splunk:** Tags as a concept do not exist in the Splunk HEC wire protocol. The channel is the nearest equivalent for ACK-enabled tokens.

**OTel:** Uses channel as the ACK namespace. No equivalent of a cross-request routing tag.

**Vector:** Routes internally by source name, not by per-request channel.

**spank-rs decision:** Tag derivation priority: (1) `X-Splunk-Request-Channel` header value if present and non-empty after trimming; (2) the token principal's name (which is the token `id` from configuration). This ensures each token has a stable fallback tag even when no channel is sent. When ACK is implemented, the channel becomes the primary tag and the ackId namespace. Tag derivation from `sourcetype` or `index` is not planned (tracked as Plan.md HEC-TAG1).

### 2.6 Content-Type Handling

**Fluent Bit `splunk_prot.c:551–568`:** Explicit comment: Content-Type is not required. Falls back to content sniffing (leading `{` → JSON, else raw).

**Vector issue #23022:** Real Splunk accepts `application/json; profile=...; charset=utf-8` — substring match, not exact.

**spank-rs decision:** Content-Type is a routing hint only, not validated. The event endpoint always parses as JSON; the raw endpoint always parses as line-delimited text. Content-Type absent is not an error on either endpoint.

### 2.7 Endpoint Path Variants

**Fluent Bit `splunk_prot.c:842–866`:** Accepts `/services/collector/raw/1.0`, `/services/collector/raw`, `/services/collector/event/1.0`, `/services/collector/event`, `/services/collector` case-insensitively.

**OTel `receiver.go:159`:** Registers both `/services/collector/health` and `/services/collector/health/1.0`.

**OTel issue #2025:** Documents that some clients send to `/services/collector/event/1.0` in active deployments.

**spank-rs current state:** Handles only the base paths. The `/1.0` versioned aliases and the `/services/collector` base path alias for the event endpoint are not yet registered. These are Phase 2 scope items.

### 2.8 Indexed Fields Validation

**OTel `receiver.go:466–470`:** Validates that indexed fields (`fields` key) contain only flat JSON values — no nested objects or arrays of objects. Returns code 15 with the error text `"Error in handling indexed fields"` and an `"invalid-event-number"` field identifying which event in the batch was malformed.

**Vector:** No `fields` validation found in the source read.

**spank-rs current state:** `fields` content is accepted without validation. Nested objects are silently stored as their JSON string representation. Code 15 validation is not yet implemented.

### 2.9 Health Check Body and Status Codes

**Splunk Enterprise:** `GET /services/collector/health` → `{"text":"HEC is healthy","code":17}` when healthy. Status 200. The code 17 is specific to Splunk Enterprise's internal health model.

**OTel (before fix, issue #20871):** Returned empty body.

**Fluent Bit `in_splunk`:** Returns `{"text":"Success","code":200}` — non-compliant with Splunk wire format.

**spank-rs decision:** Uses `code:0` for available states (SERVING, DEGRADED) and `code:9` for unavailable states (STARTED, STOPPING). The Splunk Enterprise code 17 is not used because it does not map to the phase model; using code 0 (the universal success code) is more consistent with the rest of the protocol and does not require a receiver to parse a special numeric constant. The text field carries the state distinction.

### 2.10 ACK Semantics — ackId in Success Response

Vector `finish_ok` (line 1222): when ACK is enabled and the event is queued, the success response includes `"ackId": <n>` alongside `"text":"Success","code":0`. The ackId is a `u64` passed from `get_acks_status_from_channel`. When ACK is not enabled, the response is `{"text":"Success","code":0}` with no ackId field — the `Option<u64>` in `finish_ok` controls this. The `metadata` field in Vector's `HecResponse` is serialized as a flattened key: absent when `None`, present as `"ackId": <n>` when `Some`. This means the success response shape changes between ACK-enabled and ACK-disabled modes. spank-rs must produce the same shape change when HEC-ACK1 is implemented.

From OTel `receiver.go` and Splunk documentation, for future reference when HEC-ACK1 is implemented:

The ackId in the success response is a per-channel integer, not a UUID. The channel UUID is the namespace; within a channel, ackIds increment from 0. `ProcessEvent(channelID)` in OTel returns the next integer for that channel. OTel `gzipReaderPool *sync.Pool` (`receiver.go:132`) pools gzip readers to avoid per-request allocation — the same optimization applies to spank-rs if gzip throughput becomes a bottleneck, using `object_pool` or a `tokio::sync::Mutex<Vec<GzDecoder>>`. OTel registers channel IDs in a map; when the poll response returns `true` for an ackId, the entry is deleted and a subsequent query returns `false`. The `Drain::wait` semantic in `spank-core` maps to the delivery confirmation step: `wait` completes when the indexer has committed the batch, which is the moment `true` should be returned.

### 2.11 Metadata Field Carry-Forward Across Envelopes

Vector implements a `DefaultExtractor` (lines 989–1043) for each of the four metadata fields: `host`, `source`, `sourcetype`, `index`. The extractor maintains the last seen value for its field across all envelopes in a single request body. When an envelope omits a metadata field, the extractor reuses the last value seen in a previous envelope rather than falling back to an empty string or a global default.

The practical effect: in a body `{"event":"first","source":"main"}{"event":"second"}{"event":"third","source":"secondary"}`, the second event inherits `source: "main"` from the first, not the empty-string default. Vector's test at line 2207 confirms this explicitly: `events[1].source == "main"`. The third event overrides to `"secondary"`.

spank-rs current behavior: each envelope is deserialized independently. `env.source.unwrap_or_default()` uses `""` for any envelope where `source` is absent, regardless of what previous envelopes set. This diverges from Vector and from Splunk Enterprise behavior (which also carries metadata forward within a batch).

This is a correctness gap for multi-envelope bodies where only the first envelope sets metadata. A shipper that sends `{"event":"a","source":"syslog"}{"event":"b"}` expects both events to be tagged with `source: "syslog"`. spank-rs tags the second with `source: ""`. The fix requires threading state across envelope iterations in `parse_event_body`. Tracked as a new gap item; not yet in Plan.md.

### 2.12 Host Field Derivation Priority

Vector establishes a three-tier priority for the `host` field on both the event and raw endpoints (lines 714–722):

1. The `host` field in the event payload (highest priority).
2. The `X-Forwarded-For` request header value.
3. The remote socket address (peer IP of the TCP connection).

This means that behind a reverse proxy, the `host` field on events automatically reflects the original client IP from `X-Forwarded-For` when the event body does not set `host` explicitly. spank-rs sets `host` from the envelope field only, falling back to `""` (empty string). The peer address and `X-Forwarded-For` header are not consulted. This is a semantic gap for proxy deployments, not a wire protocol conformance issue.

### 2.13 No-Token Mode

Vector's authorization filter (lines 621–636): when `valid_credentials` is empty — i.e., no tokens are configured — all requests are accepted regardless of the Authorization header. A request with a well-formed `Authorization: Splunk <anything>` header passes; a request with no Authorization header also passes (the token is extracted as `None` and forwarded as-is). This is an explicit "auth disabled" mode for development use.

spank-rs requires at least one token. The `TokenStore` returns `None` on any lookup when empty, which propagates to `authenticate()` returning `Err(SpankError::Auth)`, which the handler maps to code 4. An empty token store is a configuration error, not an intentional open mode. `CFG-VAL1` (done) validates that token values are non-empty; it does not require that the token list is non-empty.

### 2.14 Splunk SDK Ingest Paths

The Splunk Python SDK (`splunklib/client.py`) does not use `/services/collector` for ingest. It uses two distinct endpoints:

- `POST /services/receivers/simple` — single-event submission. Metadata (`host`, `source`, `sourcetype`, `index`) is passed as URL query parameters; the event body is the raw POST body. Used by `Index.submit()`.
- `POST /services/receivers/stream` with `X-Splunk-Input-Mode: Streaming` header — persistent TCP connection for streaming event submission. The connection stays open; events are appended to the stream. Used by `Index.attach()` and `Index.attached_socket()`.

Neither of these paths is the HEC endpoint. They are the legacy Splunk ingest API, distinct from HEC and requiring session-key authentication (not token authentication). The SDK does not have a HEC client — HEC is for shippers (Vector, Fluent Bit, custom scripts), not for SDK-based integrations. The SDK's `Index.submit()` is suitable for spank-rs's `spank-py/API.md §5` direct-submit use case, not for the HEC receiver. spank-rs does not need to implement `/services/receivers/simple` or `/services/receivers/stream` for HEC compatibility.

The management API path for indexes is `data/indexes/` (with trailing slash, constant `PATH_INDEXES` at line 112), not `/services/data/indexes`. The SDK calls `GET data/indexes/` to list indexes, and `GET data/indexes/{name}` for a specific index. The response fields the SDK reads from the index entity include: `totalEventCount`, `maxTotalDataSizeMB`, `frozenTimePeriodInSecs`, `disabled`, `defaultDatabase`. The spank-rs `list_indexes` handler returns `totalEventCount: 0`, `currentDBSizeMB: 0`, `maxTotalDataSizeMB: 500000`, `isInternal: false`, `datatype: "event"`. The SDK's `get_default()` reads `_audit["defaultDatabase"]` — an index named `_audit` must exist for the SDK to resolve the default index name.

### 2.15 Shipper Retry Branches on HTTP Status, Not Code

spank-py/HEC.md §4.6 documents a critical behavioral fact confirmed by the spylunking source (`status_forcelist=[500,502,503,504]`): shippers branch on HTTP status code for retry decisions, not the Splunk numeric code in the response body. The implication:

- A response of HTTP 200 with `{"code":5}` (no data) is treated as a successful delivery by the shipper. The shipper does not retry.
- A response of HTTP 503 with `{"code":9}` (server busy) triggers backoff and retry.
- A response of HTTP 401 or 403 triggers re-authentication or alerting, not retry.
- A response of HTTP 400 triggers no retry — the sender treats the event as permanently rejected.

This means every rejection that should cause a shipper to retry must use HTTP 503, not HTTP 400. The only rejection that should use 503 is queue-full (code 9) and phase-not-ready (code 9). All validation errors (codes 5, 6, 7, 12, 13, 27) correctly use HTTP 400 — the shipper should not retry a malformed request. Any server-side error that is transient and retryable must use HTTP 5xx, not HTTP 4xx.

---

## 3. spank-rs Design Decisions

Each decision in this section states the options considered, the sources consulted, the tradeoffs, and the resolution. These are stable design positions — reopening one requires the conditions stated in the corresponding Plan.md item.

### 3.1 Event Field: Null Mapped to Code 12

**Options:**
- Code 12 (absent): treat null as semantically absent — no event payload, regardless of how the absence is encoded.
- Code 13 (blank): treat null as a degenerate value of the event field, same as empty string.
- Code 6 (invalid data format): treat null as a type error — the event field has a value but it is not a valid JSON type for an event.

**Evidence:** OTel maps null to code 12 via Go's nil-pointer semantics. Splunk Enterprise maps null to code 12 (inferred from behavior). Vector legacy maps null to code 6 as an implementation artifact — the null case falls through a match arm that was not designed to distinguish it from other invalid types. No implementation maps null to code 13.

**Resolution:** Code 12. Null is semantically absent. The sender expressed "no value" in the most explicit way the JSON type system allows; the appropriate response is the same as if the key were not present. Code 13 is reserved for the case where the sender explicitly chose the empty string — a distinct intent. Code 6 (Vector legacy) misclassifies a structurally valid JSON body as malformed.

**Implementation:** `HecEnvelope` uses a custom `Deserialize` impl (see §4.3) to preserve the absent/null distinction that serde's `Option<T>` erases. The match arm in `parse_event_body` handles `None | Some(Value::Null)` as a single arm returning `event_field_required()`.

### 3.2 Channel Handling Without ACK

**Options:**
- Require channel always: reject requests without a channel header regardless of ACK configuration, matching Vector's (buggy) behavior before the fix.
- Require channel only when ACK enabled: accept requests without a channel header when ACK is disabled, use the channel as a routing tag when present.
- Ignore channel always: never read the channel header.

**Evidence:** Splunk documentation scopes channel requirement to ACK-enabled tokens. OTel validates channel only when `ackExt != nil`. Vector discussion #22642 confirms the scoping. Vector issue #22653 documents that requiring channel unconditionally is a bug.

**Resolution:** Channel is optional when ACK is disabled. When present and non-empty, it is used as the routing tag (§2.5). When absent or empty, the token principal name is used as the tag. When ACK is enabled (not yet implemented), the channel is required and its absence returns code 10.

### 3.3 Empty Channel Header Treated as Absent

**Options:**
- Accept empty string as a valid channel value: consistent with treating any header value literally.
- Treat empty string as absent: the header is syntactically present but semantically empty.

**Evidence:** Vector's `required_channel()` accepts empty string without trimming — identified as a latent bug in the survey (§2.3). An empty channel would route all requests with an empty header to a single routing bucket keyed on `""`, which is not a useful distinction from "no channel".

**Resolution:** The channel value is trimmed of whitespace. An empty result after trimming is treated as absent. The header must carry a non-empty identifier to influence tag derivation.

**Implementation:** In `receiver.rs`, channel extraction is:

```rust
let tag = headers
    .get("x-splunk-request-channel")
    .and_then(|v| v.to_str().ok())
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(str::to_owned)
    .unwrap_or(principal.name);
```

The `.map(str::trim).filter(|s| !s.is_empty())` chain converts a present-but-empty header to `None` before the fallback.

### 3.4 Time Field: Fractional Seconds Only

**Options:**
- Fractional seconds only: the value `1234567890.5` is always 1,234,567,890.5 seconds since epoch.
- Unit detection by cutoff: use Vector's magnitude cutoffs to distinguish seconds, milliseconds, and nanoseconds.
- Configurable unit: let the operator specify which unit the sender uses.

**Evidence:** Splunk documentation specifies fractional seconds. Fluent Bit sends fractional seconds. The unit-detection cutoff used by Vector introduces an ambiguity window: integer values near the year-2286 boundary in seconds overlap with near-epoch millisecond values. For the documented Splunk protocol, no sender sends integer millisecond timestamps to a Splunk HEC endpoint.

**Resolution:** Fractional seconds only. The interpretation is unambiguous and matches Splunk's documented format. Clients sending integer millisecond timestamps will receive wrong event times — this is a sender defect, not a receiver defect, under the documented protocol. Tracked as Plan.md HEC-TIME1; re-opens if a specific client targeting spank-rs uses non-second timestamps.

**Implementation:** `TimeField::as_f64()` returns the numeric value directly, then the caller multiplies by `1_000_000_000.0` to convert to nanoseconds for storage.

### 3.5 Channel UUID Validation Deferred

OTel validates that the channel header value is a UUID (code 11 for non-UUID when ACK is enabled). Splunk Enterprise does not validate format — any non-empty string is accepted as a channel identifier.

spank-rs follows Splunk Enterprise: no UUID format validation. The channel is an opaque routing identifier from the server's perspective. UUID validation adds a dependency on a UUID parsing library and breaks interoperability with any shipper that uses a non-UUID channel identifier. Tracked as Plan.md HEC-UUID1.

### 3.6 Channel from Query Parameter Deferred

OTel `receiver.go:344–358` extracts the channel from the `?channel=` query parameter as a fallback when the header is absent. Splunk documentation confirms the query parameter form. spank-rs reads the header only.

The query parameter form is most relevant for ACK-enabled tokens, where the channel is required — a client that cannot set custom headers (e.g., a browser or a constrained embedded client) can pass the channel in the query string. Until HEC-ACK1 is implemented, the channel is optional and the header form is sufficient for all current deployments. Tracked as Plan.md HEC-CHAN1.

### 3.7 Token Comparison — Constant-Time Gap

`TokenStore::find` uses `HashMap::get`, which compares keys using `==` — a short-circuit comparison that returns as soon as a differing byte is found. This creates a timing side channel: a caller can measure response time to determine how many leading bytes of their guess match a registered token. At CI fixture scale this is not a meaningful attack. At production scale with many concurrent requests it is a recognized token enumeration vector.

spank-py/HEC.md §16.5.1 (TM-R3) specifies constant-time comparison via `hmac.compare_digest()`. The correct fix for spank-rs is to store token values as `[u8; 32]` HMAC-derived keys and compare with `subtle::ConstantTimeEq` from the `subtle` crate (already a transitive dependency via `rustls`). Alternatively, store tokens hashed and use `argon2::verify_raw` at fixed cost. The simpler approach — iterate all tokens and use `subtle::ConstantTimeEq::ct_eq` — avoids early exit and is sufficient for the threat model.

This is a security gap at production deployment scale. Not tracked in Plan.md yet.

### 3.8 Tag Derivation: Channel Then Token Id

The `tag` field on `QueueItem::Rows` is the per-batch routing key used inside the server. Its derivation:

1. `X-Splunk-Request-Channel` header value, if present and non-empty after trimming.
2. The authenticated principal's name, which is set to the token's `id` field from configuration.

Rationale for using token `id` rather than token `value` as the fallback: the `id` is a human-readable name (`"default"`, `"vector-prod"`) chosen by the operator, while the `value` is a credential that should not appear in logs, metrics, or routing keys. The `audit_event!` macro logs the principal name after successful authentication; that name flows through as the tag when no channel is present.

---

## 4. Code Structure

### 4.1 Module Map

The `spank-hec` crate contains five modules. Their responsibilities are bounded to a single concern each, with explicit interfaces between them.

| Module | Responsibility |
|--------|----------------|
| `authenticator` | `Authenticator` trait and `HecTokenAuthenticator` — maps `HecCredential` to `Principal` using `TokenStore`. |
| `token_store` | `TokenStore` — concurrent token registry with `upsert`, `find`, `revoke`. Backed by `parking_lot::RwLock<HashMap>`. |
| `processor` | Body decoding (`decode_body`) and event parsing (`parse_event_body`, `parse_raw_body`). Owns `HecEnvelope` deserialization, `TimeField` coercion, and event field validation. Returns `Rows` or `RequestOutcome`. |
| `outcome` | `RequestOutcome` — the inbound protocol surface. Translates ingest outcomes to `{text, code}` wire bodies. Does not interact with `SpankError`. See `docs/Errors.md §1` for the inbound/outbound split. |
| `receiver` | `HecState`, route construction, HTTP handlers, `spawn_consumer`. Owns the phase admission check, auth dispatch, body routing to `processor`, queue submission, and the `QueueItem` consumer task. |

`sender.rs` contains the `Sender` trait for the consumer-side interface; implementors live in `spank-store`.

### 4.2 Request Pipeline

Each incoming HEC request traverses these stages in order. An error at any stage returns immediately; later stages are not reached.

```
POST /services/collector/event
        |
        v
  [Phase admission]      HecPhase::admits_work() → false → 503 code 9
        |
        v
  [Size cap]             body.len() > max_content_length → 400 code 6
        |
        v
  [Auth]                 extract_credential() → None → 401 code 2
                         authenticate() → Err → 401 code 4
        |
        v
  [Decode body]          decode_body(body, content-encoding) → malformed gzip → 400 code 6
        |
        v
  [Parse]                parse_event_body(&body) → Err(RequestOutcome) → propagated
                         empty rows → 400 code 5
        |
        v
  [Tag derivation]       channel header → trim/filter → principal.name fallback
        |
        v
  [Queue submit]         queue.try_send(QueueItem::Rows) → Full → 503 code 9
        |
        v
  200 code 0 "Success"
```

The consumer task runs independently: it receives `QueueItem::Rows`, calls `sender.submit(rows)`, and emits `ingest_event!`. `QueueItem::Sentinel` triggers `sender.flush` and `drain.signal`.

### 4.3 HecEnvelope Deserialization — Absent vs. Null

The `event` field requires three-way discrimination: absent (code 12), null (code 12), empty string (code 13). serde's standard `Option<T>` deserialization cannot express this distinction because `serde_json` maps both key-absent and key-present-`null` to `None` during `deserialize_option`. The null token is consumed by the `SeqAccess`/`MapAccess` machinery before the inner deserializer is invoked, so no custom inner type can observe the difference.

The correct approach uses `serde_json::Map::deserialize` to obtain the raw map, then `map.remove("event")`:

- `remove` returns `None` when the key is absent.
- `remove` returns `Some(Value::Null)` when the key is present with a JSON `null` value.
- `remove` returns `Some(Value::String(...))` for string values.
- `remove` returns `Some(Value::Object(...))`, `Some(Value::Array(...))`, etc. for other types.

This is the only correct way to distinguish absent from null in `serde_json` without writing a fully custom visitor. The same approach applies to `time`, `host`, `source`, `sourcetype`, `index`, and `fields` — each is extracted via `map.remove`, which is also more efficient than `serde_json::Value` roundtripping through the derive path.

The custom `Deserialize` impl for `HecEnvelope` is in `processor.rs`. The module-level doc comment explains the null/absent distinction and the error code assignment.

### 4.4 TimeField — Number and String Coercion

```rust
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TimeField {
    Number(f64),
    Text(String),
}
```

The untagged representation tries `Number` first (serde_json deserializes any JSON number to `f64`), then `Text` (any JSON string). The `as_f64` method on `Text` uses `str::trim().parse::<f64>()`, which handles leading and trailing whitespace and returns `None` for non-numeric strings. `None` at the call site falls back to `now_ns()`.

The `time` field extraction in `HecEnvelope::deserialize` uses:

```rust
let time = map
    .remove("time")
    .and_then(|v| serde_json::from_value::<TimeField>(v).ok());
```

A `time` value that is neither a JSON number nor a parseable decimal string (e.g., a JSON array) silently falls back to server time. This matches the lenient fallback used for invalid string values.

### 4.5 Channel Extraction in receiver

Channel extraction in `handle()` applies trim and empty-filter before the fallback:

```rust
let tag = headers
    .get("x-splunk-request-channel")
    .and_then(|v| v.to_str().ok())
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(str::to_owned)
    .unwrap_or(principal.name);
```

The `str::trim` step handles headers that consist of only whitespace. The `filter` step converts an empty-after-trim result to `None`, which causes `unwrap_or` to use `principal.name`. The `map(str::to_owned)` step before `unwrap_or` is necessary because `trim` returns `&str` borrowed from the header value, while `principal.name` is a `String` — the ownership shapes must match at `unwrap_or`.

### 4.6 Queue and Consumer

`HecState` holds a `mpsc::Sender<QueueItem>`. The queue capacity is `cfg.hec.queue_depth`. Submit uses `try_send` — never `send().await` — so backpressure is detected immediately and returned as HTTP 503 rather than blocking the handler task. See `docs/Errors.md §3` for the full backpressure model.

The consumer task (`spawn_consumer`) loops on `tokio::select!` with two arms: the lifecycle cancellation token (clean stop) and the queue receiver. `QueueItem::Rows` dispatches to `sender.submit`; `QueueItem::Sentinel` dispatches to `sender.flush` and then `drain.signal`. The `QUEUE_DEPTH_CURRENT` gauge is updated after each receive from `rx.len()`.

---

## 5. Deferred Items

The following Plan.md items directly affect the protocol behavior documented here. Each entry states what will change when the item is resolved.

| Plan.md ID | Effect on this document |
|------------|------------------------|
| HEC-ACK1 | §1.7 ACK endpoint becomes implemented. §1.8 channel enforcement for ACK-enabled tokens becomes active. §3.2 channel handling updated to cover ACK mode. §4.2 request pipeline extended with ackId generation. |
| HEC-UUID1 | §1.8 and §3.5 updated if UUID validation is added: code 11 becomes enforced for non-UUID channel values when ACK is enabled. |
| HEC-TIME1 | §1.4 `time` field description and §3.4 updated if unit detection by cutoff is added. |
| HEC-CHAN1 | §1.8 updated to document query parameter extraction alongside header. §4.5 updated with extraction logic. |
| HEC-TAG1 | §3.7 updated if `sourcetype` or `index` participates in tag derivation. |

---

## 6. References

[1] Splunk, *HTTP Event Collector error codes*, Splunk documentation — codes 0, 2, 4, 5, 6, 9, 10, 11, 12, 13, 14, 15.

[2] Splunk, *HTTP Event Collector walkthroughs*, Splunk documentation — ACK protocol, ackId semantics, channel header requirements.

[3] OTel `receiver.go`, `opentelemetry-collector-contrib/receiver/splunkhecreceiver/` — channel extraction (lines 344–358), ACK gating (ackExt != nil), gzip pool (line 132), field validation (lines 466–470).

[4] OTel `splunk_to_logdata.go`, `opentelemetry-collector-contrib/receiver/splunkhecreceiver/` — event field nil check, raw endpoint metadata from query parameters.

[5] Vector `mod.rs`, `vector/src/sources/splunk_hec/` — `required_channel()` (no trim, empty string passes), `build_log_legacy` (null → code 6), `build_log_vector` (null accepted), time unit cutoff detection.

[6] Fluent Bit `splunk_prot.c`, `fluent-bit/plugins/in_splunk/` — endpoint path variants (lines 842–866), Content-Type comment (lines 551–568), gzip via Content-Encoding (line 588), tag from URI path.

[7] Vector issue #22653 — channel required unconditionally (bug); correct behavior: channel required only when ACK enabled.

[8] Vector issue #22969 — raw endpoint missing newline delimiter (sender-side bug); confirms receiver must be tested with real payloads.

[9] Vector issue #23022 — Content-Type substring matching in real Splunk.

[10] Vector discussion #22642 — channel header scoped to ACK mode confirmation.

[11] OTel issue #2025 — `/services/collector/event/1.0` path in active use.

[12] OTel issue #19219 — alignment of auth error response body with Splunk Enterprise format.

[13] OTel issue #20871 — empty health check body (now fixed).

[14] Fluent Bit issue #9517 — token case sensitivity; real Splunk: case-insensitive prefix, case-sensitive token value.

[15] `serde_json` crate, *Deserialize implementation notes* — `deserialize_option` shortcut: JSON null token is consumed before inner deserializer is invoked; `Map::remove` returns `None` for absent key and `Some(Value::Null)` for present-null.

[16] spylunking `splunk_publisher.py` — concatenated JSON body format confirmed (`self.log_payload = self.log_payload + msg`, line 476); flush threshold 524,288 bytes (512 KiB, line 510); auth header exactly `Authorization: Splunk <token>` (line 668); POSTs to `/services/collector` base path alias (line 641); retry on HTTP 500, 502, 503, 504 via `requests.adapters.HTTPAdapter` with `Retry(status_forcelist=[500,502,503,504])` (line 299).

[17] Vector `splunk_hec/mod.rs` — `DefaultExtractor` carry-forward (lines 989–1043): last-seen metadata value propagated to subsequent envelopes in same request. Test confirmation at line 2207: second event inherits source from first. `X-Forwarded-For` as host fallback (lines 714–722). No-token mode when `valid_credentials` is empty (lines 621–636). `warp::path::end()` matches `/services/collector` base path (line 357). `finish_ok(maybe_ack_id)` includes `"ackId"` in success response only when ACK enabled (lines 1222–1228). Time integer path: `t.as_u64()` → `parse_timestamp`, failure or negative → `InvalidDataFormat` (lines 800–818). `parse_timestamp` cutoffs: SEC_CUTOFF = 13569465600 (year 2400), MILLISEC_CUTOFF = 253402300800000 (year 10000) (lines 966–986). `HecStatusCode` enum with exact numeric values (lines 1155–1166). Wire text strings confirmed (lines 1186–1196).

[18] `splunk-sdk-python/splunklib/client.py` — `PATH_RECEIVERS_SIMPLE = "/services/receivers/simple"` and `PATH_RECEIVERS_STREAM = "/services/receivers/stream"` (lines 124–125) are legacy ingest paths distinct from HEC; `X-Splunk-Input-Mode: Streaming` header for stream mode (line 2221); `PATH_INDEXES = "data/indexes/"` (line 112); `get_default()` reads `_audit["defaultDatabase"]` (lines 2143–2150); `Index.submit()` passes metadata as query parameters (lines 2330–2338).

[19] spank-py/HEC.md §4.6 — complete Splunk error code table including codes 1, 3, 7, 16, 17, 19, 20, 27 with HTTP status assignments. §4.6 note: shippers branch on HTTP status for retry, not the numeric code; HTTP 200 with non-zero code treated as successful delivery.

[20] spank-py/HEC.md §16.5.1, §17.3 — constant-time token comparison requirement (TM-R3); `hmac.compare_digest()` as the implementation; token values never stored in logs or exposed through query interfaces (TM-R5).
