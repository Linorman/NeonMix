//! Explicit platform acceptance; ordinary tests never access ambient credentials.
use neonmix_identity::vault;

struct TemporaryCredential(String);
impl Drop for TemporaryCredential {
    fn drop(&mut self) {
        let _ = vault::remove(&self.0);
    }
}

#[test]
#[ignore = "requires explicit access to the native platform credential store"]
fn native_credential_round_trip_and_deletion() {
    let value = format!("neonmix-native-probe-{}", uuid::Uuid::new_v4());
    let credential = TemporaryCredential(vault::put(&value).unwrap());
    assert_eq!(vault::get(&credential.0).unwrap(), value);
    vault::remove(&credential.0).unwrap();
    assert!(vault::get(&credential.0).is_err());
    vault::remove(&credential.0).unwrap();
}
