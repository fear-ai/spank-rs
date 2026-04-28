//! Authenticated identity.
//!
//! A `Principal` is the result of an `Authenticator` accepting a
//! credential. It is request-scoped, never persisted, and never logged
//! with credential material.
//!
//! See `docs/Sparst.md` §8.5 for the authoritative definition.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    /// Identity string — username, token id, service account.
    pub name: String,
    /// Roles used by authorization decisions. Frozen-by-convention;
    /// we do not mutate principals after construction.
    pub roles: Vec<String>,
    /// Backend-specific attributes (token kind, source ip on creation,
    /// tenant). Read-only; not part of identity equality.
    pub metadata: BTreeMap<String, String>,
}

impl Principal {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            roles: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.roles.push(role.into());
        self
    }

    #[must_use]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }
}

impl PartialEq for Principal {
    /// Identity equality is by `name` only. Roles and metadata are
    /// authorization-time concerns and are not part of identity.
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Principal {}
