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

/// Lifecycle position of a task, as the runtime reports it.
///
/// Mirrors the protocol's own states rather than collapsing them for display:
/// a client that cannot tell `Submitted` from `Verified` cannot show whether
/// anyone actually checked the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomTaskStateValue {
    Ready,
    Leased,
    Running,
    Blocked,
    Submitted,
    Verified,
    Failed,
    Cancelled,
}

/// One unit of answerable work in a room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomTaskInfo {
    pub task_id: String,
    pub room_id: String,
    pub title: String,
    /// Objective the task belongs to, so its cost stays attributable.
    pub root_id: String,
    /// The single seat answerable for it.
    pub owner_member_id: String,
    pub state: RoomTaskStateValue,
    /// Compare-and-swap token. Send it back as `expected_revision`.
    pub revision: u64,
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Why it is blocked, when it is.
    #[serde(default)]
    pub blocked_reasons: Vec<String>,
    /// Incarnation holding the lease, while one holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_holder_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at_unix_ms: Option<u64>,
    /// Lease generation. A command from an older epoch is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    pub attempts: u32,
    /// Whether a verdict currently stands. Submitted is not verified.
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomTaskListParams {
    pub room_id: String,
    /// Only tasks whose dependencies are met and whose lease has not lapsed.
    #[serde(default)]
    pub ready_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomTaskCreateParams {
    pub room_id: String,
    pub title: String,
    /// Seat answerable for the work.
    pub owner_member_id: String,
    /// Objective to charge it to. Omitting it starts a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_id: Option<String>,
    /// Tasks that must be verified before this one is ready.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Makes the command safe to retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_pane_id: Option<String>,
}

/// What a piece of evidence points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomEvidenceKindValue {
    RawPty,
    Transcript,
    Command,
    File,
}

/// A pointer to stored proof, with the digest it had when it was cited.
///
/// Submitting and verifying both require at least one, because a claim that
/// work is done is not the same as something a later reader can check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomEvidenceRefValue {
    pub kind: RoomEvidenceKindValue,
    /// Opaque locator: a path, a pane id, a record id.
    pub handle: String,
    pub digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version: Option<u64>,
}

/// A command against one existing task.
///
/// `expected_revision` is the revision the caller read. A mismatch is refused
/// rather than applied, so two agents acting on stale reads cannot both win.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RoomTaskCommandParams {
    pub room_id: String,
    pub task_id: String,
    pub expected_revision: u64,
    /// Required where the protocol demands a lease: start and submit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    /// Why the task failed. Required by `room.task.fail`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How long the claim should hold, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_ms: Option<u64>,
    /// Proof, required by submit and verify.
    #[serde(default)]
    pub evidence: Vec<RoomEvidenceRefValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_pane_id: Option<String>,
}
