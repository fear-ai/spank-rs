//! `spank-core` — the foundation crate.
//!
//! Holds types and primitives that every other crate depends on:
//!
//! - [`error`] — the unified [`SpankError`] taxonomy and [`Result`] alias.
//! - [`record`] — the in-memory event ([`Record`]) and the search-time alias [`Row`] / [`Rows`].
//! - [`sentinel`] — the [`Sentinel`] end/checkpoint marker.
//! - [`drain`] — the [`Drain`] wait-side handle paired with `Sentinel`.
//! - [`phase`] — the [`HecPhase`] lifecycle enum.
//! - [`lifecycle`] — `CancellationToken` hierarchy helpers.
//!
//! Nothing here imports from any other Spank crate.

pub mod drain;
pub mod error;
pub mod lifecycle;
pub mod phase;
pub mod principal;
pub mod record;
pub mod sentinel;

pub use drain::Drain;
pub use error::{Result, SpankError};
pub use phase::HecPhase;
pub use principal::Principal;
pub use record::{Record, Row, Rows};
pub use sentinel::{Sentinel, SentinelKind};
