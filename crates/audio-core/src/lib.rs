//! Platform-independent audio contracts. No UI, OS handles, network or runtime.
pub mod block;
pub mod capture;
pub mod clock;
pub mod device;
pub mod epoch;
pub mod position;
pub mod queue;
pub mod resample;
pub mod signal;
pub mod stats;
pub use block::*;
pub use device::*;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("device unavailable: {0}")]
    DeviceUnavailable(String),
    #[error("audio capture permission denied: {0}")]
    PermissionDenied(String),
    #[error("native audio error: {0}")]
    Backend(String),
}
