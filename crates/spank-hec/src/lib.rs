//! `spank-hec` — Splunk HEC receiver.
//!
//! Implements `/services/collector/event` and `/services/collector/raw`
//! per Splunk HEC. Authenticates via `TokenStore`, decodes gzip,
//! parses events, produces a `RequestOutcome`, and writes records to
//! per-channel rotating files via the `Sender` trait. Flushing on
//! channel close uses `Sentinel` + `Drain`.

pub mod authenticator;
pub mod outcome;
pub mod processor;
pub mod receiver;
pub mod sender;
pub mod token_store;

pub use authenticator::{Authenticator, HecTokenAuthenticator};
pub use outcome::RequestOutcome;
pub use receiver::{routes, HecState};
pub use sender::{FileSender, Sender};
pub use token_store::TokenStore;
