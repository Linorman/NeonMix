use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackMode {
    #[default]
    LowLatency,
    Synchronized,
}

/// Closed administrator controls. Paths, executables and keys are never client supplied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AirplayAction {
    PlaybackMode {
        mode: PlaybackMode,
    },
    Enable,
    Disable,
    Disconnect,
    Allow,
    Revoke,
    Mix {
        gain_db: f32,
        muted: bool,
        solo: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirplayCommand {
    pub expected_revision: u64,
    pub operation: AirplayAction,
}

/// Explicit targets; no command can silently move to a replacement session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AirplayActionV2 {
    Configure {
        #[serde(default = "default_receiver_count")]
        receiver_count: usize,
        multi_receiver: bool,
    },
    Enable,
    Disable,
    EnableReceiver {
        receiver_id: String,
    },
    DisableReceiver {
        receiver_id: String,
    },
    PairReceiver {
        receiver_id: String,
    },
    RenameReceiver {
        receiver_id: String,
        name: String,
    },
    AliasSource {
        source_id: String,
        alias: Option<String>,
    },
    MixSource {
        source_id: String,
        session_id: u64,
        gain_db: f32,
        muted: bool,
        solo: bool,
    },
    /// Only supplied fields change; gain/mute persist per source while solo
    /// belongs to the identified active session. Requires patch_mix_source.
    PatchMixSource {
        source_id: String,
        session_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gain_db: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        muted: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        solo: Option<bool>,
    },
    DisconnectSource {
        source_id: String,
        session_id: u64,
    },
    RevokeSource {
        source_id: String,
        session_id: Option<u64>,
    },
    AllowSource {
        source_id: String,
    },
    RepairSource {
        source_id: String,
        receiver_id: String,
    },
    PlaybackMode {
        source_id: String,
        mode: PlaybackMode,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirplayCommandV2 {
    #[serde(
        default = "legacy_command_version",
        skip_serializing_if = "is_legacy_command_version"
    )]
    pub command_version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_config_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_event_sequence: Option<u64>,
    pub command_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    pub operation: AirplayActionV2,
}

fn default_receiver_count() -> usize {
    2
}

pub const AIRPLAY_COMMAND_VERSION: u16 = 3;
fn legacy_command_version() -> u16 {
    2
}
fn is_legacy_command_version(version: &u16) -> bool {
    *version == 2
}
impl AirplayActionV2 {
    pub fn config_condition(&self) -> bool {
        match self {
            Self::Enable | Self::Disable | Self::PairReceiver { .. } => false,
            Self::PatchMixSource { gain_db, muted, .. } => gain_db.is_some() || muted.is_some(),
            _ => true,
        }
    }
    pub fn runtime_condition(&self) -> bool {
        !matches!(self, Self::AliasSource { .. } | Self::AllowSource { .. })
    }
}
impl AirplayCommandV2 {
    pub fn bound(
        command_id: String,
        runtime_epoch: String,
        credential_id: String,
        config: u64,
        event: u64,
        operation: AirplayActionV2,
    ) -> Self {
        Self {
            command_version: AIRPLAY_COMMAND_VERSION,
            command_id,
            runtime_epoch: Some(runtime_epoch),
            credential_id: Some(credential_id),
            expected_config_revision: if operation.config_condition() {
                Some(config)
            } else {
                None
            },
            expected_event_sequence: if operation.runtime_condition() {
                Some(event)
            } else {
                None
            },
            expected_revision: None,
            operation,
        }
    }
    pub fn validate_version(&self) -> Result<(), &'static str> {
        match self.command_version {
            2 if self.expected_revision.is_some()
                && self.expected_config_revision.is_none()
                && self.expected_event_sequence.is_none() =>
            {
                Ok(())
            }
            AIRPLAY_COMMAND_VERSION
                if self.expected_revision.is_none()
                    && self.runtime_epoch.as_ref().is_some_and(|e| !e.is_empty())
                    && self.credential_id.as_ref().is_some_and(|id| !id.is_empty())
                    && (!self.operation.config_condition()
                        || self.expected_config_revision.is_some())
                    && (!self.operation.runtime_condition()
                        || self.expected_event_sequence.is_some()) =>
            {
                Ok(())
            }
            2 | AIRPLAY_COMMAND_VERSION => Err("upgrade_required"),
            _ => Err("incompatible_version"),
        }
    }
}
