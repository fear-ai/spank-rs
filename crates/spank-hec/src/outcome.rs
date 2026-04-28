//! `RequestOutcome` — what a HEC `Processor` produces from one request.
//!
//! Splunk wire shape: `{ "text": ..., "code": ... }`. The HTTP status
//! is carried alongside.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RequestOutcome {
    #[serde(skip)]
    pub http_status: u16,
    pub text: String,
    pub code: u32,
}

impl RequestOutcome {
    #[must_use]
    pub fn ok() -> Self {
        Self {
            http_status: 200,
            text: "Success".into(),
            code: 0,
        }
    }

    #[must_use]
    pub fn server_busy() -> Self {
        Self {
            http_status: 503,
            text: "Server is busy".into(),
            code: 9,
        }
    }

    #[must_use]
    pub fn invalid_token() -> Self {
        Self {
            http_status: 401,
            text: "Invalid token".into(),
            code: 4,
        }
    }

    #[must_use]
    pub fn no_authorization() -> Self {
        Self {
            http_status: 401,
            text: "Token is required".into(),
            code: 2,
        }
    }

    #[must_use]
    pub fn invalid_data(message: impl Into<String>) -> Self {
        Self {
            http_status: 400,
            text: message.into(),
            code: 6,
        }
    }
}
