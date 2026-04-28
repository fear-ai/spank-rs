//! `spank-tcp` — TCP ingress.
//!
//! Listener accepts connections; each connection runs in its own
//! tokio task. The task reads newline-delimited frames with a hard
//! length cap. Every syscall path returns `SpankError::Io` with the
//! syscall name and the peer attribution.
//!
//! Output: a per-connection record stream that the binary wires to
//! a `Sender` (file or shipper). Tracing spans per connection;
//! metrics for bytes in, lines, and syscall errors by name.

pub mod listener;
pub mod receiver;

pub use listener::serve;
pub use receiver::{ConnEvent, ConnHandle};
