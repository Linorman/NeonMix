//! Private per-UID owner handshake; display names do not establish ownership.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(target_os = "linux")]
pub use crate::owner_linux::inspect_owner;

pub const OWNER_PROTOCOL_VERSION: u16 = 1;
pub const OWNER_MESSAGE_LIMIT: usize = 1024;
pub const INSPECT_OWNER: &[u8] = b"\0{\"version\":1,\"type\":\"inspect_owner\"}";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerInspection {
    pub version: u16,
    #[serde(rename = "type")]
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerIdentity {
    pub version: u16,
    pub instance_generation: Uuid,
    pub output_id: Option<Uuid>,
    pub ready: bool,
}
impl OwnerIdentity {
    pub fn new(instance: Option<&str>, output_id: Option<Uuid>) -> Result<Self, &'static str> {
        let instance_generation = instance
            .map(str::parse)
            .transpose()
            .map_err(|_| "invalid_argument")?
            .unwrap_or_else(Uuid::new_v4);
        if instance_generation.is_nil() || output_id.is_some_and(|id| id.is_nil()) {
            return Err("invalid_argument");
        }
        Ok(Self {
            version: OWNER_PROTOCOL_VERSION,
            instance_generation,
            output_id,
            ready: false,
        })
    }
    pub fn matches(&self, instance: Uuid, output_id: Uuid) -> Result<bool, &'static str> {
        if self.version != OWNER_PROTOCOL_VERSION || self.instance_generation.is_nil() {
            return Err("upgrade_required");
        }
        if self.instance_generation != instance || self.output_id != Some(output_id) {
            return Err("resource_owned_by_other_instance");
        }
        Ok(self.ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_requires_protocol_instance_and_binding_identity_even_when_the_node_is_ready() {
        let instance = Uuid::new_v4();
        let binding = Uuid::new_v4();
        assert!(OwnerIdentity::new(Some(&Uuid::nil().to_string()), Some(binding)).is_err());
        assert!(OwnerIdentity::new(None, Some(Uuid::nil())).is_err());
        let mut owner = OwnerIdentity::new(Some(&instance.to_string()), Some(binding)).unwrap();
        assert_eq!(owner.matches(instance, binding), Ok(false));
        owner.ready = true;
        assert_eq!(owner.matches(instance, binding), Ok(true));
        assert_eq!(
            owner.matches(Uuid::new_v4(), binding),
            Err("resource_owned_by_other_instance")
        );
        assert_eq!(
            owner.matches(instance, Uuid::new_v4()),
            Err("resource_owned_by_other_instance")
        );
        owner.version = 0;
        assert_eq!(owner.matches(instance, binding), Err("upgrade_required"));
    }
}
