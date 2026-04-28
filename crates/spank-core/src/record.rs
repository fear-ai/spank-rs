//! In-memory event types.
//!
//! [`Record`] is the canonical event. [`Row`] is a synonym used at
//! search-execution sites to read like SQL/dataframe vocabulary; they
//! are the same object. [`Rows`] is `Vec<Row>` and is what crosses
//! channel boundaries inside the ingest Pipeline.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Splunk-aligned event envelope, carrying both wire and indexed fields.
///
/// Fields prefixed with `_` are reserved per Splunk convention. All times
/// are UTC nanoseconds since the Unix epoch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    /// Event timestamp from the source.
    pub time_event_ns: i64,
    /// Time the event was committed to a bucket.
    pub time_index_ns: i64,
    /// Raw bytes as received, unparsed.
    pub raw: String,
    /// Source identifier — file path, channel id, peer.
    pub source: String,
    /// Sourcetype — parser selector.
    pub sourcetype: String,
    /// Host of origin.
    pub host: String,
    /// Index name.
    pub index: String,
    /// Indexed fields.
    pub fields: BTreeMap<String, String>,
}

impl Record {
    #[must_use]
    pub fn builder(raw: impl Into<String>) -> RecordBuilder {
        RecordBuilder::new(raw)
    }
}

/// Synonym used during search execution. Same object as [`Record`].
pub type Row = Record;

/// Many [`Row`]s. Used as the unit on bounded channels and at backend
/// write boundaries.
pub type Rows = Vec<Row>;

/// Convenience builder for [`Record`].
#[derive(Debug)]
pub struct RecordBuilder {
    inner: Record,
}

impl RecordBuilder {
    fn new(raw: impl Into<String>) -> Self {
        Self {
            inner: Record {
                time_event_ns: 0,
                time_index_ns: 0,
                raw: raw.into(),
                source: String::new(),
                sourcetype: String::new(),
                host: String::new(),
                index: "main".to_string(),
                fields: BTreeMap::new(),
            },
        }
    }

    pub fn time_event_ns(mut self, ns: i64) -> Self {
        self.inner.time_event_ns = ns;
        self
    }

    pub fn time_index_ns(mut self, ns: i64) -> Self {
        self.inner.time_index_ns = ns;
        self
    }

    pub fn source(mut self, s: impl Into<String>) -> Self {
        self.inner.source = s.into();
        self
    }

    pub fn sourcetype(mut self, s: impl Into<String>) -> Self {
        self.inner.sourcetype = s.into();
        self
    }

    pub fn host(mut self, s: impl Into<String>) -> Self {
        self.inner.host = s.into();
        self
    }

    pub fn index(mut self, s: impl Into<String>) -> Self {
        self.inner.index = s.into();
        self
    }

    pub fn field(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.inner.fields.insert(k.into(), v.into());
        self
    }

    #[must_use]
    pub fn build(self) -> Record {
        self.inner
    }
}
