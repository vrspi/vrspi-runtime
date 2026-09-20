use serde::{Deserialize, Serialize};

/// Who is calling a room method.
///
/// `host` is the authenticated operator acting through the TUI or CLI;
/// `caller_pane_id` identifies an agent acting as one of its own seats. The
/// server derives authority from this, never from a name inside message text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomCreateParams {
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub objective: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
pub struct RoomListParams {
    /// Restrict to one workspace. A room belongs to exactly one workspace, but
    /// a workspace may hold several rooms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomTargetParams {
    pub room_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomLifecycleValue {
    Active,
    Paused,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomLifecycleParams {
    pub room_id: String,
    pub lifecycle: RoomLifecycleValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomMemberAddParams {
    pub room_id: String,
    /// Room-local handle. Unique within the room, matched case-insensitively.
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Claim the single reserved coordinating seat. Its handle must be
    /// `orchestrator`.
    #[serde(default)]
    pub orchestrator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomMemberTargetParams {
    pub room_id: String,
    pub member_id: String,
}

/// Binds a live agent to a seat, or clears the binding with a null target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomMemberBindParams {
    pub room_id: String,
    pub member_id: String,
    /// Agent target: a unique live agent name or a pane ID hosting one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomPostParams {
    pub room_id: String,
    pub body: String,
    /// Pane ID of the agent posting. Omit when the host posts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_pane_id: Option<String>,
    /// Additional recipients beyond any `@handle` mentions in the body.
    #[serde(default)]
    pub recipients: Vec<String>,
    /// Retry key. Posting twice with the same key returns the first event
    /// instead of appending a second one and waking every recipient again.
    /// Reusing a key for different content is an error, not a retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_nonce: Option<String>,
    /// Objective this post belongs to. Omitting it starts a new objective with
    /// a fresh activation allowance; supplying it shares the existing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_id: Option<String>,
}

/// Explicit receipt of a delivered room event by the addressed agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomDeliveryAckParams {
    pub room_id: String,
    pub event_id: String,
    /// Required: the host cannot acknowledge comprehension for an agent.
    pub caller_pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomEventListParams {
    pub room_id: String,
    #[serde(default)]
    pub after_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomMemoryPutParams {
    pub room_id: String,
    /// Omit to create; supply with `expected_revision` to update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    /// Revision the writer read. A mismatch is a conflict, never a silent
    /// overwrite of a newer decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomMemorySearchParams {
    pub room_id: String,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomMemoryTargetParams {
    pub room_id: String,
    pub record_id: String,
    /// Pane of the agent acting for its own seat. Absent, the caller is the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomMemberGrantParams {
    pub room_id: String,
    pub member_id: String,
    /// Whether the seat may address the whole room with `@all`.
    pub may_broadcast: bool,
    /// Pane of the orchestrator granting it. Absent, the caller is the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_pane_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomAllowanceParams {
    pub room_id: String,
    pub root_id: String,
    pub additional: u32,
}

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoomActorInfo {
    Host,
    Member { member_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomMemberInfo {
    pub member_id: String,
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub orchestrator: bool,
    /// Whether this seat may address the whole room with `@all`.
    #[serde(default, skip_serializing_if = "super::is_false")]
    pub may_broadcast: bool,
    /// Live agent incarnation filling this seat, when one is bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_instance_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomInfo {
    pub room_id: String,
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub objective: String,
    pub lifecycle: RoomLifecycleValue,
    pub members: Vec<RoomMemberInfo>,
    pub event_count: u64,
    pub record_count: u64,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomEventInfo {
    pub event_id: String,
    pub sequence: u64,
    pub actor: RoomActorInfo,
    pub body: String,
    /// Members that will actually be activated. Everyone else can read this
    /// later without a model call.
    #[serde(default)]
    pub recipients: Vec<String>,
    pub root_id: String,
    pub created_at_unix_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomRecordStatusValue {
    Proposed,
    Accepted,
    Disputed,
    Stale,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomRecordInfo {
    pub record_id: String,
    pub revision: u64,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub status: RoomRecordStatusValue,
    pub updated_at_unix_ms: u64,
}

/// A short search hit. Bodies are fetched separately so a search response
/// cannot blow an activation packet's budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomRecordHit {
    pub record_id: String,
    pub revision: u64,
    pub title: String,
    pub summary: String,
    pub status: RoomRecordStatusValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomBudgetInfo {
    pub root_id: String,
    pub allowance: u32,
    pub used: u32,
    pub remaining: u32,
    pub paused: bool,
}
