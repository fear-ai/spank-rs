//! `TokenStore` — HEC token holder + lifecycle.
//!
//! Replaces the ad-hoc dict from the Python implementation. The store
//! is the single source of truth for which tokens are valid; rotation
//! is a method, not a mutation.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use spank_cfg::HecToken;

#[derive(Clone, Debug)]
pub struct TokenRecord {
    pub id: String,
    pub allowed_indexes: Vec<String>,
}

#[derive(Default)]
struct Inner {
    /// `value -> record`
    by_value: HashMap<String, TokenRecord>,
}

#[derive(Clone, Default)]
pub struct TokenStore {
    inner: Arc<RwLock<Inner>>,
}

impl TokenStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_config(tokens: &[HecToken]) -> Self {
        let store = Self::default();
        for t in tokens {
            store.upsert(t);
        }
        store
    }

    pub fn upsert(&self, token: &HecToken) {
        let mut inner = self.inner.write();
        inner.by_value.insert(
            token.value.clone(),
            TokenRecord {
                id: token.id.clone(),
                allowed_indexes: token.allowed_indexes.clone(),
            },
        );
    }

    /// Look up by token value (the credential the client sends).
    #[must_use]
    pub fn find(&self, value: &str) -> Option<TokenRecord> {
        self.inner.read().by_value.get(value).cloned()
    }

    /// Remove a token by value. Returns whether it existed.
    pub fn revoke(&self, value: &str) -> bool {
        self.inner.write().by_value.remove(value).is_some()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.read().by_value.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_find_revoke() {
        let s = TokenStore::new();
        s.upsert(&HecToken {
            id: "id1".into(),
            value: "secret".into(),
            allowed_indexes: vec!["main".into()],
        });
        assert_eq!(s.find("secret").unwrap().id, "id1");
        assert!(s.revoke("secret"));
        assert!(s.find("secret").is_none());
    }
}
