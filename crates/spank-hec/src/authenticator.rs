//! Authenticator ABC and HEC-token implementation.
//!
//! See `docs/Sparst.md` §7.1, §8.5.

use spank_core::{Principal, Result, SpankError};

use crate::token_store::TokenStore;

/// Credential carried in an HEC `Authorization` header.
#[derive(Debug, Clone)]
pub struct HecCredential {
    pub token_value: String,
}

/// `Authenticator` trait. Concrete impls map a credential to a
/// `Principal`. Authorization is a separate decision.
pub trait Authenticator: Send + Sync + 'static {
    fn authenticate(&self, credential: &HecCredential) -> Result<Principal>;
}

/// HEC token authenticator backed by a `TokenStore`.
pub struct HecTokenAuthenticator {
    store: TokenStore,
}

impl HecTokenAuthenticator {
    #[must_use]
    pub fn new(store: TokenStore) -> Self {
        Self { store }
    }
}

impl Authenticator for HecTokenAuthenticator {
    fn authenticate(&self, credential: &HecCredential) -> Result<Principal> {
        match self.store.find(&credential.token_value) {
            Some(rec) => {
                let mut p = Principal::new(rec.id.clone()).with_role("hec_writer");
                for idx in rec.allowed_indexes {
                    p.metadata.insert(format!("allowed_index:{idx}"), "yes".into());
                }
                p.metadata.insert("token_id".into(), rec.id);
                Ok(p)
            }
            None => Err(SpankError::Auth {
                message: "invalid token".into(),
            }),
        }
    }
}
