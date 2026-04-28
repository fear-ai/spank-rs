//! HEC body processor.
//!
//! Decodes gzip if needed, parses event-format JSON, validates fields,
//! and produces the `Rows` for indexing along with the `RequestOutcome`
//! the receiver should send on the wire.

use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde::Deserialize;
use spank_core::{Record, Rows};

use crate::outcome::RequestOutcome;

#[derive(Debug, Deserialize)]
struct HecEnvelope {
    #[serde(default)]
    event: serde_json::Value,
    #[serde(default)]
    time: Option<f64>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    sourcetype: Option<String>,
    #[serde(default)]
    index: Option<String>,
    #[serde(default)]
    fields: Option<serde_json::Map<String, serde_json::Value>>,
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
/// # Errors
/// Returns `RequestOutcome::invalid_data` on parse failure.
pub fn parse_event_body(body: &[u8]) -> Result<Rows, RequestOutcome> {
    let de = serde_json::Deserializer::from_slice(body);
    let mut rows = Rows::new();
    let now_ns = now_ns();
    for v in de.into_iter::<HecEnvelope>() {
        let env = v.map_err(|e| RequestOutcome::invalid_data(format!("invalid event JSON: {e}")))?;
        let raw = match &env.event {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let t_event_ns = env
            .time
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
