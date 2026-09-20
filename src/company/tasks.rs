//! Task protocol and durable state for company rooms.
//!
//! Rooms already carry a conversation, seats, a per-recipient delivery ledger,
//! and shared knowledge. What they cannot express is *work*: who owns a piece
//! of it, whether it is ready, what evidence supports the claim that it is
//! finished, and whether that claim still holds after the artifact changed.
//! This module is that layer, and nothing else: pure data and transition
//! policy, testable without PTYs, async, or a model.
//!
//! It deliberately stops short of scheduling. Choosing which ready task to
//! activate, when to retry, and how to route to an adapter consumes these
//! types from the other side of an adapter boundary.
//!
//! Four rules carry the weight:
//!
//! 1. **A claim is not an outcome.** `Submitted` records what a worker says it
//!    did. `Verified` requires acceptance evidence naming the artifact version
//!    that was actually checked, so "delivered" and "believed" never stand in
//!    for "true".
//! 2. **Stale workers cannot win.** A lease carries an epoch. A submission
//!    from a previous epoch is refused rather than overwriting the work of the
//!    lease holder that replaced it.
//! 3. **Retries must be safe.** Every command carries an idempotency key. The
//!    same key applied twice yields the first event again and changes nothing,
//!    so a scheduler may retry without inventing duplicate history.
//! 4. **History is the state.** The ledger is exactly the fold of its event
//!    log, so replaying the log reproduces it. That is what makes a delivery
//!    bug diagnosable after the fact instead of only observable live.
//!
//! The types are consumed by a scheduler that lands separately, so the public
//! surface is currently exercised by this module's tests alone.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Wire version stamped on every recorded event.
///
/// Present from the first event so a later reader can tell which rules
/// produced a record rather than guessing from its shape.
pub(crate) const TASK_PROTOCOL_VERSION: u32 = 1;

/// Longest a lease may be held before the scheduler may reclaim it.
///
/// A worker that dies mid-task would otherwise own it forever; the scheduler
/// enforces this, and the ledger only records the deadline it was given.
pub(crate) const DEFAULT_LEASE_MS: u64 = 15 * 60 * 1000;

/// Cap on stored evidence text, mirroring the room event body limit so one
/// oversized transcript cannot bloat the session snapshot.
pub(crate) const MAX_EVIDENCE_BYTES: usize = 16 * 1024;

/// Where a task is in its life.
///
/// The shape is deliberately small: every transition below is a method on the
/// ledger, so an invalid one cannot be expressed by assigning a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskState {
    /// Owned and unblocked, but nobody is working on it.
    Ready,
    /// Claimed by a seat under a lease; work has not started.
    Leased,
    /// The lease holder reported that it started.
    Running,
    /// Waiting on something outside the task.
    Blocked,
    /// The worker claims it is done. Not a verdict.
    Submitted,
    /// Accepted against evidence for the current artifact version.
    Verified,
    /// Attempted and rejected; may be retried by a new attempt.
    Failed,
    /// Abandoned by the host or the orchestrator.
    Cancelled,
}

impl TaskState {
    /// Whether the task is finished for scheduling purposes.
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Verified | Self::Cancelled)
    }
}

/// What kind of proof an evidence handle points at.
///
/// Raw pane output and a parsed transcript are kept apart on purpose: the
/// parsed form is convenient and lossy, and a disputed result has to be
/// checkable against what the terminal actually showed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceKind {
    /// Bytes as the pane emitted them.
    RawPty,
    /// Parsed, human-readable rendering of the same output.
    Transcript,
    /// A command and its exit status.
    Command,
    /// A file or artifact path at a known version.
    File,
}

/// A pointer to stored proof, plus the digest it had when recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidenceRef {
    pub kind: EvidenceKind,
    /// Opaque locator: a path, a pane id, a record id. Interpreted by whoever
    /// stored it, never by this module.
    pub handle: String,
    /// Digest of the referenced content, so a later reader can tell whether
    /// what it fetched is what was cited.
    pub digest: String,
    /// Artifact version this evidence was taken against, when it concerns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version: Option<u64>,
}

impl EvidenceRef {
    pub(crate) fn is_valid(&self) -> bool {
        !self.handle.trim().is_empty()
            && !self.digest.trim().is_empty()
            && self.handle.len() <= MAX_EVIDENCE_BYTES
    }
}

/// Who accepted a submission, under which rule, against which bytes.
///
/// Recording the verifier and rule version alongside the artifact identity is
/// what makes a verdict re-checkable later: "it passed" is only meaningful
/// with "checked by whom, under which rules, against what".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Verification {
    /// Seat or rule engine that accepted it.
    pub verifier: String,
    /// Version of the accepting rules, so a later rule change is detectable.
    pub rule_version: u64,
    pub artifact_version: Option<u64>,
    /// Digest of the artifact at the moment of acceptance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_digest: Option<String>,
    pub verified_at_unix_ms: u64,
}

/// What a submission says it produced.
///
/// Stated explicitly rather than sniffed from the first evidence item:
/// evidence may be a command or a transcript, whose digest is not the
/// artifact's, and guessing produced an artifact identity nobody claimed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ArtifactClaim {
    pub artifact_id: String,
    pub version: u64,
    pub digest: String,
}

/// A versioned thing the work produces.
///
/// Version is a counter rather than a hash so that "newer" is decidable
/// without fetching content; the digest records identity at that version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Artifact {
    pub artifact_id: String,
    pub version: u64,
    pub digest: String,
    /// Task whose verified work last advanced this artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub produced_by: Option<String>,
    pub updated_at_unix_ms: u64,
}

/// Something preventing progress, recorded rather than folded into prose.
///
/// Blockers are never dropped silently: clearing one is its own event, so a
/// coalescing step cannot quietly lose the reason work stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Blocker {
    pub blocker_id: String,
    pub reason: String,
    pub raised_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared_at_unix_ms: Option<u64>,
}

impl Blocker {
    pub(crate) fn is_open(&self) -> bool {
        self.cleared_at_unix_ms.is_none()
    }
}

/// Exclusive claim on a task by one seat's live incarnation.
///
/// The epoch increments on every claim. A submission carrying an older epoch
/// belongs to a worker that has since been replaced, and is refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Lease {
    pub member_id: String,
    /// Incarnation holding the lease. Seats outlive processes, so ownership
    /// has to name the process, not only the seat.
    pub instance_id: String,
    pub epoch: u64,
    pub attempt_id: String,
    pub expires_at_unix_ms: u64,
}

/// One unit of owned work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Task {
    pub task_id: String,
    pub room_id: String,
    pub title: String,
    /// Objective this task belongs to, so activation spend stays attributable
    /// to the room budget that authorised it.
    pub root_id: String,
    /// The single seat answerable for it.
    pub owner_member_id: String,
    pub state: TaskState,
    /// Bumped on every accepted change; the compare-and-swap token.
    pub revision: u64,
    /// Tasks that must reach `Verified` before this one is ready.
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<Blocker>,
    /// Artifact this task advances, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    /// Artifact version the current claim or verdict was made against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<Lease>,
    /// Attempts started so far, for retry budgeting by the scheduler.
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
    /// The accepted verdict, while one stands. Cleared when the artifact or a
    /// dependency moves, because it described the old state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<Verification>,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

impl Task {
    pub(crate) fn open_blockers(&self) -> impl Iterator<Item = &Blocker> {
        self.blockers.iter().filter(|blocker| blocker.is_open())
    }

    pub(crate) fn has_open_blocker(&self) -> bool {
        self.open_blockers().next().is_some()
    }
}

/// Who issued a command.
///
/// Mirrors the room `Actor`: the host is a first-class principal, so no seat
/// can claim host authority by naming itself one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TaskActor {
    Host,
    Member { member_id: String },
}

impl TaskActor {
    pub(crate) fn is_host(&self) -> bool {
        matches!(self, Self::Host)
    }

    pub(crate) fn member_id(&self) -> Option<&str> {
        match self {
            Self::Host => None,
            Self::Member { member_id } => Some(member_id),
        }
    }
}

/// A recorded change. The log of these *is* the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TaskEvent {
    pub protocol_version: u32,
    pub event_id: String,
    /// Monotonic within a ledger; replay order.
    pub sequence: u64,
    pub task_id: String,
    /// Attempt this event belongs to, when one was live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    /// Event this one answers, so causality survives compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caused_by: Option<String>,
    pub actor: TaskActor,
    pub kind: TaskEventKind,
    /// Key that made this event unique. Re-applying it is a no-op.
    pub idempotency_key: String,
    /// Canonical identity of the command that produced it, so a rebuilt
    /// ledger can still tell an exact retry from a different command reusing
    /// the same key.
    pub fingerprint: String,
    /// Task revision produced by this event.
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version: Option<u64>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
    pub created_at_unix_ms: u64,
}

/// What an event did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum TaskEventKind {
    Created {
        title: String,
        owner_member_id: String,
        room_id: String,
        root_id: String,
        depends_on: Vec<String>,
        artifact_id: Option<String>,
    },
    Claimed {
        epoch: u64,
        instance_id: String,
        expires_at_unix_ms: u64,
    },
    Started,
    Blocked {
        blocker_id: String,
        reason: String,
    },
    Unblocked {
        blocker_id: String,
    },
    Submitted {
        /// What the submission said it produced, so the artifact identity is
        /// in the journal rather than inferred at replay time.
        artifact: Option<ArtifactClaim>,
    },
    Verified {
        verifier: String,
        rule_version: u64,
    },
    Failed {
        reason: String,
    },
    Cancelled {
        reason: String,
    },
    /// The artifact moved to a new version. Recorded whether or not any task
    /// was affected, because the registry changed either way.
    ArtifactAdvanced {
        artifact_id: String,
        version: u64,
        digest: String,
    },
    /// The artifact moved on, so an earlier verdict no longer describes it.
    ArtifactInvalidated {
        artifact_id: String,
        version: u64,
        digest: String,
    },
    /// A dependency stopped being verified, so a verdict resting on it no
    /// longer rests on anything.
    DependencyInvalidated {
        dependency_id: String,
    },
    /// A dependency was declared after creation.
    DependencyAdded {
        dependency_id: String,
    },
    /// A lease was released because the process holding it is gone.
    LeaseReleased {
        epoch: u64,
    },
    /// Ownership moved to another seat.
    Reassigned {
        owner_member_id: String,
    },
}

/// Which command was issued.
///
/// Recorded on a rejection so a receipt says what was refused, not merely
/// that something was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CommandKind {
    Create,
    Claim,
    Start,
    Submit,
    Verify,
    Fail,
    Block,
    Unblock,
    AddDependency,
    AdvanceArtifact,
    ProposeDecision,
    AcceptDecision,
}

impl CommandKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Claim => "claim",
            Self::Start => "start",
            Self::Submit => "submit",
            Self::Verify => "verify",
            Self::Fail => "fail",
            Self::Block => "block",
            Self::Unblock => "unblock",
            Self::AddDependency => "add_dependency",
            Self::AdvanceArtifact => "advance_artifact",
            Self::ProposeDecision => "propose_decision",
            Self::AcceptDecision => "accept_decision",
        }
    }

    /// Whether the command may omit `expected_revision`.
    ///
    /// Only creation may: there is no prior revision to compare against. Every
    /// other command states the revision it believes it is changing, which is
    /// what makes concurrent writers detectable instead of last-write-wins.
    fn may_omit_revision(self) -> bool {
        matches!(
            self,
            Self::Create | Self::AdvanceArtifact | Self::ProposeDecision
        )
    }
}

/// Identity of a command's *meaning*, used to tell a retry from a different
/// command that happens to reuse a key.
///
/// Built from semantic fields only. Transport-level noise — when the retry was
/// sent, which connection carried it — is deliberately excluded, so the same
/// intent fingerprints identically however many times it is resent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommandDescriptor {
    pub kind: CommandKind,
    pub task_id: String,
    pub expected_revision: Option<u64>,
    pub epoch: Option<u64>,
    /// Payload fields that change what the command means.
    pub payload: Vec<String>,
}

/// Joins fields into one string that two different requests cannot collide on.
///
/// Escaping the separator is the entire point. Without it, moving a delimiter
/// across a field boundary produces the same string from different input, and
/// anything keyed on that string — a retried command, a resent room post —
/// would treat a genuinely new request as a duplicate and silently drop it.
pub(crate) fn canonical_fingerprint<'a>(fields: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for (index, field) in fields.into_iter().enumerate() {
        if index > 0 {
            out.push('|');
        }
        out.push_str(&field.replace('\\', "\\\\").replace('|', "\\p"));
    }
    out
}

impl CommandDescriptor {
    fn fingerprint(&self) -> String {
        let mut out = String::new();
        out.push_str(self.kind.as_str());
        out.push('|');
        out.push_str(&self.task_id);
        out.push('|');
        match self.expected_revision {
            Some(revision) => out.push_str(&revision.to_string()),
            None => out.push('-'),
        }
        out.push('|');
        match self.epoch {
            Some(epoch) => out.push_str(&epoch.to_string()),
            None => out.push('-'),
        }
        if !self.payload.is_empty() {
            out.push('|');
            out.push_str(&canonical_fingerprint(
                self.payload.iter().map(String::as_str),
            ));
        }
        out
    }
}

/// A durable record that a command was refused.
///
/// Distinct from a task transition on purpose: it changes nothing about the
/// work, and replaying it must not touch the task projection. It exists so a
/// refusal is observable after the fact — "the successor rejected a dead
/// worker's result at 14:02" — rather than vanishing into a return value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RejectionReceipt {
    pub protocol_version: u32,
    pub receipt_id: String,
    /// Position in the same ordered journal as accepted events.
    pub sequence: u64,
    pub command: CommandKind,
    /// Canonical identity of the refused command.
    pub fingerprint: String,
    pub idempotency_key: String,
    pub actor: TaskActor,
    pub task_id: String,
    /// Attempt and epoch the caller submitted under, when it named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_attempt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_epoch: Option<u64>,
    /// What the ledger actually held at that moment.
    pub observed_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_epoch: Option<u64>,
    pub error_code: String,
    /// The structured refusal, so a retry returns the same answer.
    pub cause: TaskError,
    pub created_at_unix_ms: u64,
}

/// Where a decision stands.
///
/// Proposing and accepting are separate acts, as with task verdicts: an
/// author may put a decision to the room, but cannot ratify its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DecisionStatus {
    Proposed,
    Accepted,
    /// Replaced by a later decision, which names it.
    Superseded,
    Rejected,
}

/// A recorded decision: what was decided, why, and on whose authority.
///
/// Decisions are durable room state rather than prose in a message, because
/// the reason work was shaped a certain way outlives the conversation that
/// produced it, and a superseded decision has to stay readable to explain
/// what changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DecisionRecord {
    pub decision_id: String,
    pub room_id: String,
    pub title: String,
    /// What was decided, in one statement.
    pub statement: String,
    /// Why, so a later reader can tell whether the reason still holds.
    pub rationale: String,
    pub status: DecisionStatus,
    pub proposed_by: TaskActor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_by: Option<TaskActor>,
    /// The decision this one replaces, and the one that replaced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
    /// Compare-and-swap token, as for tasks.
    pub revision: u64,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

/// A change to the decision projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DecisionEvent {
    pub protocol_version: u32,
    pub event_id: String,
    pub sequence: u64,
    pub decision_id: String,
    /// The event this one answers, so an Accepted parent and its Superseded
    /// child keep explicit causality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caused_by: Option<String>,
    pub actor: TaskActor,
    pub kind: DecisionEventKind,
    pub idempotency_key: String,
    pub fingerprint: String,
    pub revision: u64,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
    pub created_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum DecisionEventKind {
    Proposed {
        room_id: String,
        title: String,
        statement: String,
        rationale: String,
        supersedes: Option<String>,
    },
    Accepted,
    Rejected {
        reason: String,
    },
    /// Recorded on the *old* decision when a new one replaces it.
    Superseded {
        by: String,
    },
}

/// One ordered entry in the ledger's journal.
///
/// Accepted work and refused commands share one order, so "what happened, in
/// what sequence" is answerable from a single list. Only the first kind moves
/// the task projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub(crate) enum JournalEntry {
    Event(TaskEvent),
    Decision(DecisionEvent),
    Rejection(RejectionReceipt),
}

impl JournalEntry {
    pub(crate) fn sequence(&self) -> u64 {
        match self {
            Self::Event(event) => event.sequence,
            Self::Decision(event) => event.sequence,
            Self::Rejection(receipt) => receipt.sequence,
        }
    }

    pub(crate) fn as_event(&self) -> Option<&TaskEvent> {
        match self {
            Self::Event(event) => Some(event),
            _ => None,
        }
    }

    pub(crate) fn as_decision(&self) -> Option<&DecisionEvent> {
        match self {
            Self::Decision(event) => Some(event),
            _ => None,
        }
    }

    pub(crate) fn as_rejection(&self) -> Option<&RejectionReceipt> {
        match self {
            Self::Rejection(receipt) => Some(receipt),
            _ => None,
        }
    }
}

/// What became of a command that carried a given idempotency key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub(crate) enum CommandOutcome {
    Accepted { event_ids: Vec<String> },
    Rejected { receipt_id: String },
}

/// The idempotency projection: one entry per key ever used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AppliedCommand {
    pub fingerprint: String,
    pub outcome: CommandOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TaskError {
    TaskNotFound,
    TaskExists,
    TitleEmpty,
    OwnerEmpty,
    IdempotencyKeyEmpty,
    /// The caller's expected revision is not the task's current revision.
    RevisionConflict {
        expected: u64,
        actual: u64,
    },
    /// Transition is not legal from the task's current state.
    InvalidTransition {
        from: TaskState,
        to: String,
    },
    /// The task is leased by someone else, or by an older epoch.
    LeaseHeld {
        holder: String,
    },
    StaleEpoch {
        expected: u64,
        actual: u64,
    },
    NotLeaseHolder,
    /// Only the host or the owning seat may do this.
    NotAuthorized,
    /// Verification needs evidence naming the current artifact version.
    EvidenceRequired,
    EvidenceInvalid,
    /// Acceptance cited a version other than the one on the task.
    EvidenceVersionMismatch {
        cited: Option<u64>,
        current: Option<u64>,
    },
    /// Acceptance cited content other than the artifact's current bytes.
    EvidenceDigestMismatch {
        artifact_id: String,
    },
    VerifierEmpty,
    DecisionNotFound,
    /// A decision that has been superseded or rejected cannot change again.
    DecisionSettled,
    StatementEmpty,
    /// The same idempotency key was reused for a different command.
    IdempotencyConflict {
        key: String,
    },
    /// A non-create command omitted the revision it believes it is changing.
    RevisionRequired,
    /// Work may only depend on work in the same room.
    CrossRoomDependency {
        task_id: String,
    },
    /// A dependency is missing, unfinished, or would form a cycle.
    DependencyMissing {
        task_id: String,
    },
    DependencyCycle {
        task_id: String,
    },
    /// The task still has an unresolved blocker.
    BlockerOpen {
        blocker_id: String,
    },
    BlockerNotFound,
}

impl TaskError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::TaskNotFound => "task_not_found",
            Self::TaskExists => "task_exists",
            Self::TitleEmpty => "task_title_empty",
            Self::OwnerEmpty => "task_owner_empty",
            Self::IdempotencyKeyEmpty => "task_idempotency_key_empty",
            Self::RevisionConflict { .. } => "task_revision_conflict",
            Self::InvalidTransition { .. } => "task_invalid_transition",
            Self::LeaseHeld { .. } => "task_lease_held",
            Self::StaleEpoch { .. } => "task_stale_epoch",
            Self::NotLeaseHolder => "task_not_lease_holder",
            Self::NotAuthorized => "task_not_authorized",
            Self::EvidenceRequired => "task_evidence_required",
            Self::EvidenceInvalid => "task_evidence_invalid",
            Self::EvidenceVersionMismatch { .. } => "task_evidence_version_mismatch",
            Self::EvidenceDigestMismatch { .. } => "task_evidence_digest_mismatch",
            Self::VerifierEmpty => "task_verifier_empty",
            Self::DecisionNotFound => "decision_not_found",
            Self::DecisionSettled => "decision_settled",
            Self::StatementEmpty => "decision_statement_empty",
            Self::IdempotencyConflict { .. } => "task_idempotency_conflict",
            Self::RevisionRequired => "task_revision_required",
            Self::CrossRoomDependency { .. } => "task_cross_room_dependency",
            Self::DependencyMissing { .. } => "task_dependency_missing",
            Self::DependencyCycle { .. } => "task_dependency_cycle",
            Self::BlockerOpen { .. } => "task_blocker_open",
            Self::BlockerNotFound => "task_blocker_not_found",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::TaskNotFound => "task was not found".into(),
            Self::TaskExists => "a task with that id already exists".into(),
            Self::TitleEmpty => "task title must not be empty".into(),
            Self::OwnerEmpty => "a task needs exactly one owning seat".into(),
            Self::IdempotencyKeyEmpty => "every command needs an idempotency key".into(),
            Self::RevisionConflict { expected, actual } => {
                format!("task changed since you read it (expected r{expected}, now r{actual})")
            }
            Self::InvalidTransition { from, to } => {
                format!("cannot {to} a task that is {from:?}")
            }
            Self::LeaseHeld { holder } => format!("task is leased by {holder}"),
            Self::StaleEpoch { expected, actual } => {
                format!("lease epoch {actual} is stale; the task is on epoch {expected}")
            }
            Self::NotLeaseHolder => "only the lease holder may report on this attempt".into(),
            Self::NotAuthorized => "only the host or the owning seat may do this".into(),
            Self::EvidenceRequired => {
                "verification needs acceptance evidence for the current artifact version".into()
            }
            Self::EvidenceInvalid => "evidence needs a handle and a digest".into(),
            Self::EvidenceVersionMismatch { cited, current } => {
                format!("evidence cites artifact version {cited:?} but the task is at {current:?}")
            }
            Self::EvidenceDigestMismatch { artifact_id } => {
                format!("evidence does not cite the current bytes of {artifact_id}")
            }
            Self::VerifierEmpty => "acceptance must name its verifier".into(),
            Self::DecisionNotFound => "decision was not found".into(),
            Self::DecisionSettled => "a superseded or rejected decision cannot change".into(),
            Self::StatementEmpty => "a decision needs a statement".into(),
            Self::IdempotencyConflict { key } => {
                format!("idempotency key {key} was already used for a different command")
            }
            Self::RevisionRequired => {
                "this command must state the task revision it is changing".into()
            }
            Self::CrossRoomDependency { task_id } => {
                format!("{task_id} belongs to another room; declare a cross-room contract first")
            }
            Self::DependencyMissing { task_id } => format!("dependency {task_id} does not exist"),
            Self::DependencyCycle { task_id } => {
                format!("dependency on {task_id} would form a cycle")
            }
            Self::BlockerOpen { blocker_id } => {
                format!("blocker {blocker_id} is still open")
            }
            Self::BlockerNotFound => "blocker was not found".into(),
        }
    }
}

/// A command, as data.
///
/// The scheduler names what it wants done and hands it to a [`TaskStore`];
/// it never reaches into the ledger. Having the command set as one type is
/// what makes that boundary checkable: a new command has to appear here, and
/// anything that can be applied can also be logged, queued, or replayed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub(crate) enum TaskMutation {
    Create {
        room_id: String,
        root_id: String,
        title: String,
        owner_member_id: String,
        depends_on: Vec<String>,
        artifact_id: Option<String>,
    },
    Claim {
        task_id: String,
        instance_id: String,
        lease_ms: u64,
    },
    Start {
        task_id: String,
        epoch: u64,
    },
    Submit {
        task_id: String,
        epoch: u64,
        artifact: Option<ArtifactClaim>,
        evidence: Vec<EvidenceRef>,
    },
    Verify {
        task_id: String,
        verifier: String,
        rule_version: u64,
        acceptance: Vec<EvidenceRef>,
    },
    Fail {
        task_id: String,
        epoch: Option<u64>,
        reason: String,
    },
    Block {
        task_id: String,
        epoch: Option<u64>,
        reason: String,
    },
    Unblock {
        task_id: String,
        blocker_id: String,
    },
    AddDependency {
        task_id: String,
        dependency_id: String,
    },
    AdvanceArtifact {
        artifact_id: String,
        digest: String,
    },
    ProposeDecision {
        room_id: String,
        title: String,
        statement: String,
        rationale: String,
        supersedes: Option<String>,
        evidence: Vec<EvidenceRef>,
    },
    AcceptDecision {
        decision_id: String,
    },
}

/// What an accepted command produced.
///
/// Empty `events` is a legitimate result: a command whose content the ledger
/// already holds changes nothing and says so, rather than inventing history.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CommandResult {
    pub events: Vec<TaskEvent>,
    pub decisions: Vec<DecisionEvent>,
}

impl CommandResult {
    pub(crate) fn is_noop(&self) -> bool {
        self.events.is_empty() && self.decisions.is_empty()
    }
}

/// Why a command was refused, and the durable proof when one was written.
///
/// A caller that lost a race gets both: the error to act on, and the receipt
/// id to cite. A caller that was malformed or unauthorized gets only the
/// error, because nothing was written for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandRejection {
    pub error: TaskError,
    /// Boxed so the common path — an accepted command — does not carry the
    /// size of a receipt it will never use.
    pub receipt: Option<Box<RejectionReceipt>>,
}

impl CommandRejection {
    pub(crate) fn code(&self) -> &'static str {
        self.error.code()
    }

    /// Whether the refusal is recorded in the journal.
    pub(crate) fn is_durable(&self) -> bool {
        self.receipt.is_some()
    }
}

/// A failure of the storage beneath the ledger, not of the command.
///
/// Kept outside the domain outcome on purpose: "the disk did not accept this"
/// and "the rules refused this" are different facts, and a caller that cannot
/// tell them apart will retry the wrong one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoreError {
    /// The journal delta could not be made durable. Nothing was published.
    Persistence { detail: String },
}

impl StoreError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Persistence { .. } => "store_persistence_failed",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Persistence { detail } => {
                format!("the journal could not be committed: {detail}")
            }
        }
    }
}

/// What a committed command turned out to be.
///
/// Both variants mean the same thing about durability: the journal delta is
/// on disk. They differ only in what the rules decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommittedOutcome {
    Applied(CommandResult),
    Rejected(CommandRejection),
}

impl CommittedOutcome {
    pub(crate) fn applied(self) -> Option<CommandResult> {
        match self {
            Self::Applied(result) => Some(result),
            Self::Rejected(_) => None,
        }
    }

    pub(crate) fn rejection(&self) -> Option<&CommandRejection> {
        match self {
            Self::Rejected(rejection) => Some(rejection),
            Self::Applied(_) => None,
        }
    }
}

/// The journal delta a command produced, held before it is made durable.
///
/// Staging exists so the live projection is never moved by a command whose
/// record has not been committed. The reducer computes the whole outcome on a
/// copy; the store persists the delta; only then is the projection published.
#[derive(Debug, Clone)]
pub(crate) struct StagedCommit {
    /// Entries this command appends, in order.
    pub delta: Vec<JournalEntry>,
    pub outcome: CommittedOutcome,
    /// The projection as it will be once published.
    next: TaskLedger,
}

impl StagedCommit {
    pub(crate) fn delta(&self) -> &[JournalEntry] {
        &self.delta
    }
}

/// Where a committed journal delta is made durable.
///
/// The runtime supplies this; the protocol layer only requires that it either
/// succeeds or reports a [`StoreError`], never both.
pub(crate) trait JournalSink {
    fn persist(&mut self, delta: &[JournalEntry]) -> Result<(), StoreError>;
}

/// The boundary a scheduler codes against.
///
/// Read methods answer "what may be done now"; [`TaskStore::apply`] is the
/// only way to change anything. Implementations own their internals, so a
/// scheduler cannot reach past this trait into ledger state, and a test can
/// substitute a mock without the scheduler noticing.
pub(crate) trait TaskStore {
    /// Applies one command. Accepted commands return what they recorded;
    /// refused ones return the error and, for a stale attempt, its receipt.
    fn apply(
        &mut self,
        ctx: CommandContext,
        mutation: TaskMutation,
    ) -> Result<CommittedOutcome, StoreError>;

    fn task(&self, task_id: &str) -> Option<&Task>;
    fn tasks(&self) -> &[Task];
    /// Work a scheduler may hand out at `now_unix_ms`, decided from durable
    /// state alone.
    fn ready_tasks(&self, now_unix_ms: u64) -> Vec<&Task>;
    fn artifact(&self, artifact_id: &str) -> Option<&Artifact>;
    fn decision(&self, decision_id: &str) -> Option<&DecisionRecord>;
    /// The ordered journal, for observability and replay.
    fn journal(&self) -> &[JournalEntry];
    fn receipt(&self, receipt_id: &str) -> Option<&RejectionReceipt>;

    /// One bounded page of the journal, after `cursor`, in sequence order.
    ///
    /// Consumers must never have to hold the whole journal to make progress;
    /// bounded reads are what keeps context bounded as history grows.
    fn journal_page(&self, cursor: Option<u64>, limit: usize) -> JournalPage;

    /// Ready work, deterministically ordered and capped.
    fn ready_page(&self, now_unix_ms: u64, limit: usize) -> Vec<&Task>;
}

/// A bounded slice of the journal, with the cursor to continue from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JournalPage {
    pub entries: Vec<JournalEntry>,
    /// Sequence to pass as the next `cursor`, or `None` at the end.
    pub next_cursor: Option<u64>,
}

/// Durable task state for one company, and the log that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct TaskLedger {
    #[serde(default)]
    tasks: Vec<Task>,
    #[serde(default)]
    artifacts: Vec<Artifact>,
    #[serde(default)]
    decisions: Vec<DecisionRecord>,
    /// Append-only history: accepted events and refused commands in one
    /// order. Replaying it rebuilds every projection below.
    #[serde(default)]
    journal: Vec<JournalEntry>,
    /// One entry per idempotency key ever used, with the command's
    /// fingerprint and what became of it. A retry is answered from here; a
    /// key reused for different work is refused from here.
    #[serde(default)]
    applied: HashMap<String, AppliedCommand>,
    #[serde(default)]
    next_task_id: u64,
    #[serde(default)]
    next_event_id: u64,
    #[serde(default)]
    next_receipt_id: u64,
    #[serde(default)]
    next_decision_id: u64,
    #[serde(default)]
    next_attempt_id: u64,
    #[serde(default)]
    next_blocker_id: u64,
}

/// Who a command requires its caller to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandAuthority {
    /// The seat answerable for the task, or the host.
    OwnerOrHost,
    /// Anyone but the seat answerable for it. Accepting your own work is not
    /// review, so the owner is excluded rather than merely discouraged.
    ReviewerOrHost,
}

/// Fields every command carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommandContext {
    pub actor: TaskActor,
    /// Makes the command safe to retry.
    pub idempotency_key: String,
    /// Revision the caller believes the task is at. `None` skips the check,
    /// which only creation may do.
    pub expected_revision: Option<u64>,
    /// Event this command answers.
    pub caused_by: Option<String>,
    pub now_unix_ms: u64,
}

impl TaskLedger {
    pub(crate) fn is_empty(&self) -> bool {
        self.tasks.is_empty() && self.journal.is_empty()
    }

    pub(crate) fn task(&self, task_id: &str) -> Option<&Task> {
        self.tasks.iter().find(|task| task.task_id == task_id)
    }

    pub(crate) fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// The ordered journal: accepted events and refusals together.
    pub(crate) fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }

    /// Accepted events only, for readers that care about the task projection.
    pub(crate) fn events(&self) -> impl Iterator<Item = &TaskEvent> {
        self.journal.iter().filter_map(JournalEntry::as_event)
    }

    /// Durable refusals, newest last.
    pub(crate) fn rejections(&self) -> impl Iterator<Item = &RejectionReceipt> {
        self.journal.iter().filter_map(JournalEntry::as_rejection)
    }

    pub(crate) fn receipt(&self, receipt_id: &str) -> Option<&RejectionReceipt> {
        self.rejections()
            .find(|receipt| receipt.receipt_id == receipt_id)
    }

    pub(crate) fn artifact(&self, artifact_id: &str) -> Option<&Artifact> {
        self.artifacts
            .iter()
            .find(|artifact| artifact.artifact_id == artifact_id)
    }

    /// Tasks a scheduler may hand out right now.
    ///
    /// Readiness is decided here, from durable state alone: no model is asked
    /// what to do next, which is what keeps board writes from waking anyone.
    pub(crate) fn ready_tasks(&self, now_unix_ms: u64) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|task| self.is_ready(task, now_unix_ms))
            .collect()
    }

    fn is_ready(&self, task: &Task, now_unix_ms: u64) -> bool {
        let claimable = match task.state {
            TaskState::Ready | TaskState::Failed => true,
            // A lease that has run out returns the task to the pool; the
            // worker holding it may have died without saying so.
            TaskState::Leased | TaskState::Running => task
                .lease
                .as_ref()
                .is_some_and(|lease| lease.expires_at_unix_ms <= now_unix_ms),
            _ => false,
        };
        claimable
            && !task.has_open_blocker()
            && task.depends_on.iter().all(|dependency| {
                self.task(dependency)
                    .is_some_and(|dep| dep.state == TaskState::Verified)
            })
    }

    /// Creates a task owned by exactly one seat.
    pub(crate) fn create(
        &mut self,
        ctx: CommandContext,
        room_id: String,
        root_id: String,
        title: String,
        owner_member_id: String,
        depends_on: Vec<String>,
        artifact_id: Option<String>,
    ) -> Result<TaskEvent, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::Create,
            task_id: String::new(),
            expected_revision: None,
            epoch: None,
            payload: vec![
                room_id.clone(),
                root_id.clone(),
                title.clone(),
                owner_member_id.clone(),
                depends_on.join(","),
                artifact_id.clone().unwrap_or_default(),
            ],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return existing.into_iter().next().ok_or(TaskError::TaskNotFound);
        }
        if title.trim().is_empty() {
            return Err(TaskError::TitleEmpty);
        }
        if owner_member_id.trim().is_empty() {
            return Err(TaskError::OwnerEmpty);
        }
        // Only the host or the owner may put work on the board, so a worker
        // cannot invent its own assignments.
        if !ctx.actor.is_host() && ctx.actor.member_id() != Some(owner_member_id.as_str()) {
            return Err(TaskError::NotAuthorized);
        }
        for dependency in &depends_on {
            let Some(dependency_task) = self.task(dependency) else {
                return Err(TaskError::DependencyMissing {
                    task_id: dependency.clone(),
                });
            };
            // Work may only wait on work its own room can see. A cross-room
            // edge would make one room's readiness depend on state its
            // members cannot read, so it needs an explicit contract first.
            if dependency_task.room_id != room_id {
                return Err(TaskError::CrossRoomDependency {
                    task_id: dependency.clone(),
                });
            }
        }

        self.next_task_id = self.next_task_id.saturating_add(1);
        let task_id = format!("task_{:x}", self.next_task_id);
        let task = Task {
            task_id: task_id.clone(),
            room_id: room_id.clone(),
            title: title.clone(),
            root_id: root_id.clone(),
            owner_member_id: owner_member_id.clone(),
            state: TaskState::Ready,
            revision: 1,
            depends_on,
            blockers: Vec::new(),
            artifact_id,
            artifact_version: None,
            lease: None,
            attempts: 0,
            evidence: Vec::new(),
            verification: None,
            created_at_unix_ms: ctx.now_unix_ms,
            updated_at_unix_ms: ctx.now_unix_ms,
        };
        let depends_on = task.depends_on.clone();
        let artifact_id = task.artifact_id.clone();
        self.tasks.push(task);
        Ok(self.record(
            &ctx,
            &descriptor,
            &task_id,
            None,
            TaskEventKind::Created {
                title,
                owner_member_id,
                room_id,
                root_id,
                depends_on,
                artifact_id,
            },
            1,
            None,
            Vec::new(),
        ))
    }

    /// Claims a task for one incarnation, starting a new attempt.
    ///
    /// The epoch advances on every claim, which is what lets a submission from
    /// a replaced worker be told apart from the current one's.
    pub(crate) fn claim(
        &mut self,
        ctx: CommandContext,
        task_id: &str,
        instance_id: String,
        lease_ms: u64,
    ) -> Result<TaskEvent, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::Claim,
            task_id: task_id.to_string(),
            expected_revision: ctx.expected_revision,
            epoch: None,
            payload: vec![instance_id.clone()],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return existing.into_iter().next().ok_or(TaskError::TaskNotFound);
        }
        self.validate(
            &ctx,
            &descriptor,
            task_id,
            None,
            false,
            CommandAuthority::OwnerOrHost,
        )?;
        let now = ctx.now_unix_ms;
        let ready = {
            let task = self.require(task_id)?;
            self.is_ready(task, now)
        };
        if !ready {
            let task = self.require(task_id)?;
            if let Some(blocker) = task.blockers.iter().find(|blocker| blocker.is_open()) {
                return Err(TaskError::BlockerOpen {
                    blocker_id: blocker.blocker_id.clone(),
                });
            }
            if let Some(lease) = task.lease.as_ref() {
                if lease.expires_at_unix_ms > now {
                    return Err(TaskError::LeaseHeld {
                        holder: lease.member_id.clone(),
                    });
                }
            }
            if task.state.is_terminal() || task.state == TaskState::Submitted {
                return Err(TaskError::InvalidTransition {
                    from: task.state,
                    to: "claim".into(),
                });
            }
            return Err(TaskError::DependencyMissing {
                task_id: task
                    .depends_on
                    .first()
                    .cloned()
                    .unwrap_or_else(|| task_id.to_string()),
            });
        }
        self.next_attempt_id = self.next_attempt_id.saturating_add(1);
        let attempt_id = format!("attempt_{:x}", self.next_attempt_id);
        let task = self.require_mut(task_id)?;
        let owner = task.owner_member_id.clone();
        let epoch = task.lease.as_ref().map_or(1, |lease| lease.epoch + 1);
        let expires_at_unix_ms = now.saturating_add(lease_ms.max(1));
        task.lease = Some(Lease {
            member_id: owner,
            instance_id: instance_id.clone(),
            epoch,
            attempt_id: attempt_id.clone(),
            expires_at_unix_ms,
        });
        task.attempts = task.attempts.saturating_add(1);
        task.state = TaskState::Leased;
        task.evidence.clear();
        let revision = Self::bump(task, now);
        Ok(self.record(
            &ctx,
            &descriptor,
            task_id,
            Some(attempt_id),
            TaskEventKind::Claimed {
                epoch,
                instance_id,
                expires_at_unix_ms,
            },
            revision,
            None,
            Vec::new(),
        ))
    }

    /// The lease holder reports that work has begun.
    pub(crate) fn start(
        &mut self,
        ctx: CommandContext,
        task_id: &str,
        epoch: u64,
    ) -> Result<TaskEvent, TaskError> {
        self.attempt_transition(
            ctx,
            CommandKind::Start,
            task_id,
            epoch,
            TaskState::Running,
            "start",
            |state| state == TaskState::Leased,
        )
    }

    /// Records a worker's claim that the task is done.
    ///
    /// Evidence is required here, and the state is `Submitted`, never
    /// `Verified`: the worker is a witness, not the judge.
    pub(crate) fn submit(
        &mut self,
        ctx: CommandContext,
        task_id: &str,
        epoch: u64,
        artifact: Option<ArtifactClaim>,
        evidence: Vec<EvidenceRef>,
    ) -> Result<TaskEvent, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::Submit,
            task_id: task_id.to_string(),
            expected_revision: ctx.expected_revision,
            epoch: Some(epoch),
            payload: vec![
                artifact
                    .as_ref()
                    .map(|claim| {
                        format!("{}@{}:{}", claim.artifact_id, claim.version, claim.digest)
                    })
                    .unwrap_or_default(),
                evidence
                    .iter()
                    .map(|item| format!("{}:{}", item.handle, item.digest))
                    .collect::<Vec<_>>()
                    .join(","),
            ],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return existing.into_iter().next().ok_or(TaskError::TaskNotFound);
        }
        if evidence.is_empty() {
            return Err(TaskError::EvidenceRequired);
        }
        if evidence.iter().any(|item| !item.is_valid()) {
            return Err(TaskError::EvidenceInvalid);
        }
        if artifact.as_ref().is_some_and(|claim| {
            claim.artifact_id.trim().is_empty() || claim.digest.trim().is_empty()
        }) {
            return Err(TaskError::EvidenceInvalid);
        }
        self.validate(
            &ctx,
            &descriptor,
            task_id,
            Some(epoch),
            true,
            CommandAuthority::OwnerOrHost,
        )?;
        let now = ctx.now_unix_ms;
        {
            let task = self.require(task_id)?;
            if !matches!(task.state, TaskState::Leased | TaskState::Running) {
                return Err(TaskError::InvalidTransition {
                    from: task.state,
                    to: "submit".into(),
                });
            }
        }
        let artifact_version = artifact.as_ref().map(|claim| claim.version);
        let task = self.require_mut(task_id)?;
        task.state = TaskState::Submitted;
        task.evidence = evidence.clone();
        if let Some(claim) = artifact.as_ref() {
            task.artifact_id = Some(claim.artifact_id.clone());
            task.artifact_version = Some(claim.version);
        }
        let revision = Self::bump(task, now);
        let attempt = task.lease.as_ref().map(|lease| lease.attempt_id.clone());
        if let Some(claim) = artifact.as_ref() {
            self.observe_artifact(&claim.artifact_id, claim.version, Some(&claim.digest), now);
        }
        Ok(self.record(
            &ctx,
            &descriptor,
            task_id,
            attempt,
            TaskEventKind::Submitted {
                artifact: artifact.clone(),
            },
            revision,
            artifact_version,
            evidence,
        ))
    }

    /// Accepts a submission against evidence for the current artifact version.
    ///
    /// The version check is the point: a reviewer who looked at an older build
    /// cannot mark the task verified for a newer one.
    pub(crate) fn verify(
        &mut self,
        ctx: CommandContext,
        task_id: &str,
        verifier: String,
        rule_version: u64,
        acceptance: Vec<EvidenceRef>,
    ) -> Result<TaskEvent, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::Verify,
            task_id: task_id.to_string(),
            expected_revision: ctx.expected_revision,
            epoch: None,
            payload: vec![
                verifier.clone(),
                rule_version.to_string(),
                acceptance
                    .iter()
                    .map(|item| format!("{}:{}", item.handle, item.digest))
                    .collect::<Vec<_>>()
                    .join(","),
            ],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return existing.into_iter().next().ok_or(TaskError::TaskNotFound);
        }
        if verifier.trim().is_empty() {
            return Err(TaskError::VerifierEmpty);
        }
        if acceptance.is_empty() {
            return Err(TaskError::EvidenceRequired);
        }
        if acceptance.iter().any(|item| !item.is_valid()) {
            return Err(TaskError::EvidenceInvalid);
        }
        let now = ctx.now_unix_ms;
        self.validate(
            &ctx,
            &descriptor,
            task_id,
            None,
            false,
            CommandAuthority::ReviewerOrHost,
        )?;
        let current_version = {
            let task = self.require(task_id)?;
            if task.state != TaskState::Submitted {
                return Err(TaskError::InvalidTransition {
                    from: task.state,
                    to: "verify".into(),
                });
            }
            // A worker may not sign off its own claim, whoever else is asleep.
            if ctx.actor.member_id() == Some(task.owner_member_id.as_str()) {
                return Err(TaskError::NotAuthorized);
            }

            if let Some(blocker) = task.blockers.iter().find(|blocker| blocker.is_open()) {
                return Err(TaskError::BlockerOpen {
                    blocker_id: blocker.blocker_id.clone(),
                });
            }
            task.artifact_version
        };
        if acceptance
            .iter()
            .all(|item| item.artifact_version != current_version)
        {
            return Err(TaskError::EvidenceVersionMismatch {
                cited: acceptance.first().and_then(|item| item.artifact_version),
                current: current_version,
            });
        }
        // Version says "which round", the digest says "which bytes". A
        // reviewer who checked a rebuild that kept the version number has not
        // checked what is on disk now.
        let artifact = self
            .task(task_id)
            .and_then(|task| task.artifact_id.clone())
            .and_then(|artifact_id| self.artifact(&artifact_id).cloned());
        if let Some(artifact) = artifact.as_ref() {
            if !artifact.digest.is_empty()
                && !acceptance.iter().any(|item| item.digest == artifact.digest)
            {
                return Err(TaskError::EvidenceDigestMismatch {
                    artifact_id: artifact.artifact_id.clone(),
                });
            }
        }
        let verification = Verification {
            verifier: verifier.clone(),
            rule_version,
            artifact_version: current_version,
            artifact_digest: artifact.as_ref().map(|artifact| artifact.digest.clone()),
            verified_at_unix_ms: now,
        };
        let task = self.require_mut(task_id)?;
        task.state = TaskState::Verified;
        task.evidence.extend(acceptance.iter().cloned());
        task.verification = Some(verification);
        task.lease = None;
        let revision = Self::bump(task, now);
        let event = self.record(
            &ctx,
            &descriptor,
            task_id,
            None,
            TaskEventKind::Verified {
                verifier,
                rule_version,
            },
            revision,
            current_version,
            acceptance,
        );
        Ok(event)
    }

    /// Declares a dependency after creation, rejecting cycles before anything
    /// changes.
    ///
    /// Adding a dependency also withdraws any verdict this task carried: it
    /// was accepted without that requirement, so it says nothing about the
    /// task as it now stands.
    pub(crate) fn add_dependency(
        &mut self,
        ctx: CommandContext,
        task_id: &str,
        dependency_id: &str,
    ) -> Result<Vec<TaskEvent>, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::AddDependency,
            task_id: task_id.to_string(),
            expected_revision: ctx.expected_revision,
            epoch: None,
            payload: vec![dependency_id.to_string()],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return Ok(existing);
        }
        self.validate(
            &ctx,
            &descriptor,
            task_id,
            None,
            false,
            CommandAuthority::OwnerOrHost,
        )?;
        let now = ctx.now_unix_ms;
        {
            let task = self.require(task_id)?;
            let Some(dependency) = self.task(dependency_id) else {
                return Err(TaskError::DependencyMissing {
                    task_id: dependency_id.to_string(),
                });
            };
            if dependency.room_id != task.room_id {
                return Err(TaskError::CrossRoomDependency {
                    task_id: dependency_id.to_string(),
                });
            }
            if task.depends_on.iter().any(|id| id == dependency_id) {
                return Ok(Vec::new());
            }
            // Refuse before mutating: a cycle must never exist even briefly.
            if dependency_id == task_id || self.reaches(dependency_id, task_id) {
                return Err(TaskError::DependencyCycle {
                    task_id: dependency_id.to_string(),
                });
            }
        }
        let task = self.require_mut(task_id)?;
        task.depends_on.push(dependency_id.to_string());
        let revision = Self::bump(task, now);
        let mut events = vec![self.record(
            &ctx,
            &descriptor,
            task_id,
            None,
            TaskEventKind::DependencyAdded {
                dependency_id: dependency_id.to_string(),
            },
            revision,
            None,
            Vec::new(),
        )];
        // The task's own verdict goes first: it was accepted without this
        // requirement, so it says nothing about the task as it now stands. A
        // task left Verified while resting on unverified work is exactly the
        // stale green this layer exists to prevent.
        let withdraw_self = self
            .task(task_id)
            .is_some_and(|task| task.state == TaskState::Verified);
        if withdraw_self {
            let task = self.require_mut(task_id)?;
            task.state = TaskState::Ready;
            task.verification = None;
            task.evidence.clear();
            task.lease = None;
            let revision = Self::bump(task, now);
            let scoped = CommandContext {
                caused_by: Some(events[0].event_id.clone()),
                ..ctx.clone()
            };
            events.push(self.record(
                &scoped,
                &descriptor,
                task_id,
                None,
                TaskEventKind::DependencyInvalidated {
                    dependency_id: dependency_id.to_string(),
                },
                revision,
                None,
                Vec::new(),
            ));
        }
        events.extend(self.withdraw_verdicts_from(task_id, &ctx, &descriptor, now, 1));
        Ok(events)
    }

    /// Whether `from` can reach `to` through declared dependencies.
    fn reaches(&self, from: &str, to: &str) -> bool {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut stack = vec![from];
        while let Some(current) = stack.pop() {
            if current == to {
                return true;
            }
            if !seen.insert(current) {
                continue;
            }
            if let Some(task) = self.task(current) {
                stack.extend(task.depends_on.iter().map(String::as_str));
            }
        }
        false
    }

    /// Withdraws the verdicts of everything that depended on `task_id`.
    ///
    /// A verdict is a statement about a task *and the things it rests on*. If
    /// one of those stops being verified, the dependent's verdict is stale,
    /// and leaving it green is how a board starts lying.
    #[allow(clippy::too_many_arguments)]
    fn withdraw_verdicts_from(
        &mut self,
        task_id: &str,
        ctx: &CommandContext,
        descriptor: &CommandDescriptor,
        now: u64,
        key_offset: usize,
    ) -> Vec<TaskEvent> {
        let mut events = Vec::new();
        let mut queue = vec![task_id.to_string()];
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(current) = queue.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            let dependents: Vec<String> = self
                .tasks
                .iter()
                .filter(|task| task.depends_on.iter().any(|id| id == &current))
                .filter(|task| task.state == TaskState::Verified)
                .map(|task| task.task_id.clone())
                .collect();
            for dependent in dependents {
                let Ok(task) = self.require_mut(&dependent) else {
                    continue;
                };
                task.state = TaskState::Ready;
                task.verification = None;
                task.evidence.clear();
                task.lease = None;
                let revision = Self::bump(task, now);
                let scoped = CommandContext {
                    caused_by: ctx.caused_by.clone(),
                    ..ctx.clone()
                };
                let _ = key_offset;
                events.push(self.record(
                    &scoped,
                    descriptor,
                    &dependent,
                    None,
                    TaskEventKind::DependencyInvalidated {
                        dependency_id: current.clone(),
                    },
                    revision,
                    None,
                    Vec::new(),
                ));
                queue.push(dependent);
            }
        }
        events
    }

    /// Rejects an attempt. The task returns to the pool for a new one.
    ///
    /// `epoch` names the attempt being failed. A member must supply it
    /// whenever a lease is live, so a replaced worker cannot end its
    /// successor's attempt; the host may act without one when it is clearing
    /// work administratively.
    pub(crate) fn fail(
        &mut self,
        ctx: CommandContext,
        task_id: &str,
        epoch: Option<u64>,
        reason: String,
    ) -> Result<TaskEvent, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::Fail,
            task_id: task_id.to_string(),
            expected_revision: ctx.expected_revision,
            epoch,
            payload: vec![reason.clone()],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return existing.into_iter().next().ok_or(TaskError::TaskNotFound);
        }
        let epoch = self.required_attempt_epoch(&ctx, task_id, epoch)?;
        self.validate(
            &ctx,
            &descriptor,
            task_id,
            epoch,
            false,
            CommandAuthority::OwnerOrHost,
        )?;
        let now = ctx.now_unix_ms;
        {
            let task = self.require(task_id)?;
            if task.state.is_terminal() {
                return Err(TaskError::InvalidTransition {
                    from: task.state,
                    to: "fail".into(),
                });
            }
        }
        let task = self.require_mut(task_id)?;
        task.state = TaskState::Failed;
        task.lease = None;
        let revision = Self::bump(task, now);
        Ok(self.record(
            &ctx,
            &descriptor,
            task_id,
            None,
            TaskEventKind::Failed { reason },
            revision,
            None,
            Vec::new(),
        ))
    }

    /// Raises a blocker. The task stops being ready until it is cleared.
    ///
    /// Blocking releases the lease. A blocked task is not being worked, and
    /// leaving ownership with an attempt that has stopped would let a dead
    /// worker keep the task to itself; the invariant that a lease only exists
    /// while work is out is now true rather than merely asserted.
    pub(crate) fn block(
        &mut self,
        ctx: CommandContext,
        task_id: &str,
        epoch: Option<u64>,
        reason: String,
    ) -> Result<TaskEvent, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::Block,
            task_id: task_id.to_string(),
            expected_revision: ctx.expected_revision,
            epoch,
            payload: vec![reason.clone()],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return existing.into_iter().next().ok_or(TaskError::TaskNotFound);
        }
        let epoch = self.required_attempt_epoch(&ctx, task_id, epoch)?;
        self.validate(
            &ctx,
            &descriptor,
            task_id,
            epoch,
            false,
            CommandAuthority::OwnerOrHost,
        )?;
        let now = ctx.now_unix_ms;
        {
            let task = self.require(task_id)?;
            if task.state.is_terminal() {
                return Err(TaskError::InvalidTransition {
                    from: task.state,
                    to: "block".into(),
                });
            }
        }
        self.next_blocker_id = self.next_blocker_id.saturating_add(1);
        let blocker_id = format!("blocker_{:x}", self.next_blocker_id);
        let task = self.require_mut(task_id)?;
        task.blockers.push(Blocker {
            blocker_id: blocker_id.clone(),
            reason: reason.clone(),
            raised_at_unix_ms: now,
            cleared_at_unix_ms: None,
        });
        task.state = TaskState::Blocked;
        task.lease = None;
        let revision = Self::bump(task, now);
        Ok(self.record(
            &ctx,
            &descriptor,
            task_id,
            None,
            TaskEventKind::Blocked { blocker_id, reason },
            revision,
            None,
            Vec::new(),
        ))
    }

    /// The epoch an administrative command must carry.
    ///
    /// While an attempt is live, a member has to name it; the host does not,
    /// because clearing stuck work is exactly what host authority is for.
    fn required_attempt_epoch(
        &self,
        ctx: &CommandContext,
        task_id: &str,
        epoch: Option<u64>,
    ) -> Result<Option<u64>, TaskError> {
        if epoch.is_some() {
            return Ok(epoch);
        }
        let live = self
            .task(task_id)
            .and_then(|task| task.lease.as_ref())
            .map(|lease| lease.epoch);
        match live {
            Some(_) if !ctx.actor.is_host() => Err(TaskError::NotLeaseHolder),
            _ => Ok(None),
        }
    }

    /// Clears one blocker. The task becomes ready only when none are left.
    pub(crate) fn unblock(
        &mut self,
        ctx: CommandContext,
        task_id: &str,
        blocker_id: &str,
    ) -> Result<TaskEvent, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::Unblock,
            task_id: task_id.to_string(),
            expected_revision: ctx.expected_revision,
            epoch: None,
            payload: vec![blocker_id.to_string()],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return existing.into_iter().next().ok_or(TaskError::TaskNotFound);
        }
        self.validate(
            &ctx,
            &descriptor,
            task_id,
            None,
            false,
            CommandAuthority::OwnerOrHost,
        )?;
        let now = ctx.now_unix_ms;
        let task = self.require_mut(task_id)?;
        let Some(blocker) = task
            .blockers
            .iter_mut()
            .find(|blocker| blocker.blocker_id == blocker_id && blocker.is_open())
        else {
            return Err(TaskError::BlockerNotFound);
        };
        blocker.cleared_at_unix_ms = Some(now);
        if !task.has_open_blocker() && task.state == TaskState::Blocked {
            task.state = TaskState::Ready;
        }
        let revision = Self::bump(task, now);
        Ok(self.record(
            &ctx,
            &descriptor,
            task_id,
            None,
            TaskEventKind::Unblocked {
                blocker_id: blocker_id.to_string(),
            },
            revision,
            None,
            Vec::new(),
        ))
    }

    /// Advances an artifact to new content, withdrawing the verdicts that
    /// described the old content.
    ///
    /// The advance itself is always recorded, even when no task is affected:
    /// the registry moved, and a journal that omitted it would rebuild a
    /// ledger pointing at content that no longer exists. Invalidations are
    /// recorded causally beneath that event, so the fan-out is one command
    /// with a visible parent rather than a scatter of orphans.
    pub(crate) fn advance_artifact(
        &mut self,
        ctx: CommandContext,
        artifact_id: &str,
        digest: String,
    ) -> Result<Vec<TaskEvent>, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::AdvanceArtifact,
            task_id: String::new(),
            expected_revision: ctx.expected_revision,
            epoch: None,
            payload: vec![artifact_id.to_string(), digest.clone()],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return Ok(existing);
        }
        if !ctx.actor.is_host() && ctx.actor.member_id().is_none() {
            return Err(TaskError::NotAuthorized);
        }
        if artifact_id.trim().is_empty() || digest.trim().is_empty() {
            return Err(TaskError::EvidenceInvalid);
        }
        let now = ctx.now_unix_ms;
        // Re-reporting content the registry already holds changes nothing, so
        // there is nothing to record. Without this, a fresh key carrying the
        // same digest would advance the version for no reason.
        if self
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_id == artifact_id && artifact.digest == digest)
        {
            return Ok(Vec::new());
        }
        let version = match self
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.artifact_id == artifact_id)
        {
            Some(artifact) => {
                artifact.version = artifact.version.saturating_add(1);
                artifact.digest.clone_from(&digest);
                artifact.updated_at_unix_ms = now;
                artifact.version
            }
            None => {
                self.artifacts.push(Artifact {
                    artifact_id: artifact_id.to_string(),
                    version: 1,
                    digest: digest.clone(),
                    produced_by: None,
                    updated_at_unix_ms: now,
                });
                1
            }
        };

        // The command-level event carries the command's own key, so the
        // journal records that this command ran even when it touched no task.
        let mut events = vec![self.record(
            &ctx,
            &descriptor,
            "",
            None,
            TaskEventKind::ArtifactAdvanced {
                artifact_id: artifact_id.to_string(),
                version,
                digest: digest.clone(),
            },
            0,
            Some(version),
            Vec::new(),
        )];
        let parent = events[0].event_id.clone();

        let affected: Vec<String> = self
            .tasks
            .iter()
            .filter(|task| {
                task.artifact_id.as_deref() == Some(artifact_id)
                    && matches!(task.state, TaskState::Verified | TaskState::Submitted)
            })
            .map(|task| task.task_id.clone())
            .collect();

        for task_id in &affected {
            let task = self.require_mut(task_id)?;
            task.state = TaskState::Ready;
            task.artifact_version = Some(version);
            task.lease = None;
            // Evidence for the superseded version leaves the task, but the
            // events that recorded it stay in the journal.
            task.evidence.clear();
            task.verification = None;
            let revision = Self::bump(task, now);
            // Children share the command's key: one command is one
            // transaction, so a retry replays the whole ordered batch rather
            // than whichever event happened to be recorded first.
            let scoped = CommandContext {
                caused_by: Some(parent.clone()),
                ..ctx.clone()
            };
            events.push(self.record(
                &scoped,
                &descriptor,
                task_id,
                None,
                TaskEventKind::ArtifactInvalidated {
                    artifact_id: artifact_id.to_string(),
                    version,
                    digest: digest.clone(),
                },
                revision,
                Some(version),
                Vec::new(),
            ));
        }
        // Anything verified *because* these were verified is stale too, so the
        // withdrawal follows the dependency edges outward.
        let caused = CommandContext {
            caused_by: Some(parent),
            ..ctx.clone()
        };
        for task_id in affected {
            let offset = events.len();
            let cascaded = self.withdraw_verdicts_from(&task_id, &caused, &descriptor, now, offset);
            events.extend(cascaded);
        }
        Ok(events)
    }

    pub(crate) fn decision(&self, decision_id: &str) -> Option<&DecisionRecord> {
        self.decisions
            .iter()
            .find(|decision| decision.decision_id == decision_id)
    }

    pub(crate) fn decisions(&self) -> &[DecisionRecord] {
        &self.decisions
    }

    /// Puts a decision to the room.
    ///
    /// It starts `Proposed` whoever writes it, including the host: recording
    /// what was decided and ratifying it are different acts, and collapsing
    /// them would make "we decided" unfalsifiable.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn propose_decision(
        &mut self,
        ctx: CommandContext,
        room_id: String,
        title: String,
        statement: String,
        rationale: String,
        supersedes: Option<String>,
        evidence: Vec<EvidenceRef>,
    ) -> Result<DecisionEvent, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::ProposeDecision,
            task_id: String::new(),
            expected_revision: None,
            epoch: None,
            payload: vec![
                room_id.clone(),
                title.clone(),
                statement.clone(),
                rationale.clone(),
                supersedes.clone().unwrap_or_default(),
            ],
        };
        if let Some(existing) = self.replayed_decision(&ctx, &descriptor)? {
            return existing
                .into_iter()
                .next()
                .ok_or(TaskError::DecisionNotFound);
        }
        if statement.trim().is_empty() {
            return Err(TaskError::StatementEmpty);
        }
        if title.trim().is_empty() {
            return Err(TaskError::TitleEmpty);
        }
        if evidence.iter().any(|item| !item.is_valid()) {
            return Err(TaskError::EvidenceInvalid);
        }
        if let Some(previous) = supersedes.as_deref() {
            let Some(previous) = self.decision(previous) else {
                return Err(TaskError::DecisionNotFound);
            };
            if previous.status == DecisionStatus::Superseded {
                return Err(TaskError::DecisionSettled);
            }
        }
        self.next_decision_id = self.next_decision_id.saturating_add(1);
        let decision_id = format!("dec_{:x}", self.next_decision_id);
        self.decisions.push(DecisionRecord {
            decision_id: decision_id.clone(),
            room_id: room_id.clone(),
            title: title.clone(),
            statement: statement.clone(),
            rationale: rationale.clone(),
            status: DecisionStatus::Proposed,
            proposed_by: ctx.actor.clone(),
            accepted_by: None,
            supersedes: supersedes.clone(),
            superseded_by: None,
            evidence: evidence.clone(),
            revision: 1,
            created_at_unix_ms: ctx.now_unix_ms,
            updated_at_unix_ms: ctx.now_unix_ms,
        });
        Ok(self.record_decision(
            &ctx,
            &descriptor,
            &decision_id,
            None,
            DecisionEventKind::Proposed {
                room_id,
                title,
                statement,
                rationale,
                supersedes,
            },
            1,
            evidence,
        ))
    }

    /// Ratifies a decision, retiring the one it replaces in the same
    /// transaction.
    ///
    /// Both records are checked and moved together. Two proposals may name
    /// the same predecessor, and whichever is ratified first retires it; the
    /// second then finds a predecessor that is no longer `Accepted` and is
    /// refused, rather than silently overwriting who replaced what.
    pub(crate) fn accept_decision(
        &mut self,
        ctx: CommandContext,
        decision_id: &str,
    ) -> Result<Vec<DecisionEvent>, TaskError> {
        let descriptor = CommandDescriptor {
            kind: CommandKind::AcceptDecision,
            task_id: decision_id.to_string(),
            expected_revision: ctx.expected_revision,
            epoch: None,
            payload: Vec::new(),
        };
        if let Some(existing) = self.replayed_decision(&ctx, &descriptor)? {
            return Ok(existing);
        }
        let Some(expected) = ctx.expected_revision else {
            return Err(TaskError::RevisionRequired);
        };
        let Some(decision) = self.decision(decision_id).cloned() else {
            return Err(TaskError::DecisionNotFound);
        };
        if expected != decision.revision {
            return Err(TaskError::RevisionConflict {
                expected,
                actual: decision.revision,
            });
        }
        if decision.status != DecisionStatus::Proposed {
            return Err(TaskError::DecisionSettled);
        }
        // Ratifying your own proposal is not review.
        if ctx.actor == decision.proposed_by {
            return Err(TaskError::NotAuthorized);
        }
        // The predecessor is validated before anything moves, so a losing
        // replacement changes nothing at all.
        let predecessor = match decision.supersedes.as_deref() {
            Some(previous_id) => {
                let Some(previous) = self.decision(previous_id).cloned() else {
                    return Err(TaskError::DecisionNotFound);
                };
                if previous.room_id != decision.room_id {
                    return Err(TaskError::CrossRoomDependency {
                        task_id: previous_id.to_string(),
                    });
                }
                if previous.status != DecisionStatus::Accepted {
                    // Already superseded by a competing replacement, or never
                    // ratified in the first place.
                    return Err(TaskError::DecisionSettled);
                }
                Some(previous)
            }
            None => None,
        };

        let now = ctx.now_unix_ms;
        let actor = ctx.actor.clone();
        let revision = {
            let decision = self
                .decisions
                .iter_mut()
                .find(|decision| decision.decision_id == decision_id)
                .ok_or(TaskError::DecisionNotFound)?;
            decision.status = DecisionStatus::Accepted;
            decision.accepted_by = Some(actor);
            decision.revision = decision.revision.saturating_add(1);
            decision.updated_at_unix_ms = now;
            decision.revision
        };
        let accepted = self.record_decision(
            &ctx,
            &descriptor,
            decision_id,
            None,
            DecisionEventKind::Accepted,
            revision,
            Vec::new(),
        );
        let mut events = vec![accepted.clone()];

        if let Some(previous) = predecessor {
            let previous_revision = {
                let record = self
                    .decisions
                    .iter_mut()
                    .find(|decision| decision.decision_id == previous.decision_id)
                    .ok_or(TaskError::DecisionNotFound)?;
                // Compare-and-swap on the predecessor too: if anything moved
                // it since validation, this transaction does not proceed.
                if record.revision != previous.revision || record.status != DecisionStatus::Accepted
                {
                    return Err(TaskError::DecisionSettled);
                }
                record.status = DecisionStatus::Superseded;
                record.superseded_by = Some(decision_id.to_string());
                record.revision = record.revision.saturating_add(1);
                record.updated_at_unix_ms = now;
                record.revision
            };
            let scoped = CommandContext {
                caused_by: Some(accepted.event_id.clone()),
                ..ctx.clone()
            };
            events.push(self.record_decision(
                &scoped,
                &descriptor,
                &previous.decision_id,
                Some(accepted.event_id.clone()),
                DecisionEventKind::Superseded {
                    by: decision_id.to_string(),
                },
                previous_revision,
                Vec::new(),
            ));
        }
        Ok(events)
    }

    fn replayed_decision(
        &self,
        ctx: &CommandContext,
        descriptor: &CommandDescriptor,
    ) -> Result<Option<Vec<DecisionEvent>>, TaskError> {
        if ctx.idempotency_key.trim().is_empty() {
            return Err(TaskError::IdempotencyKeyEmpty);
        }
        let Some(applied) = self.applied.get(&ctx.idempotency_key) else {
            return Ok(None);
        };
        if applied.fingerprint != descriptor.fingerprint() {
            return Err(TaskError::IdempotencyConflict {
                key: ctx.idempotency_key.clone(),
            });
        }
        match &applied.outcome {
            CommandOutcome::Accepted { event_ids } => Ok(Some(
                self.journal
                    .iter()
                    .filter_map(JournalEntry::as_decision)
                    .filter(|event| event_ids.contains(&event.event_id))
                    .cloned()
                    .collect(),
            )),
            CommandOutcome::Rejected { receipt_id } => match self.receipt(receipt_id) {
                Some(receipt) => Err(receipt.cause.clone()),
                None => Err(TaskError::DecisionNotFound),
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_decision(
        &mut self,
        ctx: &CommandContext,
        descriptor: &CommandDescriptor,
        decision_id: &str,
        caused_by: Option<String>,
        kind: DecisionEventKind,
        revision: u64,
        evidence: Vec<EvidenceRef>,
    ) -> DecisionEvent {
        self.next_event_id = self.next_event_id.saturating_add(1);
        let event = DecisionEvent {
            protocol_version: TASK_PROTOCOL_VERSION,
            event_id: format!("dev_{:x}", self.next_event_id),
            sequence: self.next_event_id,
            decision_id: decision_id.to_string(),
            caused_by,
            actor: ctx.actor.clone(),
            kind,
            idempotency_key: ctx.idempotency_key.clone(),
            fingerprint: descriptor.fingerprint(),
            revision,
            evidence,
            created_at_unix_ms: ctx.now_unix_ms,
        };
        let entry = self
            .applied
            .entry(ctx.idempotency_key.clone())
            .or_insert_with(|| AppliedCommand {
                fingerprint: descriptor.fingerprint(),
                outcome: CommandOutcome::Accepted {
                    event_ids: Vec::new(),
                },
            });
        if let CommandOutcome::Accepted { event_ids } = &mut entry.outcome {
            event_ids.push(event.event_id.clone());
        }
        self.journal.push(JournalEntry::Decision(event.clone()));
        event
    }

    /// Folds a recorded decision event into the decision projection.
    fn apply_decision(&mut self, event: &DecisionEvent) -> Result<(), TaskError> {
        self.next_event_id = self.next_event_id.max(event.sequence);
        let applied = self
            .applied
            .entry(event.idempotency_key.clone())
            .or_insert_with(|| AppliedCommand {
                fingerprint: event.fingerprint.clone(),
                outcome: CommandOutcome::Accepted {
                    event_ids: Vec::new(),
                },
            });
        if let CommandOutcome::Accepted { event_ids } = &mut applied.outcome {
            event_ids.push(event.event_id.clone());
        }
        match &event.kind {
            DecisionEventKind::Proposed {
                room_id,
                title,
                statement,
                rationale,
                supersedes,
            } => {
                if let Some(parsed) = event
                    .decision_id
                    .strip_prefix("dec_")
                    .and_then(|suffix| u64::from_str_radix(suffix, 16).ok())
                {
                    self.next_decision_id = self.next_decision_id.max(parsed);
                }
                self.decisions.push(DecisionRecord {
                    decision_id: event.decision_id.clone(),
                    room_id: room_id.clone(),
                    title: title.clone(),
                    statement: statement.clone(),
                    rationale: rationale.clone(),
                    status: DecisionStatus::Proposed,
                    proposed_by: event.actor.clone(),
                    accepted_by: None,
                    supersedes: supersedes.clone(),
                    superseded_by: None,
                    evidence: event.evidence.clone(),
                    revision: event.revision,
                    created_at_unix_ms: event.created_at_unix_ms,
                    updated_at_unix_ms: event.created_at_unix_ms,
                });
            }
            other => {
                let Some(decision) = self
                    .decisions
                    .iter_mut()
                    .find(|decision| decision.decision_id == event.decision_id)
                else {
                    return Err(TaskError::DecisionNotFound);
                };
                match other {
                    DecisionEventKind::Proposed { .. } => {
                        unreachable!("handled above")
                    }
                    DecisionEventKind::Accepted => {
                        decision.status = DecisionStatus::Accepted;
                        decision.accepted_by = Some(event.actor.clone());
                    }
                    DecisionEventKind::Rejected { .. } => {
                        decision.status = DecisionStatus::Rejected;
                    }
                    DecisionEventKind::Superseded { by } => {
                        decision.status = DecisionStatus::Superseded;
                        decision.superseded_by = Some(by.clone());
                    }
                }
                decision.revision = event.revision;
                decision.updated_at_unix_ms = event.created_at_unix_ms;
            }
        }
        self.journal.push(JournalEntry::Decision(event.clone()));
        Ok(())
    }

    /// Releases every lease, because a restart ends every process that held
    /// one.
    ///
    /// Tasks, evidence, verdicts and history all survive: they describe work,
    /// not processes. A lease is the one piece of state naming a live
    /// incarnation, so keeping it would leave tasks owned by workers that no
    /// longer exist — the same failure that once left room mail addressed to
    /// dead incarnations after a restart. Each release is recorded, so a
    /// replay of the log still reproduces the ledger.
    pub(crate) fn release_leases_for_cold_restore(&mut self, now_unix_ms: u64) -> Vec<TaskEvent> {
        let held: Vec<(String, u64)> = self
            .tasks
            .iter()
            .filter_map(|task| {
                task.lease
                    .as_ref()
                    .map(|lease| (task.task_id.clone(), lease.epoch))
            })
            .collect();
        let mut events = Vec::new();
        for (task_id, epoch) in held {
            let Ok(task) = self.require_mut(&task_id) else {
                continue;
            };
            task.lease = None;
            // A submitted claim still stands: it is waiting on a reviewer,
            // not on the worker that made it.
            if matches!(task.state, TaskState::Leased | TaskState::Running) {
                task.state = TaskState::Ready;
            }
            let revision = Self::bump(task, now_unix_ms);
            let ctx = CommandContext {
                actor: TaskActor::Host,
                // Deterministic and unique per lease, so restoring twice
                // cannot record the same release twice.
                idempotency_key: format!("cold-restore:{task_id}:{epoch}"),
                expected_revision: None,
                caused_by: None,
                now_unix_ms,
            };
            let descriptor = CommandDescriptor {
                kind: CommandKind::Fail,
                task_id: task_id.clone(),
                expected_revision: None,
                epoch: Some(epoch),
                payload: vec!["cold-restore".into()],
            };
            events.push(self.record(
                &ctx,
                &descriptor,
                &task_id,
                None,
                TaskEventKind::LeaseReleased { epoch },
                revision,
                None,
                Vec::new(),
            ));
        }
        events
    }

    /// Rebuilds a ledger from its log alone.
    ///
    /// This is the definition of "history is the state": if a replay ever
    /// disagrees with the live ledger, one of the two is wrong, and the test
    /// harness below says which.
    pub(crate) fn replay(journal: &[JournalEntry]) -> Result<Self, TaskError> {
        let mut ledger = Self::default();
        for entry in journal {
            match entry {
                JournalEntry::Event(event) => ledger.apply_recorded(event)?,
                // A refusal moves the receipt and idempotency projections and
                // nothing else. Touching the task projection here is exactly
                // the bug the receipt exists to avoid.
                JournalEntry::Decision(event) => ledger.apply_decision(event)?,
                JournalEntry::Rejection(receipt) => ledger.apply_rejection(receipt),
            }
        }
        Ok(ledger)
    }

    /// Folds a recorded refusal into the receipt and idempotency projections.
    fn apply_rejection(&mut self, receipt: &RejectionReceipt) {
        self.next_event_id = self.next_event_id.max(receipt.sequence);
        if let Some(parsed) = receipt
            .receipt_id
            .strip_prefix("trj_")
            .and_then(|suffix| u64::from_str_radix(suffix, 16).ok())
        {
            self.next_receipt_id = self.next_receipt_id.max(parsed);
        }
        self.applied.insert(
            receipt.idempotency_key.clone(),
            AppliedCommand {
                fingerprint: receipt.fingerprint.clone(),
                outcome: CommandOutcome::Rejected {
                    receipt_id: receipt.receipt_id.clone(),
                },
            },
        );
        self.journal.push(JournalEntry::Rejection(receipt.clone()));
    }

    /// Highest sequence this ledger has absorbed.
    pub(crate) fn last_sequence(&self) -> u64 {
        self.journal.last().map(JournalEntry::sequence).unwrap_or(0)
    }

    /// Folds one already-durable entry into this ledger.
    ///
    /// Recovery uses this to catch a restored snapshot up to the journal: the
    /// snapshot is a cache of the projection, and the journal is the record,
    /// so entries committed after the last save are replayed rather than lost.
    pub(crate) fn absorb(&mut self, entry: &JournalEntry) -> Result<(), TaskError> {
        match entry {
            JournalEntry::Event(event) => self.apply_recorded(event),
            JournalEntry::Decision(event) => self.apply_decision(event),
            JournalEntry::Rejection(receipt) => {
                self.apply_rejection(receipt);
                Ok(())
            }
        }
    }

    /// Structural checks that must hold after any sequence of commands.
    pub(crate) fn assert_invariants(&self) -> Result<(), TaskError> {
        for task in &self.tasks {
            if task.owner_member_id.trim().is_empty() {
                return Err(TaskError::OwnerEmpty);
            }
            // A lease only makes sense while the work is out.
            if task.lease.is_some()
                && !matches!(
                    task.state,
                    TaskState::Leased | TaskState::Running | TaskState::Submitted
                )
            {
                return Err(TaskError::InvalidTransition {
                    from: task.state,
                    to: "hold a lease".into(),
                });
            }
            if task.state == TaskState::Verified && task.evidence.is_empty() {
                return Err(TaskError::EvidenceRequired);
            }
            if task.state == TaskState::Verified && task.verification.is_none() {
                return Err(TaskError::VerifierEmpty);
            }
            if task.state == TaskState::Ready && task.has_open_blocker() {
                return Err(TaskError::BlockerOpen {
                    blocker_id: task
                        .open_blockers()
                        .next()
                        .map(|blocker| blocker.blocker_id.clone())
                        .unwrap_or_default(),
                });
            }
            self.check_no_cycle(&task.task_id, &mut HashSet::new())?;
        }
        Ok(())
    }

    fn check_no_cycle(&self, task_id: &str, seen: &mut HashSet<String>) -> Result<(), TaskError> {
        if !seen.insert(task_id.to_string()) {
            return Err(TaskError::DependencyCycle {
                task_id: task_id.to_string(),
            });
        }
        if let Some(task) = self.task(task_id) {
            for dependency in &task.depends_on {
                self.check_no_cycle(dependency, seen)?;
            }
        }
        seen.remove(task_id);
        Ok(())
    }

    // ---- internals -------------------------------------------------------

    fn require(&self, task_id: &str) -> Result<&Task, TaskError> {
        self.task(task_id).ok_or(TaskError::TaskNotFound)
    }

    fn require_mut(&mut self, task_id: &str) -> Result<&mut Task, TaskError> {
        self.tasks
            .iter_mut()
            .find(|task| task.task_id == task_id)
            .ok_or(TaskError::TaskNotFound)
    }

    fn check_revision(&self, task: &Task, ctx: &CommandContext) -> Result<(), TaskError> {
        match ctx.expected_revision {
            Some(expected) if expected != task.revision => Err(TaskError::RevisionConflict {
                expected,
                actual: task.revision,
            }),
            _ => Ok(()),
        }
    }

    fn check_epoch(task: &Task, ctx: &CommandContext, epoch: u64) -> Result<(), TaskError> {
        let Some(lease) = task.lease.as_ref() else {
            return Err(TaskError::NotLeaseHolder);
        };
        if lease.epoch != epoch {
            return Err(TaskError::StaleEpoch {
                expected: lease.epoch,
                actual: epoch,
            });
        }
        // The host may report on behalf of a worker it is driving.
        if !ctx.actor.is_host() && ctx.actor.member_id() != Some(lease.member_id.as_str()) {
            return Err(TaskError::NotLeaseHolder);
        }
        Ok(())
    }

    /// Records the artifact version a worker says it built against.
    ///
    /// The registry is the authority on "latest known version", so a
    /// submission that observed a newer one advances it. Without this, a
    /// later invalidation would bump from a version nobody ever saw.
    fn observe_artifact(
        &mut self,
        artifact_id: &str,
        version: u64,
        digest: Option<&str>,
        now: u64,
    ) {
        match self
            .artifacts
            .iter_mut()
            .find(|artifact| artifact.artifact_id == artifact_id)
        {
            Some(artifact) => {
                if version > artifact.version {
                    artifact.version = version;
                    if let Some(digest) = digest {
                        artifact.digest = digest.to_string();
                    }
                    artifact.updated_at_unix_ms = now;
                }
            }
            None => self.artifacts.push(Artifact {
                artifact_id: artifact_id.to_string(),
                version,
                digest: digest.unwrap_or_default().to_string(),
                produced_by: None,
                updated_at_unix_ms: now,
            }),
        }
    }

    fn bump(task: &mut Task, now: u64) -> u64 {
        task.revision = task.revision.saturating_add(1);
        task.updated_at_unix_ms = now;
        task.revision
    }

    /// Answers a command that carries a key already seen.
    ///
    /// Three cases, in order: an unused key proceeds; the same key with the
    /// same meaning replays its original outcome, whether that was acceptance
    /// or refusal; the same key with different meaning is a conflict, because
    /// silently treating it as a retry would drop real work.
    fn replayed(
        &self,
        ctx: &CommandContext,
        descriptor: &CommandDescriptor,
    ) -> Result<Option<Vec<TaskEvent>>, TaskError> {
        if ctx.idempotency_key.trim().is_empty() {
            return Err(TaskError::IdempotencyKeyEmpty);
        }
        let Some(applied) = self.applied.get(&ctx.idempotency_key) else {
            return Ok(None);
        };
        if applied.fingerprint != descriptor.fingerprint() {
            return Err(TaskError::IdempotencyConflict {
                key: ctx.idempotency_key.clone(),
            });
        }
        match &applied.outcome {
            // Journal order, not the order ids were recorded in, so the batch
            // a caller sees is the same on the first call and on every retry.
            CommandOutcome::Accepted { event_ids } => Ok(Some(
                self.journal
                    .iter()
                    .filter_map(JournalEntry::as_event)
                    .filter(|event| event_ids.contains(&event.event_id))
                    .cloned()
                    .collect(),
            )),
            // A refused command stays refused, with the same answer, however
            // often it is resent and whether or not a restart intervened.
            CommandOutcome::Rejected { receipt_id } => match self.receipt(receipt_id) {
                Some(receipt) => Err(receipt.cause.clone()),
                None => Err(TaskError::TaskNotFound),
            },
        }
    }

    /// Records that a command was refused, without touching the task.
    ///
    /// Appending happens before the caller is told, so a refusal that reaches
    /// a worker is always already in the journal. The reverse order would let
    /// a worker act on a refusal the ledger never recorded.
    fn reject(
        &mut self,
        ctx: &CommandContext,
        descriptor: &CommandDescriptor,
        task_id: &str,
        submitted_attempt: Option<String>,
        cause: TaskError,
    ) -> TaskError {
        let task = self.task(task_id);
        let observed_revision = task.map_or(0, |task| task.revision);
        let observed_epoch = task
            .and_then(|task| task.lease.as_ref())
            .map(|lease| lease.epoch);
        self.next_event_id = self.next_event_id.saturating_add(1);
        self.next_receipt_id = self.next_receipt_id.saturating_add(1);
        let receipt = RejectionReceipt {
            protocol_version: TASK_PROTOCOL_VERSION,
            receipt_id: format!("trj_{:x}", self.next_receipt_id),
            sequence: self.next_event_id,
            command: descriptor.kind,
            fingerprint: descriptor.fingerprint(),
            idempotency_key: ctx.idempotency_key.clone(),
            actor: ctx.actor.clone(),
            task_id: task_id.to_string(),
            submitted_attempt,
            submitted_epoch: descriptor.epoch,
            observed_revision,
            observed_epoch,
            error_code: cause.code().to_string(),
            cause: cause.clone(),
            created_at_unix_ms: ctx.now_unix_ms,
        };
        self.applied.insert(
            ctx.idempotency_key.clone(),
            AppliedCommand {
                fingerprint: descriptor.fingerprint(),
                outcome: CommandOutcome::Rejected {
                    receipt_id: receipt.receipt_id.clone(),
                },
            },
        );
        self.journal.push(JournalEntry::Rejection(receipt));
        cause
    }

    /// The fixed validation order for a command against a task.
    ///
    /// Order matters and is part of the contract: a caller that is both stale
    /// on revision and stale on epoch is always told about the revision, so
    /// the answer to a given situation never depends on internal ordering.
    /// Only the last two steps are "stale attempt" refusals that earn a
    /// durable receipt; a malformed, unknown or unauthorized command is
    /// refused without writing anything, so unauthenticated traffic can never
    /// append to the journal.
    fn validate(
        &mut self,
        ctx: &CommandContext,
        descriptor: &CommandDescriptor,
        task_id: &str,
        epoch: Option<u64>,
        require_lease_holder: bool,
        authority: CommandAuthority,
    ) -> Result<(), TaskError> {
        // 1. The command must name the revision it believes it is changing.
        if ctx.expected_revision.is_none() && !descriptor.kind.may_omit_revision() {
            return Err(TaskError::RevisionRequired);
        }
        // 2. The task must exist.
        let Some(task) = self.task(task_id).cloned() else {
            return Err(TaskError::TaskNotFound);
        };
        // 3. Authorization, before anything durable is written. Who may act
        //    depends on the command: work is done by its owner, but accepting
        //    work is precisely not the owner's to do.
        let authorized = match authority {
            CommandAuthority::OwnerOrHost => {
                ctx.actor.is_host() || ctx.actor.member_id() == Some(task.owner_member_id.as_str())
            }
            CommandAuthority::ReviewerOrHost => {
                ctx.actor.is_host()
                    || (ctx.actor.member_id().is_some()
                        && ctx.actor.member_id() != Some(task.owner_member_id.as_str()))
            }
        };
        if !authorized {
            return Err(TaskError::NotAuthorized);
        }
        if require_lease_holder {
            let Some(lease) = task.lease.as_ref() else {
                return Err(TaskError::NotLeaseHolder);
            };
            if !ctx.actor.is_host() && ctx.actor.member_id() != Some(lease.member_id.as_str()) {
                return Err(TaskError::NotLeaseHolder);
            }
        }
        // 4. Compare-and-swap, then 5. lease epoch. Both are stale attempts
        //    by an authorized caller, so both are recorded.
        if let Some(expected) = ctx.expected_revision {
            if expected != task.revision {
                let cause = TaskError::RevisionConflict {
                    expected,
                    actual: task.revision,
                };
                let attempt = task.lease.as_ref().map(|lease| lease.attempt_id.clone());
                return Err(self.reject(ctx, descriptor, task_id, attempt, cause));
            }
        }
        if let Some(epoch) = epoch {
            let current = task.lease.as_ref().map(|lease| lease.epoch);
            if current != Some(epoch) {
                let cause = match current {
                    Some(actual) => TaskError::StaleEpoch {
                        expected: actual,
                        actual: epoch,
                    },
                    None => TaskError::NotLeaseHolder,
                };
                // No live lease is not a stale attempt; it is a caller with no
                // standing, so it is refused without a receipt.
                if current.is_none() {
                    return Err(cause);
                }
                let attempt = task.lease.as_ref().map(|lease| lease.attempt_id.clone());
                return Err(self.reject(ctx, descriptor, task_id, attempt, cause));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn attempt_transition(
        &mut self,
        ctx: CommandContext,
        kind: CommandKind,
        task_id: &str,
        epoch: u64,
        to: TaskState,
        label: &'static str,
        allowed_from: impl Fn(TaskState) -> bool,
    ) -> Result<TaskEvent, TaskError> {
        let descriptor = CommandDescriptor {
            kind,
            task_id: task_id.to_string(),
            expected_revision: ctx.expected_revision,
            epoch: Some(epoch),
            payload: vec![label.to_string()],
        };
        if let Some(existing) = self.replayed(&ctx, &descriptor)? {
            return existing.into_iter().next().ok_or(TaskError::TaskNotFound);
        }
        self.validate(
            &ctx,
            &descriptor,
            task_id,
            Some(epoch),
            true,
            CommandAuthority::OwnerOrHost,
        )?;
        let now = ctx.now_unix_ms;
        {
            let task = self.require(task_id)?;
            if !allowed_from(task.state) {
                return Err(TaskError::InvalidTransition {
                    from: task.state,
                    to: label.to_string(),
                });
            }
        }
        let task = self.require_mut(task_id)?;
        task.state = to;
        let revision = Self::bump(task, now);
        let attempt = task.lease.as_ref().map(|lease| lease.attempt_id.clone());
        Ok(self.record(
            &ctx,
            &descriptor,
            task_id,
            attempt,
            TaskEventKind::Started,
            revision,
            None,
            Vec::new(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        ctx: &CommandContext,
        descriptor: &CommandDescriptor,
        task_id: &str,
        attempt_id: Option<String>,
        kind: TaskEventKind,
        revision: u64,
        artifact_version: Option<u64>,
        evidence: Vec<EvidenceRef>,
    ) -> TaskEvent {
        self.next_event_id = self.next_event_id.saturating_add(1);
        let event = TaskEvent {
            protocol_version: TASK_PROTOCOL_VERSION,
            event_id: format!("tev_{:x}", self.next_event_id),
            sequence: self.next_event_id,
            task_id: task_id.to_string(),
            attempt_id,
            caused_by: ctx.caused_by.clone(),
            actor: ctx.actor.clone(),
            kind,
            idempotency_key: ctx.idempotency_key.clone(),
            fingerprint: descriptor.fingerprint(),
            revision,
            artifact_version,
            evidence,
            created_at_unix_ms: ctx.now_unix_ms,
        };
        // A key may cover several events (a fan-out); each accepted event id
        // is remembered so a retry replays the whole outcome, not a fragment.
        let entry = self
            .applied
            .entry(ctx.idempotency_key.clone())
            .or_insert_with(|| AppliedCommand {
                fingerprint: descriptor.fingerprint(),
                outcome: CommandOutcome::Accepted {
                    event_ids: Vec::new(),
                },
            });
        if let CommandOutcome::Accepted { event_ids } = &mut entry.outcome {
            event_ids.push(event.event_id.clone());
        }
        self.journal.push(JournalEntry::Event(event.clone()));
        event
    }

    /// Folds one recorded event into state during replay.
    fn apply_recorded(&mut self, event: &TaskEvent) -> Result<(), TaskError> {
        self.next_event_id = self.next_event_id.max(event.sequence);
        let entry = self
            .applied
            .entry(event.idempotency_key.clone())
            .or_insert_with(|| AppliedCommand {
                // The fingerprint is carried on the event so a rebuilt ledger
                // can still tell a retry from a different command.
                fingerprint: event.fingerprint.clone(),
                outcome: CommandOutcome::Accepted {
                    event_ids: Vec::new(),
                },
            });
        if let CommandOutcome::Accepted { event_ids } = &mut entry.outcome {
            event_ids.push(event.event_id.clone());
        }
        // The registry moves whether or not a task was affected.
        if let TaskEventKind::ArtifactAdvanced {
            artifact_id,
            version,
            digest,
        } = &event.kind
        {
            self.observe_artifact(
                artifact_id,
                *version,
                Some(digest),
                event.created_at_unix_ms,
            );
            if let Some(artifact) = self
                .artifacts
                .iter_mut()
                .find(|artifact| &artifact.artifact_id == artifact_id)
            {
                artifact.version = *version;
                artifact.digest.clone_from(digest);
                artifact.updated_at_unix_ms = event.created_at_unix_ms;
            }
            self.journal.push(JournalEntry::Event(event.clone()));
            return Ok(());
        }
        match &event.kind {
            TaskEventKind::Created {
                title,
                owner_member_id,
                room_id,
                root_id,
                depends_on,
                artifact_id,
            } => {
                if self.task(&event.task_id).is_some() {
                    return Err(TaskError::TaskExists);
                }
                if let Some(suffix) = event.task_id.strip_prefix("task_") {
                    if let Ok(parsed) = u64::from_str_radix(suffix, 16) {
                        self.next_task_id = self.next_task_id.max(parsed);
                    }
                }
                self.tasks.push(Task {
                    task_id: event.task_id.clone(),
                    room_id: room_id.clone(),
                    title: title.clone(),
                    root_id: root_id.clone(),
                    owner_member_id: owner_member_id.clone(),
                    state: TaskState::Ready,
                    revision: event.revision,
                    depends_on: depends_on.clone(),
                    blockers: Vec::new(),
                    artifact_id: artifact_id.clone(),
                    artifact_version: None,
                    lease: None,
                    attempts: 0,
                    evidence: Vec::new(),
                    verification: None,
                    created_at_unix_ms: event.created_at_unix_ms,
                    updated_at_unix_ms: event.created_at_unix_ms,
                });
            }
            other => {
                let mut observed: Option<ArtifactClaim> = None;
                // A verdict's digest comes from the registry as it stood when
                // the verdict was made, exactly as `verify` computed it.
                // Reading it back off the evidence would invent a digest for
                // a task that has no artifact at all.
                let verified_digest = match other {
                    TaskEventKind::Verified { .. } => self
                        .task(&event.task_id)
                        .and_then(|task| task.artifact_id.clone())
                        .and_then(|artifact_id| {
                            self.artifact(&artifact_id)
                                .map(|artifact| artifact.digest.clone())
                        }),
                    _ => None,
                };
                // The artifact registry is keyed off the same events, so it is
                // rebuilt before the task borrow rather than inside it.
                if let TaskEventKind::ArtifactInvalidated {
                    artifact_id,
                    version,
                    digest,
                } = other
                {
                    match self
                        .artifacts
                        .iter_mut()
                        .find(|artifact| &artifact.artifact_id == artifact_id)
                    {
                        Some(artifact) => {
                            artifact.version = *version;
                            artifact.digest.clone_from(digest);
                            artifact.updated_at_unix_ms = event.created_at_unix_ms;
                        }
                        None => self.artifacts.push(Artifact {
                            artifact_id: artifact_id.clone(),
                            version: *version,
                            digest: digest.clone(),
                            produced_by: None,
                            updated_at_unix_ms: event.created_at_unix_ms,
                        }),
                    }
                }
                if let Some(attempt_id) = event.attempt_id.as_deref() {
                    if let Some(parsed) = attempt_id
                        .strip_prefix("attempt_")
                        .and_then(|suffix| u64::from_str_radix(suffix, 16).ok())
                    {
                        self.next_attempt_id = self.next_attempt_id.max(parsed);
                    }
                }
                if let TaskEventKind::Blocked { blocker_id, .. } = other {
                    if let Some(parsed) = blocker_id
                        .strip_prefix("blocker_")
                        .and_then(|suffix| u64::from_str_radix(suffix, 16).ok())
                    {
                        self.next_blocker_id = self.next_blocker_id.max(parsed);
                    }
                }
                let task = self.require_mut(&event.task_id)?;
                match other {
                    TaskEventKind::Created { .. } | TaskEventKind::ArtifactAdvanced { .. } => {
                        unreachable!("handled before the task borrow")
                    }
                    TaskEventKind::Claimed {
                        epoch,
                        instance_id,
                        expires_at_unix_ms,
                    } => {
                        task.state = TaskState::Leased;
                        task.attempts = task.attempts.saturating_add(1);
                        task.evidence.clear();
                        task.lease = Some(Lease {
                            member_id: task.owner_member_id.clone(),
                            instance_id: instance_id.clone(),
                            epoch: *epoch,
                            attempt_id: event.attempt_id.clone().unwrap_or_default(),
                            expires_at_unix_ms: *expires_at_unix_ms,
                        });
                    }
                    TaskEventKind::Started => task.state = TaskState::Running,
                    TaskEventKind::Blocked { blocker_id, reason } => {
                        task.blockers.push(Blocker {
                            blocker_id: blocker_id.clone(),
                            reason: reason.clone(),
                            raised_at_unix_ms: event.created_at_unix_ms,
                            cleared_at_unix_ms: None,
                        });
                        task.state = TaskState::Blocked;
                    }
                    TaskEventKind::Unblocked { blocker_id } => {
                        if let Some(blocker) = task
                            .blockers
                            .iter_mut()
                            .find(|blocker| &blocker.blocker_id == blocker_id)
                        {
                            blocker.cleared_at_unix_ms = Some(event.created_at_unix_ms);
                        }
                        if !task.has_open_blocker() && task.state == TaskState::Blocked {
                            task.state = TaskState::Ready;
                        }
                    }
                    TaskEventKind::Submitted { artifact } => {
                        task.state = TaskState::Submitted;
                        task.evidence = event.evidence.clone();
                        if let Some(claim) = artifact {
                            task.artifact_id = Some(claim.artifact_id.clone());
                            task.artifact_version = Some(claim.version);
                            observed = Some(claim.clone());
                        }
                    }
                    TaskEventKind::Verified {
                        verifier,
                        rule_version,
                    } => {
                        task.state = TaskState::Verified;
                        task.evidence.extend(event.evidence.iter().cloned());
                        task.verification = Some(Verification {
                            verifier: verifier.clone(),
                            rule_version: *rule_version,
                            artifact_version: event.artifact_version,
                            artifact_digest: verified_digest.clone(),
                            verified_at_unix_ms: event.created_at_unix_ms,
                        });
                        task.lease = None;
                    }
                    TaskEventKind::DependencyAdded { dependency_id } => {
                        if !task.depends_on.iter().any(|id| id == dependency_id) {
                            task.depends_on.push(dependency_id.clone());
                        }
                    }
                    TaskEventKind::LeaseReleased { .. } => {
                        task.lease = None;
                        if matches!(task.state, TaskState::Leased | TaskState::Running) {
                            task.state = TaskState::Ready;
                        }
                    }
                    TaskEventKind::DependencyInvalidated { .. } => {
                        task.state = TaskState::Ready;
                        task.verification = None;
                        task.evidence.clear();
                        task.lease = None;
                    }
                    TaskEventKind::Failed { .. } => {
                        task.state = TaskState::Failed;
                        task.lease = None;
                    }
                    TaskEventKind::Cancelled { .. } => {
                        task.state = TaskState::Cancelled;
                        task.lease = None;
                    }
                    TaskEventKind::ArtifactInvalidated { version, .. } => {
                        task.state = TaskState::Ready;
                        task.artifact_version = Some(*version);
                        task.lease = None;
                        task.evidence.clear();
                        task.verification = None;
                    }
                    TaskEventKind::Reassigned { owner_member_id } => {
                        task.owner_member_id = owner_member_id.clone();
                        task.lease = None;
                    }
                }
                let task = self.require_mut(&event.task_id)?;
                task.revision = event.revision;
                task.updated_at_unix_ms = event.created_at_unix_ms;
                if let Some(claim) = observed {
                    self.observe_artifact(
                        &claim.artifact_id,
                        claim.version,
                        Some(&claim.digest),
                        event.created_at_unix_ms,
                    );
                }
            }
        }
        self.journal.push(JournalEntry::Event(event.clone()));
        Ok(())
    }
}

impl TaskLedger {
    /// Applies one command to this ledger and reports what it decided.
    ///
    /// Pure in the sense that matters here: it moves only this value, never
    /// storage. A durable store stages this on a copy, commits the delta, and
    /// only then publishes the result.
    pub(crate) fn reduce(
        &mut self,
        ctx: CommandContext,
        mutation: TaskMutation,
    ) -> CommittedOutcome {
        // Every command routes through here, so the receipt written during
        // validation is the one handed back with the error.
        let before = self.journal.len();
        let outcome = match mutation {
            TaskMutation::Create {
                room_id,
                root_id,
                title,
                owner_member_id,
                depends_on,
                artifact_id,
            } => self
                .create(
                    ctx,
                    room_id,
                    root_id,
                    title,
                    owner_member_id,
                    depends_on,
                    artifact_id,
                )
                .map(|event| CommandResult {
                    events: vec![event],
                    decisions: Vec::new(),
                }),
            TaskMutation::Claim {
                task_id,
                instance_id,
                lease_ms,
            } => self
                .claim(ctx, &task_id, instance_id, lease_ms)
                .map(|event| CommandResult {
                    events: vec![event],
                    decisions: Vec::new(),
                }),
            TaskMutation::Start { task_id, epoch } => {
                self.start(ctx, &task_id, epoch).map(|event| CommandResult {
                    events: vec![event],
                    decisions: Vec::new(),
                })
            }
            TaskMutation::Submit {
                task_id,
                epoch,
                artifact,
                evidence,
            } => self
                .submit(ctx, &task_id, epoch, artifact, evidence)
                .map(|event| CommandResult {
                    events: vec![event],
                    decisions: Vec::new(),
                }),
            TaskMutation::Verify {
                task_id,
                verifier,
                rule_version,
                acceptance,
            } => self
                .verify(ctx, &task_id, verifier, rule_version, acceptance)
                .map(|event| CommandResult {
                    events: vec![event],
                    decisions: Vec::new(),
                }),
            TaskMutation::Fail {
                task_id,
                epoch,
                reason,
            } => self
                .fail(ctx, &task_id, epoch, reason)
                .map(|event| CommandResult {
                    events: vec![event],
                    decisions: Vec::new(),
                }),
            TaskMutation::Block {
                task_id,
                epoch,
                reason,
            } => self
                .block(ctx, &task_id, epoch, reason)
                .map(|event| CommandResult {
                    events: vec![event],
                    decisions: Vec::new(),
                }),
            TaskMutation::Unblock {
                task_id,
                blocker_id,
            } => self
                .unblock(ctx, &task_id, &blocker_id)
                .map(|event| CommandResult {
                    events: vec![event],
                    decisions: Vec::new(),
                }),
            TaskMutation::AddDependency {
                task_id,
                dependency_id,
            } => self
                .add_dependency(ctx, &task_id, &dependency_id)
                .map(|events| CommandResult {
                    events,
                    decisions: Vec::new(),
                }),
            TaskMutation::AdvanceArtifact {
                artifact_id,
                digest,
            } => self
                .advance_artifact(ctx, &artifact_id, digest)
                .map(|events| CommandResult {
                    events,
                    decisions: Vec::new(),
                }),
            TaskMutation::ProposeDecision {
                room_id,
                title,
                statement,
                rationale,
                supersedes,
                evidence,
            } => self
                .propose_decision(
                    ctx, room_id, title, statement, rationale, supersedes, evidence,
                )
                .map(|event| CommandResult {
                    events: Vec::new(),
                    decisions: vec![event],
                }),
            TaskMutation::AcceptDecision { decision_id } => self
                .accept_decision(ctx, &decision_id)
                .map(|decisions| CommandResult {
                    events: Vec::new(),
                    decisions,
                }),
        };
        match outcome {
            Ok(result) => CommittedOutcome::Applied(result),
            Err(error) => {
                // A receipt written by this command is the last journal entry.
                let receipt = self
                    .journal
                    .get(before..)
                    .and_then(|written| written.iter().rev().find_map(JournalEntry::as_rejection))
                    .cloned()
                    .map(Box::new);
                CommittedOutcome::Rejected(CommandRejection { error, receipt })
            }
        }
    }

    /// Computes a command's whole outcome without moving this ledger.
    pub(crate) fn stage(&self, ctx: CommandContext, mutation: TaskMutation) -> StagedCommit {
        // The copy is what makes "nothing moved" true even for a command that
        // writes a rejection receipt: the receipt lives in the delta until it
        // is committed, not in the live projection.
        let mut next = self.clone();
        let before = next.journal.len();
        let outcome = next.reduce(ctx, mutation);
        let delta = next.journal.get(before..).unwrap_or_default().to_vec();
        StagedCommit {
            delta,
            outcome,
            next,
        }
    }

    /// Publishes a staged commit. Call only after its delta is durable.
    pub(crate) fn publish(&mut self, staged: StagedCommit) -> CommittedOutcome {
        *self = staged.next;
        staged.outcome
    }

    fn journal_page_inner(&self, cursor: Option<u64>, limit: usize) -> JournalPage {
        let after = cursor.unwrap_or(0);
        let entries: Vec<JournalEntry> = self
            .journal
            .iter()
            .filter(|entry| entry.sequence() > after)
            .take(limit)
            .cloned()
            .collect();
        let next_cursor = entries
            .last()
            .map(JournalEntry::sequence)
            .filter(|last| self.journal.iter().any(|entry| entry.sequence() > *last));
        JournalPage {
            entries,
            next_cursor,
        }
    }

    /// Ready work in a stable order: oldest first, ties broken by id.
    ///
    /// Deterministic ordering matters because two schedulers reading the same
    /// ledger must choose the same work, and a test must be able to assert
    /// what comes back.
    fn ready_page_inner(&self, now_unix_ms: u64, limit: usize) -> Vec<&Task> {
        let mut ready = self.ready_tasks(now_unix_ms);
        ready.sort_by(|left, right| {
            left.created_at_unix_ms
                .cmp(&right.created_at_unix_ms)
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
        ready.truncate(limit);
        ready
    }
}

impl TaskStore for TaskLedger {
    /// The in-memory ledger has no storage beneath it, so a command can only
    /// be applied or rejected, never lost.
    fn apply(
        &mut self,
        ctx: CommandContext,
        mutation: TaskMutation,
    ) -> Result<CommittedOutcome, StoreError> {
        let staged = self.stage(ctx, mutation);
        Ok(self.publish(staged))
    }

    fn task(&self, task_id: &str) -> Option<&Task> {
        TaskLedger::task(self, task_id)
    }

    fn tasks(&self) -> &[Task] {
        TaskLedger::tasks(self)
    }

    fn ready_tasks(&self, now_unix_ms: u64) -> Vec<&Task> {
        TaskLedger::ready_tasks(self, now_unix_ms)
    }

    fn artifact(&self, artifact_id: &str) -> Option<&Artifact> {
        TaskLedger::artifact(self, artifact_id)
    }

    fn decision(&self, decision_id: &str) -> Option<&DecisionRecord> {
        TaskLedger::decision(self, decision_id)
    }

    fn journal(&self) -> &[JournalEntry] {
        TaskLedger::journal(self)
    }

    fn receipt(&self, receipt_id: &str) -> Option<&RejectionReceipt> {
        TaskLedger::receipt(self, receipt_id)
    }

    fn journal_page(&self, cursor: Option<u64>, limit: usize) -> JournalPage {
        self.journal_page_inner(cursor, limit)
    }

    fn ready_page(&self, now_unix_ms: u64, limit: usize) -> Vec<&Task> {
        self.ready_page_inner(now_unix_ms, limit)
    }
}

/// Applies one command with commit-before-reply ordering.
///
/// The single implementation of the rule: stage on a copy, make the delta
/// durable, then publish. Every store goes through here so the ordering
/// cannot drift between the in-tree wrapper and the runtime's own.
pub(crate) fn commit_mutation<S: JournalSink + ?Sized>(
    ledger: &mut TaskLedger,
    sink: &mut S,
    ctx: CommandContext,
    mutation: TaskMutation,
) -> Result<CommittedOutcome, StoreError> {
    let staged = ledger.stage(ctx, mutation);
    // A command that changed nothing has nothing to commit.
    if !staged.delta.is_empty() {
        sink.persist(&staged.delta)?;
    }
    Ok(ledger.publish(staged))
}

/// A store that commits every journal delta before it answers.
///
/// This is the shape the runtime needs: stage on a copy, persist the delta,
/// publish the projection, then return. A persistence failure surfaces as a
/// [`StoreError`] with the live projection untouched, so no caller can be
/// told a command succeeded — or was durably refused — before its record
/// exists.
#[derive(Debug, Clone)]
pub(crate) struct DurableStore<S: JournalSink> {
    ledger: TaskLedger,
    sink: S,
}

impl<S: JournalSink> DurableStore<S> {
    pub(crate) fn new(ledger: TaskLedger, sink: S) -> Self {
        Self { ledger, sink }
    }

    pub(crate) fn ledger(&self) -> &TaskLedger {
        &self.ledger
    }

    pub(crate) fn into_parts(self) -> (TaskLedger, S) {
        (self.ledger, self.sink)
    }
}

impl<S: JournalSink> TaskStore for DurableStore<S> {
    fn apply(
        &mut self,
        ctx: CommandContext,
        mutation: TaskMutation,
    ) -> Result<CommittedOutcome, StoreError> {
        commit_mutation(&mut self.ledger, &mut self.sink, ctx, mutation)
    }

    fn task(&self, task_id: &str) -> Option<&Task> {
        self.ledger.task(task_id)
    }

    fn tasks(&self) -> &[Task] {
        TaskLedger::tasks(&self.ledger)
    }

    fn ready_tasks(&self, now_unix_ms: u64) -> Vec<&Task> {
        TaskLedger::ready_tasks(&self.ledger, now_unix_ms)
    }

    fn artifact(&self, artifact_id: &str) -> Option<&Artifact> {
        self.ledger.artifact(artifact_id)
    }

    fn decision(&self, decision_id: &str) -> Option<&DecisionRecord> {
        self.ledger.decision(decision_id)
    }

    fn journal(&self) -> &[JournalEntry] {
        TaskLedger::journal(&self.ledger)
    }

    fn receipt(&self, receipt_id: &str) -> Option<&RejectionReceipt> {
        self.ledger.receipt(receipt_id)
    }

    fn journal_page(&self, cursor: Option<u64>, limit: usize) -> JournalPage {
        self.ledger.journal_page_inner(cursor, limit)
    }

    fn ready_page(&self, now_unix_ms: u64, limit: usize) -> Vec<&Task> {
        self.ledger.ready_page_inner(now_unix_ms, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> TaskActor {
        TaskActor::Host
    }

    fn member(id: &str) -> TaskActor {
        TaskActor::Member {
            member_id: id.to_string(),
        }
    }

    fn ctx(actor: TaskActor, key: &str, expected: Option<u64>, now: u64) -> CommandContext {
        CommandContext {
            actor,
            idempotency_key: key.to_string(),
            expected_revision: expected,
            caused_by: None,
            now_unix_ms: now,
        }
    }

    /// Current revision of a task, which every non-create command must state.
    fn at(ledger: &TaskLedger, task_id: &str) -> Option<u64> {
        ledger.task(task_id).map(|task| task.revision)
    }

    fn evidence(version: Option<u64>) -> Vec<EvidenceRef> {
        vec![EvidenceRef {
            kind: EvidenceKind::Command,
            handle: "just test".into(),
            digest: "sha256:abc".into(),
            artifact_version: version,
        }]
    }

    fn claim_of(version: u64) -> Option<ArtifactClaim> {
        Some(ArtifactClaim {
            artifact_id: "protocol".into(),
            version,
            digest: "sha256:abc".into(),
        })
    }

    /// A ledger with one ready task owned by `seat_11`.
    fn ledger_with_task() -> (TaskLedger, String) {
        let mut ledger = TaskLedger::default();
        let event = ledger
            .create(
                ctx(host(), "k-create", None, 10),
                "room_3".into(),
                "evt_2b".into(),
                "Protocol foundation".into(),
                "seat_11".into(),
                Vec::new(),
                Some("protocol".into()),
            )
            .expect("create");
        (ledger, event.task_id)
    }

    fn epoch_of(ledger: &TaskLedger, task_id: &str) -> u64 {
        ledger
            .task(task_id)
            .and_then(|task| task.lease.as_ref().map(|lease| lease.epoch))
            .expect("lease")
    }

    /// Drives a task from Ready to Verified the way a worker and a reviewer
    /// would, keeping every command revision-correct.
    fn drive_to_verified(
        ledger: &mut TaskLedger,
        task_id: &str,
        worker: &str,
        reviewer: &str,
        version: u64,
        clock: &mut u64,
    ) {
        drive_to_verified_against(
            ledger,
            task_id,
            worker,
            reviewer,
            "protocol",
            "sha256:abc",
            version,
            clock,
        );
    }

    /// Drives a task to Verified against a named artifact, so a test can keep
    /// two pieces of work on separate artifacts.
    #[allow(clippy::too_many_arguments)]
    fn drive_to_verified_against(
        ledger: &mut TaskLedger,
        task_id: &str,
        worker: &str,
        reviewer: &str,
        artifact_id: &str,
        digest: &str,
        version: u64,
        clock: &mut u64,
    ) {
        *clock += 1;
        let key = format!("c-{task_id}-{clock}");
        ledger
            .claim(
                ctx(member(worker), &key, at(ledger, task_id), *clock),
                task_id,
                "agent_x".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        let epoch = epoch_of(ledger, task_id);
        *clock += 1;
        let key = format!("s-{task_id}-{clock}");
        ledger
            .submit(
                ctx(member(worker), &key, at(ledger, task_id), *clock),
                task_id,
                epoch,
                Some(ArtifactClaim {
                    artifact_id: artifact_id.to_string(),
                    version,
                    digest: digest.to_string(),
                }),
                vec![EvidenceRef {
                    kind: EvidenceKind::Command,
                    handle: "just test".into(),
                    digest: digest.to_string(),
                    artifact_version: Some(version),
                }],
            )
            .expect("submit");
        *clock += 1;
        let key = format!("v-{task_id}-{clock}");
        ledger
            .verify(
                ctx(member(reviewer), &key, at(ledger, task_id), *clock),
                task_id,
                reviewer.to_string(),
                1,
                vec![EvidenceRef {
                    kind: EvidenceKind::Command,
                    handle: "just test".into(),
                    digest: digest.to_string(),
                    artifact_version: Some(version),
                }],
            )
            .expect("verify");
    }

    #[test]
    fn a_task_starts_ready_with_one_owner_and_a_stamped_event() {
        let (ledger, task_id) = ledger_with_task();
        let task = ledger.task(&task_id).expect("task");
        assert_eq!(task.state, TaskState::Ready);
        assert_eq!(task.owner_member_id, "seat_11");
        assert_eq!(task.revision, 1);
        let event = ledger.events().next().expect("event");
        assert_eq!(event.protocol_version, TASK_PROTOCOL_VERSION);
        assert_eq!(event.idempotency_key, "k-create");
        assert!(!event.fingerprint.is_empty());
        assert_eq!(ledger.ready_tasks(20).len(), 1);
        ledger.assert_invariants().expect("invariants");
    }

    #[test]
    fn every_command_but_creation_must_state_the_revision_it_changes() {
        // Optional CAS meant any caller could skip the check entirely, which
        // made concurrent writers silently last-write-wins.
        let (mut ledger, task_id) = ledger_with_task();
        assert_eq!(
            ledger.claim(
                ctx(member("seat_11"), "k1", None, 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS
            ),
            Err(TaskError::RevisionRequired)
        );
        ledger
            .claim(
                ctx(member("seat_11"), "k2", Some(1), 21),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim with a revision");
    }

    #[test]
    fn a_stale_writer_is_refused_by_compare_and_swap() {
        let (mut ledger, task_id) = ledger_with_task();
        ledger
            .claim(
                ctx(member("seat_11"), "k-claim", Some(1), 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        // A caller still holding revision 1 has not seen the claim.
        let stale = ledger.block(
            ctx(host(), "k-block", Some(1), 30),
            &task_id,
            None,
            "needs review".into(),
        );
        assert_eq!(
            stale,
            Err(TaskError::RevisionConflict {
                expected: 1,
                actual: 2
            })
        );
        // A stale CAS is a recorded refusal, and it changes nothing.
        assert_eq!(ledger.rejections().count(), 1);
        assert_eq!(ledger.task(&task_id).expect("task").revision, 2);
    }

    #[test]
    fn a_replaced_worker_cannot_submit_over_its_successor() {
        let (mut ledger, task_id) = ledger_with_task();
        ledger
            .claim(
                ctx(member("seat_11"), "k1", at(&ledger, &task_id), 20),
                &task_id,
                "agent_1".into(),
                10,
            )
            .expect("first claim");
        let first_epoch = epoch_of(&ledger, &task_id);
        // The lease lapses and a replacement incarnation takes the task.
        ledger
            .claim(
                ctx(member("seat_11"), "k2", at(&ledger, &task_id), 100),
                &task_id,
                "agent_2".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("second claim");
        let successor = ledger.task(&task_id).expect("task").clone();
        let artifacts_before = ledger.artifact("protocol").cloned();

        // The exact command, kept verbatim: a retry that refreshed its
        // expected revision would be a different command, not a retry.
        let stale_revision = at(&ledger, &task_id);
        let stale = ledger.submit(
            ctx(member("seat_11"), "k3", stale_revision, 110),
            &task_id,
            first_epoch,
            claim_of(1),
            evidence(Some(1)),
        );
        assert_eq!(
            stale,
            Err(TaskError::StaleEpoch {
                expected: 2,
                actual: 1
            }),
            "the dead worker's submission must not overwrite its successor"
        );

        // Nothing about the task, its lease, or the artifact moved.
        assert_eq!(ledger.task(&task_id).expect("task"), &successor);
        assert_eq!(ledger.artifact("protocol").cloned(), artifacts_before);

        // Exactly one receipt, carrying what was refused and what was true.
        let receipts: Vec<&RejectionReceipt> = ledger.rejections().collect();
        assert_eq!(receipts.len(), 1);
        let receipt = receipts[0];
        assert_eq!(receipt.protocol_version, TASK_PROTOCOL_VERSION);
        assert_eq!(receipt.command, CommandKind::Submit);
        assert_eq!(receipt.idempotency_key, "k3");
        assert_eq!(receipt.task_id, task_id);
        assert_eq!(receipt.submitted_epoch, Some(first_epoch));
        assert_eq!(receipt.observed_epoch, Some(2));
        assert_eq!(receipt.observed_revision, successor.revision);
        assert_eq!(receipt.error_code, "task_stale_epoch");
        assert_eq!(receipt.actor, member("seat_11"));
        assert!(receipt.submitted_attempt.is_some());

        // An exact retry returns the original rejection without appending.
        let retry = ledger.submit(
            ctx(member("seat_11"), "k3", stale_revision, 111),
            &task_id,
            first_epoch,
            claim_of(1),
            evidence(Some(1)),
        );
        assert_eq!(retry, stale);
        assert_eq!(ledger.rejections().count(), 1, "no second append");

        // Still stable after the lease is replaced again.
        ledger
            .claim(
                ctx(member("seat_11"), "k4", at(&ledger, &task_id), 1_000_000),
                &task_id,
                "agent_3".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("third claim");
        let retry = ledger.submit(
            ctx(member("seat_11"), "k3", stale_revision, 1_000_001),
            &task_id,
            first_epoch,
            claim_of(1),
            evidence(Some(1)),
        );
        assert_eq!(retry, stale, "the answer to a given command does not drift");
        assert_eq!(ledger.rejections().count(), 1);

        // Reusing the key for different work is a conflict, not a retry.
        assert_eq!(
            ledger.submit(
                // Same key, same revision, different work.
                ctx(member("seat_11"), "k3", stale_revision, 1_000_002),
                &task_id,
                first_epoch,
                claim_of(9),
                evidence(Some(9)),
            ),
            Err(TaskError::IdempotencyConflict { key: "k3".into() })
        );

        // And the whole thing, refusals included, replays exactly.
        let replayed = TaskLedger::replay(ledger.journal()).expect("replay");
        assert_eq!(replayed, ledger);
        // A refusal replayed after a restart is still refused, not re-run.
        let mut restored = replayed;
        let after_restart = restored.submit(
            ctx(member("seat_11"), "k3", stale_revision, 1_000_003),
            &task_id,
            first_epoch,
            claim_of(1),
            evidence(Some(1)),
        );
        assert_eq!(after_restart, stale);
        assert_eq!(restored.rejections().count(), 1);
    }

    #[test]
    fn malformed_and_unauthorized_commands_never_reach_the_journal() {
        // A durable receipt is for an authorized caller that lost a race, not
        // for anything that can reach the API.
        let (mut ledger, task_id) = ledger_with_task();
        ledger
            .claim(
                ctx(member("seat_11"), "k1", at(&ledger, &task_id), 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        let epoch = epoch_of(&ledger, &task_id);
        let before = ledger.clone();

        assert_eq!(
            ledger.submit(
                ctx(member("stranger"), "u1", at(&ledger, &task_id), 30),
                &task_id,
                epoch,
                claim_of(1),
                evidence(Some(1))
            ),
            Err(TaskError::NotAuthorized)
        );
        assert_eq!(
            ledger.submit(
                ctx(member("seat_11"), "u2", at(&ledger, &task_id), 31),
                &task_id,
                epoch,
                claim_of(1),
                Vec::new()
            ),
            Err(TaskError::EvidenceRequired)
        );
        assert_eq!(
            ledger.submit(
                ctx(member("seat_11"), "   ", at(&ledger, &task_id), 32),
                &task_id,
                epoch,
                claim_of(1),
                evidence(Some(1))
            ),
            Err(TaskError::IdempotencyKeyEmpty)
        );
        assert_eq!(
            before, ledger,
            "refusals that are not stale attempts leave no trace"
        );
    }

    #[test]
    fn submitting_is_a_claim_and_verifying_needs_matching_acceptance_evidence() {
        let (mut ledger, task_id) = ledger_with_task();
        ledger
            .claim(
                ctx(member("seat_11"), "k1", at(&ledger, &task_id), 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        let epoch = epoch_of(&ledger, &task_id);
        ledger
            .submit(
                ctx(member("seat_11"), "k2", at(&ledger, &task_id), 30),
                &task_id,
                epoch,
                claim_of(4),
                evidence(Some(4)),
            )
            .expect("submit");
        assert_eq!(
            ledger.task(&task_id).expect("task").state,
            TaskState::Submitted,
            "a worker's word is a claim, not a verdict"
        );

        // The worker cannot accept its own claim.
        assert_eq!(
            ledger.verify(
                ctx(member("seat_11"), "k3", at(&ledger, &task_id), 40),
                &task_id,
                "seat_11".into(),
                1,
                evidence(Some(4))
            ),
            Err(TaskError::NotAuthorized)
        );
        // Nor may a reviewer accept it against a version it did not check.
        assert_eq!(
            ledger.verify(
                ctx(host(), "k4", at(&ledger, &task_id), 41),
                &task_id,
                "seat_10".into(),
                1,
                evidence(Some(3))
            ),
            Err(TaskError::EvidenceVersionMismatch {
                cited: Some(3),
                current: Some(4)
            })
        );
        ledger
            .verify(
                ctx(host(), "k5", at(&ledger, &task_id), 42),
                &task_id,
                "seat_10".into(),
                1,
                evidence(Some(4)),
            )
            .expect("verify");
        assert_eq!(
            ledger.task(&task_id).expect("task").state,
            TaskState::Verified
        );
        ledger.assert_invariants().expect("invariants");
    }

    #[test]
    fn acceptance_must_name_a_verifier_and_cite_the_current_bytes() {
        let (mut ledger, task_id) = ledger_with_task();
        ledger
            .claim(
                ctx(member("seat_11"), "k1", at(&ledger, &task_id), 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        let epoch = epoch_of(&ledger, &task_id);
        ledger
            .submit(
                ctx(member("seat_11"), "k2", at(&ledger, &task_id), 30),
                &task_id,
                epoch,
                claim_of(1),
                evidence(Some(1)),
            )
            .expect("submit");

        assert_eq!(
            ledger.verify(
                ctx(host(), "k3", at(&ledger, &task_id), 40),
                &task_id,
                "  ".into(),
                1,
                evidence(Some(1))
            ),
            Err(TaskError::VerifierEmpty),
            "a verdict with no verifier is unattributable"
        );

        // Same version, different bytes: a rebuild that kept the number is
        // not the thing the reviewer checked.
        let rebuilt = vec![EvidenceRef {
            kind: EvidenceKind::Command,
            handle: "just test".into(),
            digest: "sha256:different".into(),
            artifact_version: Some(1),
        }];
        assert_eq!(
            ledger.verify(
                ctx(host(), "k4", at(&ledger, &task_id), 41),
                &task_id,
                "seat_10".into(),
                1,
                rebuilt
            ),
            Err(TaskError::EvidenceDigestMismatch {
                artifact_id: "protocol".into()
            })
        );

        ledger
            .verify(
                ctx(host(), "k5", at(&ledger, &task_id), 42),
                &task_id,
                "seat_10".into(),
                7,
                evidence(Some(1)),
            )
            .expect("verify");
        let verification = ledger
            .task(&task_id)
            .and_then(|task| task.verification.clone())
            .expect("verdict");
        assert_eq!(verification.verifier, "seat_10");
        assert_eq!(verification.rule_version, 7);
        assert_eq!(verification.artifact_digest.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn an_artifact_identity_comes_from_the_submission_not_from_the_evidence() {
        // Evidence may be a command or a transcript, whose digest is not the
        // artifact's; the submission states what it produced.
        let (mut ledger, task_id) = ledger_with_task();
        ledger
            .claim(
                ctx(member("seat_11"), "k1", at(&ledger, &task_id), 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        let epoch = epoch_of(&ledger, &task_id);
        let transcript = vec![EvidenceRef {
            kind: EvidenceKind::Transcript,
            handle: "pane wT:p2".into(),
            digest: "sha256:transcript".into(),
            artifact_version: Some(5),
        }];
        ledger
            .submit(
                ctx(member("seat_11"), "k2", at(&ledger, &task_id), 30),
                &task_id,
                epoch,
                Some(ArtifactClaim {
                    artifact_id: "protocol".into(),
                    version: 5,
                    digest: "sha256:artifact".into(),
                }),
                transcript,
            )
            .expect("submit");
        let artifact = ledger.artifact("protocol").expect("artifact");
        assert_eq!(
            artifact.digest, "sha256:artifact",
            "the artifact digest is the one claimed, not the transcript's"
        );
        assert_eq!(artifact.version, 5);
    }

    #[test]
    fn blocking_releases_the_lease_and_only_the_live_attempt_may_do_it() {
        // The invariant said a lease exists only while work is out, but block
        // left one behind, so a blocked task stayed owned by a stopped worker.
        let (mut ledger, task_id) = ledger_with_task();
        ledger
            .claim(
                ctx(member("seat_11"), "k1", at(&ledger, &task_id), 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        let epoch = epoch_of(&ledger, &task_id);

        // A member must name the attempt it is ending.
        assert_eq!(
            ledger.block(
                ctx(member("seat_11"), "k2", at(&ledger, &task_id), 21),
                &task_id,
                None,
                "question".into()
            ),
            Err(TaskError::NotLeaseHolder)
        );
        // A stale attempt cannot end its successor's work.
        assert_eq!(
            ledger.block(
                ctx(member("seat_11"), "k3", at(&ledger, &task_id), 22),
                &task_id,
                Some(epoch + 5),
                "question".into()
            ),
            Err(TaskError::StaleEpoch {
                expected: epoch,
                actual: epoch + 5
            })
        );
        ledger
            .block(
                ctx(member("seat_11"), "k4", at(&ledger, &task_id), 23),
                &task_id,
                Some(epoch),
                "question".into(),
            )
            .expect("block");
        let task = ledger.task(&task_id).expect("task");
        assert_eq!(task.state, TaskState::Blocked);
        assert!(task.lease.is_none(), "a blocked task is not being worked");
        ledger.assert_invariants().expect("invariants");

        // The host may clear stuck work without naming an attempt.
        ledger
            .fail(
                ctx(host(), "k5", at(&ledger, &task_id), 24),
                &task_id,
                None,
                "abandoned".into(),
            )
            .expect("host fail");
        assert_eq!(
            ledger.task(&task_id).expect("task").state,
            TaskState::Failed
        );
    }

    #[test]
    fn a_blocker_holds_a_task_back_until_it_is_cleared_and_is_never_dropped() {
        let (mut ledger, task_id) = ledger_with_task();
        let raised = ledger
            .block(
                ctx(host(), "k1", at(&ledger, &task_id), 20),
                &task_id,
                None,
                "needs a decision".into(),
            )
            .expect("block");
        let TaskEventKind::Blocked { blocker_id, .. } = raised.kind.clone() else {
            panic!("expected a blocker");
        };
        assert!(ledger.ready_tasks(30).is_empty());
        assert_eq!(
            ledger.claim(
                ctx(member("seat_11"), "k2", at(&ledger, &task_id), 30),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS
            ),
            Err(TaskError::BlockerOpen {
                blocker_id: blocker_id.clone()
            })
        );

        ledger
            .block(
                ctx(host(), "k3", at(&ledger, &task_id), 31),
                &task_id,
                None,
                "second reason".into(),
            )
            .expect("second blocker");
        ledger
            .unblock(
                ctx(host(), "k4", at(&ledger, &task_id), 40),
                &task_id,
                &blocker_id,
            )
            .expect("clear one");
        assert!(
            ledger.ready_tasks(41).is_empty(),
            "one cleared blocker does not clear the rest"
        );
        let task = ledger.task(&task_id).expect("task");
        assert_eq!(task.blockers.len(), 2);
        assert_eq!(task.open_blockers().count(), 1);
    }

    #[test]
    fn dependencies_and_lapsed_leases_decide_readiness() {
        let (mut ledger, first) = ledger_with_task();
        let second = ledger
            .create(
                ctx(host(), "k-dep", None, 11),
                "room_3".into(),
                "evt_2b".into(),
                "Scheduler".into(),
                "seat_10".into(),
                vec![first.clone()],
                None,
            )
            .expect("create")
            .task_id;
        let ready: Vec<&str> = ledger
            .ready_tasks(20)
            .iter()
            .map(|task| task.task_id.as_str())
            .collect();
        assert_eq!(ready, vec![first.as_str()], "the dependent task waits");

        ledger
            .claim(
                ctx(member("seat_11"), "k1", at(&ledger, &first), 20),
                &first,
                "agent_1".into(),
                10,
            )
            .expect("claim");
        assert!(ledger.ready_tasks(25).is_empty(), "a live lease owns it");
        assert_eq!(
            ledger.ready_tasks(31).len(),
            1,
            "an expired lease releases it"
        );
        assert!(ledger
            .task(&second)
            .expect("task")
            .depends_on
            .contains(&first));
    }

    #[test]
    fn work_may_not_depend_on_another_rooms_work() {
        let (mut ledger, first) = ledger_with_task();
        let elsewhere = ledger
            .create(
                ctx(host(), "k-other", None, 11),
                "room_9".into(),
                "evt_x".into(),
                "Other room".into(),
                "seat_11".into(),
                Vec::new(),
                None,
            )
            .expect("create")
            .task_id;
        assert_eq!(
            ledger.create(
                ctx(host(), "k-cross", None, 12),
                "room_3".into(),
                "evt_2b".into(),
                "Cross".into(),
                "seat_11".into(),
                vec![elsewhere.clone()],
                None,
            ),
            Err(TaskError::CrossRoomDependency {
                task_id: elsewhere.clone()
            })
        );
        assert_eq!(
            ledger.add_dependency(
                ctx(host(), "k-cross2", at(&ledger, &first), 13),
                &first,
                &elsewhere
            ),
            Err(TaskError::CrossRoomDependency { task_id: elsewhere })
        );
    }

    #[test]
    fn a_dependency_cycle_is_refused_before_anything_changes() {
        let (mut ledger, first) = ledger_with_task();
        let second = ledger
            .create(
                ctx(host(), "k-b", None, 11),
                "room_3".into(),
                "evt_2b".into(),
                "Second".into(),
                "seat_10".into(),
                vec![first.clone()],
                None,
            )
            .expect("create")
            .task_id;

        let before = ledger.clone();
        assert_eq!(
            ledger.add_dependency(
                ctx(host(), "k-cycle", at(&ledger, &first), 20),
                &first,
                &second
            ),
            Err(TaskError::DependencyCycle {
                task_id: second.clone()
            })
        );
        assert_eq!(before, ledger, "a refused cycle leaves no trace");
        assert_eq!(
            ledger.add_dependency(
                ctx(host(), "k-self", at(&ledger, &first), 21),
                &first,
                &first
            ),
            Err(TaskError::DependencyCycle {
                task_id: first.clone()
            })
        );
        ledger.assert_invariants().expect("invariants");
    }

    #[test]
    fn advancing_an_artifact_is_recorded_even_when_no_task_is_affected() {
        // The registry moved; a journal that omitted it would rebuild a
        // ledger pointing at content that no longer exists.
        let (mut ledger, _task_id) = ledger_with_task();
        let events = ledger
            .advance_artifact(ctx(host(), "adv", None, 50), "protocol", "sha256:v1".into())
            .expect("advance");
        assert_eq!(events.len(), 1, "one command-level event, no fan-out");
        assert!(matches!(
            events[0].kind,
            TaskEventKind::ArtifactAdvanced { .. }
        ));
        assert_eq!(ledger.artifact("protocol").expect("artifact").version, 1);
        assert_eq!(
            TaskLedger::replay(ledger.journal()).expect("replay"),
            ledger,
            "an advance with no affected task must survive replay"
        );
    }

    #[test]
    fn advancing_an_artifact_withdraws_the_verdicts_that_rested_on_it() {
        let mut clock = 100;
        let (mut ledger, base) = ledger_with_task();
        let middle = ledger
            .create(
                ctx(host(), "k-mid", None, 11),
                "room_3".into(),
                "evt_2b".into(),
                "Scheduler".into(),
                "seat_10".into(),
                vec![base.clone()],
                None,
            )
            .expect("create")
            .task_id;
        let top = ledger
            .create(
                ctx(host(), "k-top", None, 12),
                "room_3".into(),
                "evt_2b".into(),
                "Integration".into(),
                "seat_10".into(),
                vec![middle.clone()],
                None,
            )
            .expect("create")
            .task_id;

        drive_to_verified(&mut ledger, &base, "seat_11", "seat_10", 1, &mut clock);
        drive_to_verified(&mut ledger, &middle, "seat_10", "seat_11", 1, &mut clock);
        drive_to_verified(&mut ledger, &top, "seat_10", "seat_11", 1, &mut clock);
        for id in [&base, &middle, &top] {
            assert_eq!(ledger.task(id).expect("task").state, TaskState::Verified);
        }

        clock += 1;
        let events = ledger
            .advance_artifact(
                ctx(host(), "inv", None, clock),
                "protocol",
                "sha256:v2".into(),
            )
            .expect("advance");

        let parent = &events[0];
        assert!(matches!(
            parent.kind,
            TaskEventKind::ArtifactAdvanced { .. }
        ));
        assert!(
            events[1..]
                .iter()
                .all(|event| event.caused_by.as_deref() == Some(parent.event_id.as_str())),
            "the fan-out hangs beneath the command that caused it"
        );

        assert_eq!(ledger.task(&base).expect("task").state, TaskState::Ready);
        assert_eq!(
            ledger.task(&middle).expect("task").state,
            TaskState::Ready,
            "a verdict that rested on the invalidated task is stale"
        );
        assert_eq!(
            ledger.task(&top).expect("task").state,
            TaskState::Ready,
            "and the withdrawal follows the chain outward"
        );
        assert!(ledger.task(&middle).expect("task").verification.is_none());

        let ready: Vec<&str> = ledger
            .ready_tasks(clock + 1)
            .iter()
            .map(|task| task.task_id.as_str())
            .collect();
        assert_eq!(ready, vec![base.as_str()]);
        ledger.assert_invariants().expect("invariants");
        assert_eq!(
            TaskLedger::replay(ledger.journal()).expect("replay"),
            ledger,
            "the cascade must be recorded, not merely computed"
        );
    }

    #[test]
    fn a_retried_command_returns_the_first_outcome_and_changes_nothing() {
        let (mut ledger, task_id) = ledger_with_task();
        let first = ledger
            .claim(
                ctx(member("seat_11"), "same-key", at(&ledger, &task_id), 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        let before = ledger.clone();
        let again = ledger
            .claim(
                ctx(member("seat_11"), "same-key", Some(1), 25),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("retry");
        assert_eq!(first, again, "a retry replays its original outcome");
        assert_eq!(before, ledger, "and leaves no second mark");
        assert_eq!(ledger.task(&task_id).expect("task").attempts, 1);

        // The same key for a different command is a conflict.
        assert_eq!(
            ledger.claim(
                ctx(member("seat_11"), "same-key", Some(1), 26),
                &task_id,
                "agent_other".into(),
                DEFAULT_LEASE_MS
            ),
            Err(TaskError::IdempotencyConflict {
                key: "same-key".into()
            })
        );
    }

    #[test]
    fn a_retried_advance_does_not_move_the_artifact_twice() {
        let mut clock = 100;
        let (mut ledger, task_id) = ledger_with_task();
        drive_to_verified(&mut ledger, &task_id, "seat_11", "seat_10", 1, &mut clock);

        ledger
            .advance_artifact(ctx(host(), "inv", None, 40), "protocol", "sha256:v2".into())
            .expect("advance");
        let after_first = ledger.clone();
        ledger
            .advance_artifact(ctx(host(), "inv", None, 41), "protocol", "sha256:v2".into())
            .expect("retry");
        assert_eq!(after_first, ledger, "a retry must change nothing");
        assert_eq!(ledger.artifact("protocol").expect("artifact").version, 2);

        let mut restored = TaskLedger::replay(ledger.journal()).expect("replay");
        restored
            .advance_artifact(ctx(host(), "inv", None, 42), "protocol", "sha256:v2".into())
            .expect("retry after replay");
        assert_eq!(
            restored.artifact("protocol").expect("artifact").version,
            2,
            "a rebuilt ledger must refuse the same command too"
        );
    }

    #[test]
    fn a_restart_releases_leases_but_keeps_the_work() {
        let mut clock = 100;
        let (mut ledger, task_id) = ledger_with_task();
        let done = ledger
            .create(
                ctx(host(), "k-done", None, 11),
                "room_3".into(),
                "evt_2b".into(),
                "Finished".into(),
                "seat_11".into(),
                Vec::new(),
                None,
            )
            .expect("create")
            .task_id;
        drive_to_verified(&mut ledger, &done, "seat_11", "seat_10", 1, &mut clock);
        ledger
            .claim(
                ctx(member("seat_11"), "k1", at(&ledger, &task_id), 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        let epoch = epoch_of(&ledger, &task_id);
        ledger
            .start(
                ctx(member("seat_11"), "k2", at(&ledger, &task_id), 21),
                &task_id,
                epoch,
            )
            .expect("start");

        let released = ledger.release_leases_for_cold_restore(500);
        assert_eq!(released.len(), 1, "only the held lease is released");
        let task = ledger.task(&task_id).expect("task");
        assert!(task.lease.is_none());
        assert_eq!(task.state, TaskState::Ready, "the work returns to the pool");
        assert_eq!(task.attempts, 1, "the attempt still happened");
        assert_eq!(
            ledger.task(&done).expect("task").state,
            TaskState::Verified,
            "a verdict describes work, not a process, and survives"
        );

        let after_first = ledger.clone();
        assert!(ledger.release_leases_for_cold_restore(600).is_empty());
        assert_eq!(after_first, ledger);
        assert_eq!(
            TaskLedger::replay(ledger.journal()).expect("replay"),
            ledger,
            "the release is part of history, not a silent repair"
        );
    }

    #[test]
    fn proposing_a_decision_is_not_ratifying_it() {
        let mut ledger = TaskLedger::default();
        let proposed = ledger
            .propose_decision(
                ctx(member("seat_f"), "d1", None, 10),
                "room_3".into(),
                "Stale epochs".into(),
                "Keep StaleEpoch as an explicit, durable rejection".into(),
                "A refused submission must not be silently absorbed".into(),
                None,
                evidence(None),
            )
            .expect("propose");
        let decision_id = proposed.decision_id.clone();
        let decision = ledger.decision(&decision_id).expect("decision");
        assert_eq!(decision.status, DecisionStatus::Proposed);
        assert_eq!(decision.revision, 1);
        assert!(decision.accepted_by.is_none());

        // The proposer cannot ratify its own decision.
        assert_eq!(
            ledger.accept_decision(ctx(member("seat_f"), "d2", Some(1), 11), &decision_id),
            Err(TaskError::NotAuthorized)
        );
        // Nor may a stale writer.
        assert_eq!(
            ledger.accept_decision(ctx(host(), "d3", Some(9), 12), &decision_id),
            Err(TaskError::RevisionConflict {
                expected: 9,
                actual: 1
            })
        );
        ledger
            .accept_decision(ctx(host(), "d4", Some(1), 13), &decision_id)
            .expect("accept");
        let decision = ledger.decision(&decision_id).expect("decision");
        assert_eq!(decision.status, DecisionStatus::Accepted);
        assert_eq!(decision.accepted_by, Some(host()));
        // A settled decision does not change again.
        assert_eq!(
            ledger.accept_decision(ctx(host(), "d5", Some(2), 14), &decision_id),
            Err(TaskError::DecisionSettled)
        );
        assert_eq!(
            TaskLedger::replay(ledger.journal()).expect("replay"),
            ledger
        );
    }

    #[test]
    fn a_ratified_replacement_retires_what_it_replaced() {
        let mut ledger = TaskLedger::default();
        let first = ledger
            .propose_decision(
                ctx(member("seat_f"), "d1", None, 10),
                "room_3".into(),
                "Rejections".into(),
                "Return an error only".into(),
                "Simplest".into(),
                None,
                Vec::new(),
            )
            .expect("propose")
            .decision_id;
        ledger
            .accept_decision(ctx(host(), "d2", Some(1), 11), &first)
            .expect("accept");

        let second = ledger
            .propose_decision(
                ctx(member("seat_f"), "d3", None, 12),
                "room_3".into(),
                "Rejections, revised".into(),
                "Record a durable receipt as well".into(),
                "A refusal has to be observable after the fact".into(),
                Some(first.clone()),
                Vec::new(),
            )
            .expect("propose replacement")
            .decision_id;
        // Until it is ratified, the old decision still stands.
        assert_eq!(
            ledger.decision(&first).expect("first").status,
            DecisionStatus::Accepted
        );
        ledger
            .accept_decision(ctx(host(), "d4", Some(1), 13), &second)
            .expect("accept replacement");

        let old = ledger.decision(&first).expect("first");
        assert_eq!(old.status, DecisionStatus::Superseded);
        assert_eq!(old.superseded_by.as_deref(), Some(second.as_str()));
        assert_eq!(
            ledger
                .decision(&second)
                .expect("second")
                .supersedes
                .as_deref(),
            Some(first.as_str()),
            "the chain is readable in both directions"
        );
        // Superseding it again is refused, and the history replays.
        assert_eq!(
            ledger.propose_decision(
                ctx(member("seat_f"), "d5", None, 14),
                "room_3".into(),
                "Third".into(),
                "Something else".into(),
                "Because".into(),
                Some(first.clone()),
                Vec::new(),
            ),
            Err(TaskError::DecisionSettled)
        );
        assert_eq!(
            TaskLedger::replay(ledger.journal()).expect("replay"),
            ledger
        );
    }

    #[test]
    fn the_store_boundary_applies_commands_and_reports_durable_refusals() {
        // The scheduler sees only this trait: commands in, results or
        // refusals out, with the receipt id it can cite.
        fn drive(
            store: &mut dyn TaskStore,
            ctx: CommandContext,
            mutation: TaskMutation,
        ) -> Result<CommandResult, CommandRejection> {
            // Storage cannot fail for an in-memory ledger, so this unwraps
            // the outer result and hands back the domain outcome.
            match store.apply(ctx, mutation).expect("no storage beneath") {
                CommittedOutcome::Applied(result) => Ok(result),
                CommittedOutcome::Rejected(rejection) => Err(rejection),
            }
        }

        let mut ledger = TaskLedger::default();
        let created = drive(
            &mut ledger,
            ctx(host(), "m1", None, 10),
            TaskMutation::Create {
                room_id: "room_3".into(),
                root_id: "evt_37".into(),
                title: "Boundary".into(),
                owner_member_id: "seat_11".into(),
                depends_on: Vec::new(),
                artifact_id: Some("protocol".into()),
            },
        )
        .expect("create");
        let task_id = created.events[0].task_id.clone();
        assert!(!created.is_noop());

        drive(
            &mut ledger,
            ctx(member("seat_11"), "m2", Some(1), 20),
            TaskMutation::Claim {
                task_id: task_id.clone(),
                instance_id: "agent_1".into(),
                lease_ms: 10,
            },
        )
        .expect("claim");
        let stale_epoch = epoch_of(&ledger, &task_id);
        drive(
            &mut ledger,
            ctx(member("seat_11"), "m3", Some(2), 100),
            TaskMutation::Claim {
                task_id: task_id.clone(),
                instance_id: "agent_2".into(),
                lease_ms: DEFAULT_LEASE_MS,
            },
        )
        .expect("reclaim");

        // A stale attempt: refused, and the refusal is durable.
        let rejection = drive(
            &mut ledger,
            ctx(member("seat_11"), "m4", Some(3), 110),
            TaskMutation::Submit {
                task_id: task_id.clone(),
                epoch: stale_epoch,
                artifact: claim_of(1),
                evidence: evidence(Some(1)),
            },
        )
        .expect_err("stale submit");
        assert_eq!(rejection.code(), "task_stale_epoch");
        assert!(rejection.is_durable());
        let receipt = rejection.receipt.expect("receipt");
        assert_eq!(
            TaskStore::receipt(&ledger, &receipt.receipt_id)
                .expect("receipt is readable through the boundary")
                .idempotency_key,
            "m4"
        );

        // An unauthorized caller is refused with nothing written.
        let before = ledger.journal().len();
        let live_epoch = epoch_of(&ledger, &task_id);
        let rejection = drive(
            &mut ledger,
            ctx(member("stranger"), "m5", Some(3), 111),
            TaskMutation::Submit {
                task_id: task_id.clone(),
                epoch: live_epoch,
                artifact: claim_of(1),
                evidence: evidence(Some(1)),
            },
        )
        .expect_err("unauthorized");
        assert_eq!(rejection.code(), "task_not_authorized");
        assert!(!rejection.is_durable());
        assert_eq!(ledger.journal().len(), before, "nothing was written");

        // Reads used by a scheduler go through the trait too.
        assert!(TaskStore::ready_tasks(&ledger, 20).is_empty());
        assert_eq!(
            TaskStore::task(&ledger, &task_id).map(|task| task.state),
            Some(TaskState::Leased)
        );
        assert!(TaskStore::artifact(&ledger, "protocol").is_none());
    }

    /// A sink that refuses to write, to prove nothing is published without a
    /// durable record.
    #[derive(Debug, Clone, Default)]
    struct FailingSink {
        calls: usize,
    }

    impl JournalSink for FailingSink {
        fn persist(&mut self, _delta: &[JournalEntry]) -> Result<(), StoreError> {
            self.calls += 1;
            Err(StoreError::Persistence {
                detail: "disk full".into(),
            })
        }
    }

    #[derive(Debug, Clone, Default)]
    struct RecordingSink {
        committed: Vec<JournalEntry>,
    }

    impl JournalSink for RecordingSink {
        fn persist(&mut self, delta: &[JournalEntry]) -> Result<(), StoreError> {
            self.committed.extend_from_slice(delta);
            Ok(())
        }
    }

    #[test]
    fn a_storage_failure_publishes_nothing() {
        // Neither an accepted command nor a durable refusal may reach a
        // caller before its record exists.
        let (ledger, task_id) = ledger_with_task();
        let baseline = ledger.clone();
        let mut store = DurableStore::new(ledger, FailingSink::default());

        let error = store
            .apply(
                ctx(member("seat_11"), "s1", Some(1), 20),
                TaskMutation::Claim {
                    task_id: task_id.clone(),
                    instance_id: "agent_1".into(),
                    lease_ms: DEFAULT_LEASE_MS,
                },
            )
            .expect_err("storage refused");
        assert_eq!(error.code(), "store_persistence_failed");
        assert_eq!(
            store.ledger(),
            &baseline,
            "a failed commit leaves the live projection untouched"
        );

        // The same holds for a command whose outcome is a durable rejection:
        // the receipt is part of the delta, so it is not published either.
        let stale = store.apply(
            ctx(member("seat_11"), "s2", Some(99), 21),
            TaskMutation::Block {
                task_id: task_id.clone(),
                epoch: None,
                reason: "stale".into(),
            },
        );
        assert!(stale.is_err(), "storage failure outranks the domain answer");
        assert_eq!(store.ledger(), &baseline);
        assert_eq!(store.ledger().rejections().count(), 0);

        // With a working sink the same command commits, and what was
        // persisted is exactly what the projection gained.
        let mut store = DurableStore::new(baseline.clone(), RecordingSink::default());
        let outcome = store
            .apply(
                ctx(member("seat_11"), "s1", Some(1), 20),
                TaskMutation::Claim {
                    task_id: task_id.clone(),
                    instance_id: "agent_1".into(),
                    lease_ms: DEFAULT_LEASE_MS,
                },
            )
            .expect("committed");
        assert!(matches!(outcome, CommittedOutcome::Applied(_)));
        let (ledger, sink) = store.into_parts();
        assert_eq!(
            sink.committed.len(),
            ledger.journal().len() - baseline.journal().len()
        );
        // The delta is a delta: prior history plus what was committed is the
        // whole journal, and replaying that rebuilds the published ledger.
        let mut whole = baseline.journal().to_vec();
        whole.extend(sink.committed.iter().cloned());
        assert_eq!(whole, ledger.journal());
        assert_eq!(
            TaskLedger::replay(&whole).expect("replay the full journal"),
            ledger
        );
    }

    #[test]
    fn a_multi_event_command_returns_the_same_batch_on_retry_and_after_replay() {
        // Child events used to be keyed separately, so a retry replayed only
        // the parent and a caller could not tell a partial result from a
        // complete one.
        let mut clock = 100;
        let (mut ledger, base) = ledger_with_task();
        let middle = ledger
            .create(
                ctx(host(), "k-mid", None, 11),
                "room_3".into(),
                "evt_2b".into(),
                "Scheduler".into(),
                "seat_10".into(),
                vec![base.clone()],
                None,
            )
            .expect("create")
            .task_id;
        drive_to_verified(&mut ledger, &base, "seat_11", "seat_10", 1, &mut clock);
        drive_to_verified(&mut ledger, &middle, "seat_10", "seat_11", 1, &mut clock);

        // Artifact cascade: parent advance plus every invalidation.
        let first = ledger
            .advance_artifact(
                ctx(host(), "adv", None, 300),
                "protocol",
                "sha256:v2".into(),
            )
            .expect("advance");
        assert!(first.len() >= 2, "a real fan-out, not a single event");
        let retry = ledger
            .advance_artifact(
                ctx(host(), "adv", None, 301),
                "protocol",
                "sha256:v2".into(),
            )
            .expect("retry");
        assert_eq!(first, retry, "an exact retry returns the whole batch");
        let restored = TaskLedger::replay(ledger.journal()).expect("replay");
        let mut restored = restored;
        let after_replay = restored
            .advance_artifact(
                ctx(host(), "adv", None, 302),
                "protocol",
                "sha256:v2".into(),
            )
            .expect("retry after replay");
        assert_eq!(first, after_replay, "and so does a retry after a restart");

        // Dependency cascade: the added edge plus withdrawn verdicts.
        let top = ledger
            .create(
                ctx(host(), "k-top", None, 310),
                "room_3".into(),
                "evt_2b".into(),
                "Integration".into(),
                "seat_10".into(),
                Vec::new(),
                None,
            )
            .expect("create")
            .task_id;
        drive_to_verified_against(
            &mut ledger,
            &top,
            "seat_10",
            "seat_11",
            "integration",
            "sha256:int",
            1,
            &mut clock,
        );
        // The exact command, resent verbatim: a refreshed revision would be a
        // different command.
        let dep_revision = at(&ledger, &top);
        let dep_first = ledger
            .add_dependency(ctx(host(), "dep", dep_revision, 320), &top, &base)
            .expect("add dependency");
        assert!(dep_first.len() >= 2, "the edge plus the withdrawn verdict");
        let dep_retry = ledger
            .add_dependency(ctx(host(), "dep", dep_revision, 321), &top, &base)
            .expect("retry");
        assert_eq!(dep_first, dep_retry);
        let mut restored = TaskLedger::replay(ledger.journal()).expect("replay");
        assert_eq!(
            restored
                .add_dependency(ctx(host(), "dep", dep_revision, 322), &top, &base)
                .expect("retry after replay"),
            dep_first
        );
    }

    #[test]
    fn accepting_a_replacement_returns_both_events_every_time() {
        let mut ledger = TaskLedger::default();
        let first = ledger
            .propose_decision(
                ctx(member("seat_f"), "d1", None, 10),
                "room_3".into(),
                "Rejections".into(),
                "Return an error only".into(),
                "Simplest".into(),
                None,
                Vec::new(),
            )
            .expect("propose")
            .decision_id;
        ledger
            .accept_decision(ctx(host(), "d2", Some(1), 11), &first)
            .expect("accept");
        let second = ledger
            .propose_decision(
                ctx(member("seat_f"), "d3", None, 12),
                "room_3".into(),
                "Rejections, revised".into(),
                "Record a durable receipt too".into(),
                "A refusal must be observable".into(),
                Some(first.clone()),
                Vec::new(),
            )
            .expect("propose replacement")
            .decision_id;

        let batch = ledger
            .accept_decision(ctx(host(), "d4", Some(1), 13), &second)
            .expect("accept replacement");
        assert_eq!(batch.len(), 2, "the acceptance and the supersession");
        assert!(matches!(batch[0].kind, DecisionEventKind::Accepted));
        assert!(matches!(
            batch[1].kind,
            DecisionEventKind::Superseded { .. }
        ));
        assert_eq!(
            batch[1].caused_by.as_deref(),
            Some(batch[0].event_id.as_str()),
            "the supersession names the acceptance that caused it"
        );

        let retry = ledger
            .accept_decision(ctx(host(), "d4", Some(1), 14), &second)
            .expect("retry");
        assert_eq!(batch, retry, "the same ordered batch, not a fragment");
        let mut restored = TaskLedger::replay(ledger.journal()).expect("replay");
        assert_eq!(
            restored
                .accept_decision(ctx(host(), "d4", Some(1), 15), &second)
                .expect("retry after replay"),
            batch
        );
    }

    #[test]
    fn two_replacements_cannot_both_retire_the_same_decision() {
        let mut ledger = TaskLedger::default();
        let original = ledger
            .propose_decision(
                ctx(member("seat_f"), "d1", None, 10),
                "room_3".into(),
                "Original".into(),
                "Do it this way".into(),
                "Because".into(),
                None,
                Vec::new(),
            )
            .expect("propose")
            .decision_id;
        ledger
            .accept_decision(ctx(host(), "d2", Some(1), 11), &original)
            .expect("accept");

        // Two competing replacements, both naming the same predecessor.
        let left = ledger
            .propose_decision(
                ctx(member("seat_f"), "d3", None, 12),
                "room_3".into(),
                "Left".into(),
                "Do it the left way".into(),
                "Because".into(),
                Some(original.clone()),
                Vec::new(),
            )
            .expect("propose left")
            .decision_id;
        let right = ledger
            .propose_decision(
                ctx(member("seat_10"), "d4", None, 13),
                "room_3".into(),
                "Right".into(),
                "Do it the right way".into(),
                "Because".into(),
                Some(original.clone()),
                Vec::new(),
            )
            .expect("propose right")
            .decision_id;

        ledger
            .accept_decision(ctx(host(), "d5", Some(1), 14), &left)
            .expect("accept left");
        let before = ledger.clone();
        assert_eq!(
            ledger.accept_decision(ctx(host(), "d6", Some(1), 15), &right),
            Err(TaskError::DecisionSettled),
            "the predecessor is no longer Accepted, so the race is lost"
        );
        assert_eq!(before, ledger, "and the loser changes nothing");
        assert_eq!(
            ledger
                .decision(&original)
                .expect("original")
                .superseded_by
                .as_deref(),
            Some(left.as_str()),
            "the supersession chain still names the winner"
        );
    }

    #[test]
    fn a_replacement_may_not_reach_into_another_room() {
        let mut ledger = TaskLedger::default();
        let elsewhere = ledger
            .propose_decision(
                ctx(member("seat_f"), "d1", None, 10),
                "room_9".into(),
                "Other room".into(),
                "Their decision".into(),
                "Theirs".into(),
                None,
                Vec::new(),
            )
            .expect("propose")
            .decision_id;
        ledger
            .accept_decision(ctx(host(), "d2", Some(1), 11), &elsewhere)
            .expect("accept");
        let intruder = ledger
            .propose_decision(
                ctx(member("seat_f"), "d3", None, 12),
                "room_3".into(),
                "Reach".into(),
                "Replace another room's decision".into(),
                "Because".into(),
                Some(elsewhere.clone()),
                Vec::new(),
            )
            .expect("propose")
            .decision_id;
        assert_eq!(
            ledger.accept_decision(ctx(host(), "d4", Some(1), 13), &intruder),
            Err(TaskError::CrossRoomDependency {
                task_id: elsewhere.clone()
            })
        );
        assert_eq!(
            ledger.decision(&elsewhere).expect("other").status,
            DecisionStatus::Accepted
        );
    }

    #[test]
    fn bounded_reads_page_deterministically() {
        let mut clock = 100;
        let (mut ledger, first) = ledger_with_task();
        for index in 0..4 {
            ledger
                .create(
                    ctx(host(), &format!("b{index}"), None, 20 + index),
                    "room_3".into(),
                    "evt_2b".into(),
                    format!("Task {index}"),
                    "seat_11".into(),
                    Vec::new(),
                    None,
                )
                .expect("create");
        }
        drive_to_verified(&mut ledger, &first, "seat_11", "seat_10", 1, &mut clock);

        // The journal pages in sequence order and stops where it says it will.
        let mut cursor = None;
        let mut seen = Vec::new();
        loop {
            let page = TaskStore::journal_page(&ledger, cursor, 3);
            assert!(page.entries.len() <= 3, "the limit is honoured");
            seen.extend(page.entries.iter().map(JournalEntry::sequence));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let expected: Vec<u64> = ledger
            .journal()
            .iter()
            .map(JournalEntry::sequence)
            .collect();
        assert_eq!(
            seen, expected,
            "paging sees the journal exactly once, in order"
        );
        assert!(seen.windows(2).all(|pair| pair[0] < pair[1]));

        // Ready work is capped and ordered the same way every time.
        let page = TaskStore::ready_page(&ledger, 1_000, 2);
        assert_eq!(page.len(), 2);
        let again: Vec<&str> = TaskStore::ready_page(&ledger, 1_000, 2)
            .iter()
            .map(|task| task.task_id.as_str())
            .collect();
        let once: Vec<&str> = page.iter().map(|task| task.task_id.as_str()).collect();
        assert_eq!(once, again, "the same ledger yields the same page");
    }

    #[test]
    fn commands_and_context_survive_the_wire() {
        // A queued or forwarded command has to round-trip unchanged.
        let mutation = TaskMutation::Submit {
            task_id: "task_1".into(),
            epoch: 3,
            artifact: claim_of(2),
            evidence: evidence(Some(2)),
        };
        let ctx = ctx(member("seat_11"), "w1", Some(7), 42);
        let encoded = serde_json::to_string(&(&ctx, &mutation)).expect("encode");
        let (decoded_ctx, decoded): (CommandContext, TaskMutation) =
            serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded_ctx, ctx);
        assert_eq!(decoded, mutation);
    }

    /// The protocol-replay harness: folding the journal must reproduce every
    /// projection, or the journal is not a faithful record of what happened.
    #[test]
    fn replaying_the_journal_reproduces_the_ledger_exactly() {
        let mut clock = 200;
        let (mut ledger, task_id) = ledger_with_task();
        let review = ledger
            .create(
                ctx(host(), "s-create2", None, 11),
                "room_3".into(),
                "evt_2b".into(),
                "Review".into(),
                "seat_10".into(),
                vec![task_id.clone()],
                Some("protocol".into()),
            )
            .expect("create")
            .task_id;
        ledger
            .claim(
                ctx(member("seat_11"), "s1", at(&ledger, &task_id), 20),
                &task_id,
                "agent_1".into(),
                DEFAULT_LEASE_MS,
            )
            .expect("claim");
        let epoch = epoch_of(&ledger, &task_id);
        ledger
            .start(
                ctx(member("seat_11"), "s2", at(&ledger, &task_id), 21),
                &task_id,
                epoch,
            )
            .expect("start");
        // A refusal in the middle of real work.
        let _ = ledger.submit(
            ctx(member("seat_11"), "s-stale", at(&ledger, &task_id), 22),
            &task_id,
            epoch + 7,
            claim_of(1),
            evidence(Some(1)),
        );
        ledger
            .block(
                ctx(member("seat_11"), "s3", at(&ledger, &task_id), 23),
                &task_id,
                Some(epoch),
                "question".into(),
            )
            .expect("block");
        let blocker = ledger
            .task(&task_id)
            .expect("task")
            .blockers
            .first()
            .expect("blocker")
            .blocker_id
            .clone();
        ledger
            .unblock(
                ctx(host(), "s4", at(&ledger, &task_id), 24),
                &task_id,
                &blocker,
            )
            .expect("unblock");
        drive_to_verified(&mut ledger, &task_id, "seat_11", "seat_10", 1, &mut clock);
        ledger
            .advance_artifact(ctx(host(), "s8", None, 250), "protocol", "sha256:v2".into())
            .expect("advance");
        ledger
            .fail(
                ctx(host(), "s9", at(&ledger, &review), 251),
                &review,
                None,
                "abandoned".into(),
            )
            .expect("fail");

        let replayed = TaskLedger::replay(ledger.journal()).expect("replay");
        assert_eq!(
            replayed, ledger,
            "the journal must be a complete account of the ledger"
        );
        assert_eq!(
            TaskLedger::replay(ledger.journal()).expect("replay"),
            replayed,
            "replay is deterministic"
        );
        assert!(
            ledger.rejections().count() >= 1,
            "the refusal is in the journal"
        );
        replayed.assert_invariants().expect("invariants");
    }
}
