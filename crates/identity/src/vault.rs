//! Native credential stores only. Failure never selects a plaintext fallback.
use crate::Result;
const SERVICE: &str = "com.neonmix.identity.v1";
fn entry(reference: &str) -> Result<keyring::Entry> {
    if uuid::Uuid::parse_str(reference).is_err() {
        return Err("invalid credential reference".into());
    }
    Ok(keyring::Entry::new(SERVICE, reference)?)
}
pub fn put(value: &str) -> Result<String> {
    let reference = uuid::Uuid::new_v4().to_string();
    let entry = entry(&reference)?;
    if let Err(error) = entry.set_password(value) {
        let _ = entry.delete_credential();
        return Err(error.into());
    }
    match entry.get_password() {
        Ok(readback) if readback == value => Ok(reference),
        _ => {
            let _ = entry.delete_credential();
            Err("platform credential readback failed".into())
        }
    }
}
pub fn get(reference: &str) -> Result<String> {
    Ok(entry(reference)?.get_password()?)
}
pub fn remove(reference: &str) -> Result<()> {
    match entry(reference)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.into()),
    }
}
pub fn probe() -> Result<()> {
    let reference = put(&crate::secret())?;
    remove(&reference)?;
    match entry(&reference)?.get_password() {
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => Err("platform credential deletion failed".into()),
    }
}
