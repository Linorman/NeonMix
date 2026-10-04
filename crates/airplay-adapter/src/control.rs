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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub command_id: String,
    pub expected_revision: u64,
    pub operation: AirplayActionV2,
}

fn default_receiver_count() -> usize {
    2
}
