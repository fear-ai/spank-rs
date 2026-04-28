//! API server bootstrap.
//!
//! Owns the listener, handles graceful shutdown, and emits structured
//! lifecycle events.

use std::net::SocketAddr;

use axum::Router;
use spank_core::error::{Result, SpankError};
use spank_core::lifecycle::Lifecycle;
use spank_obs::lifecycle_event;
use tokio::net::TcpListener;

/// Serve `router` on `addr` until `lifecycle` cancels.
///
/// # Errors
/// Bind failures and accept errors are returned as `SpankError::Io`.
pub async fn serve(router: Router, addr: SocketAddr, lifecycle: Lifecycle) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| SpankError::io("bind", addr.to_string(), e))?;

    let local = listener
        .local_addr()
        .map_err(|e| SpankError::io("local_addr", addr.to_string(), e))?;

    lifecycle_event!(
        component = "api",
        kind = "ready",
        bind = %local,
        "api listening"
    );

    let shutdown = {
        let token = lifecycle.token.clone();
        async move {
            token.cancelled().await;
        }
    };

    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| SpankError::io("axum_serve", addr.to_string(), e.into()))?;

    lifecycle_event!(component = "api", kind = "stopped", "api stopped");
    Ok(())
}
