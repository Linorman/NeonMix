//! The same owner handshake is shared by the background and bound CLI Sender.
#![cfg(target_os = "linux")]
pub fn inspect_owner() -> crate::Result<Option<neonmix_output_binding::owner::OwnerIdentity>> {
    neonmix_output_binding::owner::inspect_owner()
}
