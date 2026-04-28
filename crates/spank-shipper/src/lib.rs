//! `spank-shipper` — egress senders.
//!
//! Concrete `Sender` implementations for shipping records out:
//! `TcpSender` for line-delimited JSON over TCP. The `Forwarder`
//! (HEC client) is a future addition.
//!
//! `TcpSender` reconnects with exponential backoff on disconnect.
//! Lines are queued in a bounded buffer; on overflow the policy is
//! drop-oldest (logged as backpressure).

pub mod tcp;

pub use tcp::TcpSender;
