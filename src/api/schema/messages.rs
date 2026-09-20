use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentSelfParams {
    pub caller_pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentMessageSendParams {
    pub caller_pane_id: String,
    pub target: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_nonce: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentLobbyConnectParams {
    pub caller_pane_id: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentLobbyListParams {
    pub caller_pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentLobbyTargetParams {
    pub caller_pane_id: String,
    pub lobby_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentLobbyRenameParams {
    pub caller_pane_id: String,
    pub lobby_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentLobbyRemoveMemberParams {
    pub caller_pane_id: String,
    pub lobby_id: String,
    /// Stable collaboration instance ID from `agent.lobby.list`.
    pub member_instance_id: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageBox {
    #[default]
    Inbox,
    Outbox,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentMessageListParams {
    pub caller_pane_id: String,
    #[serde(default)]
    pub mailbox: AgentMessageBox,
    #[serde(default)]
    pub after_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default)]
    pub unacknowledged_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentMessageTargetParams {
    pub caller_pane_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageState {
    Pending,
    Observed,
    Injected,
    Acknowledged,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessageWaitSelector {
    Inbox {
        #[serde(default)]
        after_sequence: u64,
    },
    State {
        message_id: String,
        until: Vec<AgentMessageState>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentMessageWaitParams {
    pub caller_pane_id: String,
    pub selector: AgentMessageWaitSelector,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentMessageParty {
    pub instance_id: String,
    pub terminal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentLobbyMember {
    #[serde(flatten)]
    pub party: AgentMessageParty,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentLobby {
    pub lobby_id: String,
    pub label: String,
    /// The member allowed to rename, remove members, or delete this lobby.
    /// Older handoff manifests did not record ownership.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_instance_id: Option<String>,
    /// Stable workspace scope for the one-space-per-workspace UI model.
    /// Ad-hoc pair lobbies and older handoff manifests leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub members: Vec<AgentLobbyMember>,
    pub created_at_unix_ms: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AgentMessage {
    pub message_id: String,
    pub sequence: u64,
    pub sender: AgentMessageParty,
    pub recipient: AgentMessageParty,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub state: AgentMessageState,
    pub created_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injected_at_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at_unix_ms: Option<u64>,
    /// A delivery was durably claimed, but the server stopped before it could
    /// prove whether the terminal submission completed. These messages are
    /// never retried automatically.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub delivery_uncertain: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_nonce: Option<String>,
}
