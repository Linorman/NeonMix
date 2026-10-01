//! Discovery is untrusted; an out-of-band invitation establishes pinned identity.
pub mod discovery;
pub mod files;
pub mod pairing;
pub mod trust;
pub mod vault;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub fn secret() -> String {
    // Two independently generated OS-random UUIDs provide 244 random bits.
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
pub fn digest(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
