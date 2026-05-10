//! HEC body processor.
//!
//! Decodes gzip if needed, parses event-format JSON, validates fields,
//! and produces the `Rows` for indexing along with the `RequestOutcome`
//! the receiver should send on the wire.
//!
//! ## Event field validation
//!
//! Splunk's documented codes for the event endpoint:
//! - Code 5: no data (empty body after decode)
//! - Code 6: invalid data format (unparseable JSON)
//! - Code 12: `event` field absent
//! - Code 13: `event` field is null or empty string
//!
//! Multi-envelope bodies: a parse failure on any envelope rejects the
//! entire request — Splunk does not skip-and-continue on malformed input.
//!
//! ## `time` field coercion
//!
//! Splunk accepts `time` as either a JSON number (`1234567890.123`) or a
//! decimal string (`"1234567890.123"`). The deserializer handles both via
//! the `TimeField` newtype.

use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde::Deserialize;
use spank_core::{Record, Rows};

use crate::outcome::RequestOutcome;

/// `time` field — Splunk accepts a JSON number or a decimal string.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TimeField {
    Number(f64),
    Text(String),
}

impl TimeField {
    fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            Self::Text(s) => s.trim().parse::<f64>().ok(),
        }
    }
}

/// Raw envelope deserialized as a map so that absent vs. null on the
/// `event` field can be distinguished. serde's `Option<T>` maps both
/// key-absent and `null` to `None`; extracting from `Value::Object`
/// preserves the difference.
struct HecEnvelope {
    /// `None` = key absent (code 12), `Some(Value::Null)` = null (code 13).
    event: Option<serde_json::Value>,
    time: Option<TimeField>,
    host: Option<String>,
    source: Option<String>,
    sourcetype: Option<String>,
    index: Option<String>,
    fields: Option<serde_json::Map<String, serde_json::Value>>,
}

impl<'de> Deserialize<'de> for HecEnvelope {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut map = serde_json::Map::deserialize(d)?;

        // `remove` returns None when key is absent; Some(Value::Null) when present-null.
        let event = map.remove("event");

        let time = map
            .remove("time")
            .and_then(|v| serde_json::from_value::<TimeField>(v).ok());

        let host = map
            .remove("host")
            .and_then(|v| v.as_str().map(str::to_owned));
        let source = map
            .remove("source")
            .and_then(|v| v.as_str().map(str::to_owned));
        let sourcetype = map
            .remove("sourcetype")
            .and_then(|v| v.as_str().map(str::to_owned));
        let index = map
            .remove("index")
            .and_then(|v| v.as_str().map(str::to_owned));
        let fields = map.remove("fields").and_then(|v| {
            if let serde_json::Value::Object(m) = v {
                Some(m)
            } else {
                None
            }
        });

        Ok(Self { event, time, host, source, sourcetype, index, fields })
    }
}

/// Processor outcome — what to enqueue and what to send on the wire.
pub struct Processed {
    pub rows: Rows,
    pub outcome: RequestOutcome,
}

/// Decompress if Content-Encoding is gzip.
///
/// # Errors
/// Returns `RequestOutcome::invalid_data` on malformed gzip.
pub fn decode_body(body: Bytes, content_encoding: Option<&str>) -> Result<Bytes, RequestOutcome> {
    if content_encoding.map(|s| s.eq_ignore_ascii_case("gzip")) == Some(true) {
        let mut dec = flate2::read::GzDecoder::new(body.as_ref());
        let mut out = Vec::new();
        dec.read_to_end(&mut out)
            .map_err(|_| RequestOutcome::invalid_data("malformed gzip"))?;
        Ok(Bytes::from(out))
    } else {
        Ok(body)
    }
}

/// Parse an event-endpoint body. The HEC event format is one or more
/// JSON envelopes concatenated (whitespace allowed between).
///
/// Returns an error outcome on the first malformed envelope — Splunk
/// does not skip-and-continue. The entire request is rejected.
///
/// # Errors
/// - `invalid_data` (code 6): JSON is not parseable
/// - `event_field_required` (code 12): `event` key absent, or present with `null`
/// - `event_field_blank` (code 13): `event` is present with an empty string
pub fn parse_event_body(body: &[u8]) -> Result<Rows, RequestOutcome> {
    let de = serde_json::Deserializer::from_slice(body);
    let mut rows = Rows::new();
    let now_ns = now_ns();
    for v in de.into_iter::<HecEnvelope>() {
        let env =
            v.map_err(|e| RequestOutcome::invalid_data(format!("invalid event JSON: {e}")))?;

        // Validate the event field.
        // Code 12 ("Event field is required"): key absent OR key present with null.
        // Code 13 ("Event field cannot be blank"): key present with empty string.
        // Null is treated as absent (code 12), not blank (code 13). This matches
        // Splunk Enterprise and the OpenTelemetry receiver, both of which map null
        // to the same outcome as a missing key. Vector's legacy namespace disagrees
        // (returns code 6 for null); we follow Splunk Enterprise here.
        let raw = match env.event {
            None | Some(serde_json::Value::Null) => {
                return Err(RequestOutcome::event_field_required())
            }
            Some(serde_json::Value::String(ref s)) if s.is_empty() => {
                return Err(RequestOutcome::event_field_blank())
            }
            Some(serde_json::Value::String(s)) => s,
            Some(other) => other.to_string(),
        };

        let t_event_ns = env
            .time
            .as_ref()
            .and_then(TimeField::as_f64)
            .map(|t| (t * 1_000_000_000.0) as i64)
            .unwrap_or(now_ns);

        let mut rec = Record::builder(raw)
            .time_event_ns(t_event_ns)
            .time_index_ns(now_ns)
            .source(env.source.unwrap_or_default())
            .sourcetype(env.sourcetype.unwrap_or_else(|| "_json".into()))
            .host(env.host.unwrap_or_default())
            .index(env.index.unwrap_or_else(|| "main".into()))
            .build();
        if let Some(fields) = env.fields {
            for (k, v) in fields {
                let val = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                rec.fields.insert(k, val);
            }
        }
        rows.push(rec);
    }
    Ok(rows)
}

/// Parse a raw-endpoint body. Each non-empty line becomes one record.
pub fn parse_raw_body(body: &[u8], default_source: &str) -> Rows {
    let now_ns = now_ns();
    body.split(|b| *b == b'\n')
        .filter(|s| !s.is_empty())
        .map(|line| {
            Record::builder(String::from_utf8_lossy(line).into_owned())
                .time_event_ns(now_ns)
                .time_index_ns(now_ns)
                .source(default_source)
                .sourcetype("_raw".to_string())
                .index("main".to_string())
                .build()
        })
        .collect()
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_absent_gives_code_12() {
        let body = br#"{"source":"test"}"#;
        let err = parse_event_body(body).unwrap_err();
        assert_eq!(err.code, 12, "missing event field should be code 12");
        assert_eq!(err.http_status, 400);
    }

    #[test]
    fn event_null_gives_code_12() {
        // null is treated as absent (code 12), not blank (code 13).
        // Matches Splunk Enterprise and OpenTelemetry receiver behavior.
        let body = br#"{"event":null}"#;
        let err = parse_event_body(body).unwrap_err();
        assert_eq!(err.code, 12, "null event field should be code 12 (same as absent)");
    }

    #[test]
    fn event_empty_string_gives_code_13() {
        let body = br#"{"event":""}"#;
        let err = parse_event_body(body).unwrap_err();
        assert_eq!(err.code, 13, "empty string event field should be code 13");
    }

    #[test]
    fn malformed_json_gives_code_6() {
        let body = b"not json";
        let err = parse_event_body(body).unwrap_err();
        assert_eq!(err.code, 6);
    }

    #[test]
    fn valid_event_string() {
        let body = br#"{"event":"hello world","source":"test"}"#;
        let rows = parse_event_body(body).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].raw, "hello world");
        assert_eq!(rows[0].source, "test");
    }

    #[test]
    fn valid_event_object() {
        let body = br#"{"event":{"key":"value"}}"#;
        let rows = parse_event_body(body).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].raw.contains("key"));
    }

    #[test]
    fn multi_envelope_all_valid() {
        let body = br#"{"event":"first"}{"event":"second"}"#;
        let rows = parse_event_body(body).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn multi_envelope_second_malformed_rejects_all() {
        let body = br#"{"event":"first"}{"source":"missing_event"}"#;
        let err = parse_event_body(body).unwrap_err();
        assert_eq!(err.code, 12, "second envelope missing event should reject all");
    }

    #[test]
    fn time_as_number() {
        let body = br#"{"event":"e","time":1234567890.5}"#;
        let rows = parse_event_body(body).unwrap();
        assert_eq!(rows[0].time_event_ns, (1234567890.5f64 * 1e9) as i64);
    }

    #[test]
    fn time_as_string() {
        let body = br#"{"event":"e","time":"1234567890.5"}"#;
        let rows = parse_event_body(body).unwrap();
        assert_eq!(rows[0].time_event_ns, (1234567890.5f64 * 1e9) as i64);
    }

    #[test]
    fn time_as_invalid_string_falls_back_to_now() {
        let body = br#"{"event":"e","time":"not-a-number"}"#;
        // Should not error; should fall back to now_ns.
        let rows = parse_event_body(body).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].time_event_ns > 0);
    }

    #[test]
    fn no_data_empty_body() {
        let rows = parse_event_body(b"").unwrap();
        assert!(rows.is_empty(), "empty body yields empty rows (caller checks)");
    }
}
