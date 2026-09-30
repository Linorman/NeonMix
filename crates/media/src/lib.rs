//! Authenticated native media adapter. GStreamer owns codec, the one jitter
//! buffer, DTLS-SRTP and replay protection. Rust owns authorization and quotas.
mod scheduling;
mod thread_priority;
pub use scheduling::SchedulingSnapshot;
pub use thread_priority::{ThreadPriority, ThreadPrioritySnapshot};
pub mod protocol;
pub mod transport;
pub use transport::*;

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("native_media: {0}")]
    Native(String),
    #[error("invalid_media_packet")]
    InvalidPacket,
    #[error("certificate_mismatch")]
    CertificateMismatch,
    #[error("media_context_inactive")]
    Inactive,
    #[error("media_queue_full")]
    QueueFull,
    #[error("media_handshake_timeout")]
    HandshakeTimeout,
}
