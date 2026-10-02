//! JSON API handlers for company rooms.
//!
//! Shared room state lives on the server and is reachable through these
//! methods, so the TUI is one client among several rather than the only way
//! to drive a room.

use crate::api::schema::{
    ResponseResult, RoomActorInfo, RoomAllowanceParams, RoomBudgetInfo, RoomCreateParams,
    RoomDeliveryAckParams, RoomEventInfo, RoomEventListParams, RoomInfo, RoomLifecycleParams,
    RoomLifecycleValue, RoomListParams, RoomMemberAddParams, RoomMemberBindParams,
    RoomMemberGrantParams, RoomMemberInfo, RoomMemberTargetParams, RoomMemoryPutParams,
    RoomMemorySearchParams, RoomMemoryTargetParams, RoomPostParams, RoomRecordHit, RoomRecordInfo,
    RoomRecordStatusValue, RoomTargetParams,
};
use crate::company::{
    Actor, CompanyError, KnowledgeRecord, RecordStatus, Room, RoomEvent, RoomLifecycle, RootBudget,
};

use super::{responses, App};

impl App {
    /// Resolves the acting principal.
    ///
    /// A caller pane is only an actor if it currently holds a seat in *this*
    /// room. A caller that asked to act as an agent and holds no seat is
    /// refused rather than quietly treated as the host: falling back would
    /// let any agent post under the operator's name and inherit the host's
    /// authority, including the right to wake the whole room. Absent a caller
    /// pane entirely, the actor is the authenticated host.
    fn room_actor(&mut self, room_id: &str, caller_pane_id: Option<&str>) -> Option<Actor> {
        let Some(caller_pane_id) = caller_pane_id else {
            return Some(Actor::Host);
        };
        let agent = self.agent_info_for_target(caller_pane_id).ok()?;
        let instance_id = self.ensure_collaboration_agent(&agent).instance_id;
        self.state.company.room(room_id).and_then(|room| {
            room.members
                .iter()
                .find(|member| member.bound_instance_id.as_deref() == Some(&instance_id))
                .map(|member| Actor::Member {
                    member_id: member.member_id.clone(),
                })
        })
    }

    pub(super) fn handle_room_create(&mut self, id: String, params: RoomCreateParams) -> String {
        match self.state.company.create_room(
            &Actor::Host,
            params.workspace_id,
            params.name,
            params.objective,
            current_unix_ms(),
        ) {
            Ok(room) => {
                self.schedule_session_save();
                responses::encode_success(
                    id,
                    ResponseResult::RoomInfo {
                        room: room_info(&room),
                    },
                )
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_list(&mut self, id: String, params: RoomListParams) -> String {
        let rooms = match params.workspace_id.as_deref() {
            Some(workspace_id) => self
                .state
                .company
                .rooms_in_workspace(workspace_id)
                .into_iter()
                .map(room_info)
                .collect(),
            None => self.state.company.rooms().iter().map(room_info).collect(),
        };
        responses::encode_success(id, ResponseResult::RoomList { rooms })
    }

    pub(super) fn handle_room_get(&mut self, id: String, params: RoomTargetParams) -> String {
        match self.state.company.room(&params.room_id) {
            Some(room) => responses::encode_success(
                id,
                ResponseResult::RoomInfo {
                    room: room_info(room),
                },
            ),
            None => encode_company_error(id, CompanyError::RoomNotFound),
        }
    }

    pub(super) fn handle_room_set_lifecycle(
        &mut self,
        id: String,
        params: RoomLifecycleParams,
    ) -> String {
        let lifecycle = match params.lifecycle {
            RoomLifecycleValue::Active => RoomLifecycle::Active,
            RoomLifecycleValue::Paused => RoomLifecycle::Paused,
            RoomLifecycleValue::Archived => RoomLifecycle::Archived,
        };
        match self
            .state
            .company
            .set_lifecycle(&Actor::Host, &params.room_id, lifecycle)
        {
            Ok(room) => {
                self.schedule_session_save();
                responses::encode_success(
                    id,
                    ResponseResult::RoomInfo {
                        room: room_info(&room),
                    },
                )
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_member_add(
        &mut self,
        id: String,
        params: RoomMemberAddParams,
    ) -> String {
        match self.state.company.add_member(
            &Actor::Host,
            &params.room_id,
            params.handle,
            params.role,
            params.orchestrator,
        ) {
            Ok(_) => {
                self.schedule_session_save();
                self.encode_room(id, &params.room_id)
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_member_remove(
        &mut self,
        id: String,
        params: RoomMemberTargetParams,
    ) -> String {
        match self
            .state
            .company
            .remove_member(&Actor::Host, &params.room_id, &params.member_id)
        {
            Ok(()) => {
                self.schedule_session_save();
                self.encode_room(id, &params.room_id)
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    /// Binds a live agent to a seat.
    ///
    /// The bound value is the collaboration incarnation, not the pane, so a
    /// pane reused by a different agent does not silently inherit the seat.
    pub(super) fn handle_room_member_bind(
        &mut self,
        id: String,
        params: RoomMemberBindParams,
    ) -> String {
        let instance_id = match params.target.as_deref() {
            Some(target) => match self.agent_info_for_target(target) {
                Ok(agent) => Some(self.ensure_collaboration_agent(&agent).instance_id),
                Err(err) => {
                    return responses::encode_error_body(id, self.agent_target_error_body(err))
                }
            },
            None => None,
        };
        match self
            .state
            .company
            .bind_member(&params.room_id, &params.member_id, instance_id)
        {
            Ok(()) => {
                self.schedule_session_save();
                self.encode_room(id, &params.room_id)
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_post(&mut self, id: String, params: RoomPostParams) -> String {
        let Some(actor) = self.room_actor(&params.room_id, params.caller_pane_id.as_deref()) else {
            return encode_company_error(id, CompanyError::NotSeated);
        };
        match self.state.company.post_idempotent(
            actor,
            &params.room_id,
            params.body,
            params.recipients,
            params.root_id,
            current_unix_ms(),
            params.client_nonce.as_deref(),
        ) {
            Ok(event) => {
                self.schedule_session_save();
                // Dispatch immediately so an addressed idle agent does not wait
                // for the next loop pass.
                let _ = self.dispatch_room_deliveries();
                responses::encode_success(
                    id,
                    ResponseResult::RoomEvent {
                        event: event_info(&event),
                    },
                )
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_ack(&mut self, id: String, params: RoomDeliveryAckParams) -> String {
        let agent = match self.agent_info_for_target(&params.caller_pane_id) {
            Ok(agent) => agent,
            Err(err) => return responses::encode_error_body(id, self.agent_target_error_body(err)),
        };
        let instance_id = self.ensure_collaboration_agent(&agent).instance_id;
        let member_id = self.state.company.room(&params.room_id).and_then(|room| {
            room.members
                .iter()
                .find(|member| member.bound_instance_id.as_deref() == Some(&instance_id))
                .map(|member| member.member_id.clone())
        });
        let Some(member_id) = member_id else {
            return encode_company_error(id, CompanyError::NotSeated);
        };
        match self.state.company.acknowledge_delivery(
            &params.room_id,
            &params.event_id,
            &member_id,
            &instance_id,
        ) {
            Ok(event) => {
                self.schedule_session_save();
                responses::encode_success(
                    id,
                    ResponseResult::RoomEvent {
                        event: event_info(&event),
                    },
                )
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_events(&mut self, id: String, params: RoomEventListParams) -> String {
        let Some(room) = self.state.company.room(&params.room_id) else {
            return encode_company_error(id, CompanyError::RoomNotFound);
        };
        let limit = params.limit.unwrap_or(50).clamp(1, 200) as usize;
        let events: Vec<RoomEventInfo> = room
            .events
            .iter()
            .filter(|event| event.sequence > params.after_sequence)
            .take(limit)
            .map(event_info)
            .collect();
        let latest_sequence = room.events.last().map(|e| e.sequence).unwrap_or(0);
        responses::encode_success(
            id,
            ResponseResult::RoomEventList {
                events,
                latest_sequence,
            },
        )
    }

    pub(super) fn handle_room_memory_put(
        &mut self,
        id: String,
        params: RoomMemoryPutParams,
    ) -> String {
        let Some(actor) = self.room_actor(&params.room_id, params.caller_pane_id.as_deref()) else {
            return encode_company_error(id, CompanyError::NotSeated);
        };
        match self.state.company.put_record(
            actor,
            &params.room_id,
            params.record_id,
            params.expected_revision,
            params.title,
            params.summary,
            params.body,
            params.tags,
            current_unix_ms(),
        ) {
            Ok(record) => {
                self.schedule_session_save();
                responses::encode_success(
                    id,
                    ResponseResult::RoomRecord {
                        record: record_info(&record),
                    },
                )
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_memory_search(
        &mut self,
        id: String,
        params: RoomMemorySearchParams,
    ) -> String {
        if self.state.company.room(&params.room_id).is_none() {
            return encode_company_error(id, CompanyError::RoomNotFound);
        }
        let limit = params.limit.unwrap_or(5) as usize;
        let hits: Vec<RoomRecordHit> = self
            .state
            .company
            .search_records(&params.room_id, &params.query, limit)
            .into_iter()
            .map(|hit| RoomRecordHit {
                record_id: hit.record_id,
                revision: hit.revision,
                title: hit.title,
                summary: hit.summary,
                status: record_status(hit.status),
            })
            .collect();
        responses::encode_success(id, ResponseResult::RoomRecordHits { hits })
    }

    pub(super) fn handle_room_memory_get(
        &mut self,
        id: String,
        params: RoomMemoryTargetParams,
    ) -> String {
        match self
            .state
            .company
            .record(&params.room_id, &params.record_id)
        {
            Some(record) => responses::encode_success(
                id,
                ResponseResult::RoomRecord {
                    record: record_info(record),
                },
            ),
            None => encode_company_error(id, CompanyError::RecordNotFound),
        }
    }

    /// Grants or revokes a seat's right to address the whole room.
    pub(super) fn handle_room_member_grant(
        &mut self,
        id: String,
        params: RoomMemberGrantParams,
    ) -> String {
        let Some(actor) = self.room_actor(&params.room_id, params.caller_pane_id.as_deref()) else {
            return encode_company_error(id, CompanyError::NotSeated);
        };
        match self.state.company.set_may_broadcast(
            &actor,
            &params.room_id,
            &params.member_id,
            params.may_broadcast,
        ) {
            Ok(_) => {
                self.schedule_session_save();
                self.encode_room(id, &params.room_id)
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    /// Deletes a room and everything in it.
    pub(super) fn handle_room_delete(&mut self, id: String, params: RoomTargetParams) -> String {
        match self
            .state
            .company
            .delete_room(&Actor::Host, &params.room_id)
        {
            Ok(()) => {
                self.schedule_session_save();
                responses::encode_success(id, ResponseResult::Ok {})
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_memory_delete(
        &mut self,
        id: String,
        params: RoomMemoryTargetParams,
    ) -> String {
        let Some(actor) = self.room_actor(&params.room_id, params.caller_pane_id.as_deref()) else {
            return encode_company_error(id, CompanyError::NotSeated);
        };
        match self
            .state
            .company
            .delete_record(&actor, &params.room_id, &params.record_id)
        {
            Ok(()) => {
                self.schedule_session_save();
                responses::encode_success(id, ResponseResult::Ok {})
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_memory_accept(
        &mut self,
        id: String,
        params: RoomMemoryTargetParams,
    ) -> String {
        let Some(actor) = self.room_actor(&params.room_id, params.caller_pane_id.as_deref()) else {
            return encode_company_error(id, CompanyError::NotSeated);
        };
        match self
            .state
            .company
            .accept_record(&actor, &params.room_id, &params.record_id)
        {
            Ok(record) => {
                self.schedule_session_save();
                responses::encode_success(
                    id,
                    ResponseResult::RoomRecord {
                        record: record_info(&record),
                    },
                )
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    pub(super) fn handle_room_allowance_extend(
        &mut self,
        id: String,
        params: RoomAllowanceParams,
    ) -> String {
        match self.state.company.extend_allowance(
            &Actor::Host,
            &params.room_id,
            &params.root_id,
            params.additional,
        ) {
            Ok(budget) => {
                self.schedule_session_save();
                responses::encode_success(
                    id,
                    ResponseResult::RoomBudget {
                        budget: budget_info(&budget),
                    },
                )
            }
            Err(err) => encode_company_error(id, err),
        }
    }

    fn encode_room(&mut self, id: String, room_id: &str) -> String {
        match self.state.company.room(room_id) {
            Some(room) => responses::encode_success(
                id,
                ResponseResult::RoomInfo {
                    room: room_info(room),
                },
            ),
            None => encode_company_error(id, CompanyError::RoomNotFound),
        }
    }
}

impl crate::app::App {
    pub(super) fn handle_room_tasks(
        &mut self,
        id: String,
        params: crate::api::schema::RoomTaskListParams,
    ) -> String {
        if self.state.company.room(&params.room_id).is_none() {
            return encode_company_error(id, CompanyError::RoomNotFound);
        }
        let now = current_unix_ms();
        let ledger = self.state.company.tasks();
        let tasks: Vec<_> = if params.ready_only {
            ledger
                .ready_tasks(now)
                .into_iter()
                .filter(|task| task.room_id == params.room_id)
                .map(task_info)
                .collect()
        } else {
            ledger
                .tasks()
                .iter()
                .filter(|task| task.room_id == params.room_id)
                .map(task_info)
                .collect()
        };
        responses::encode_success(id, ResponseResult::RoomTaskList { tasks })
    }

    pub(super) fn handle_room_task_create(
        &mut self,
        id: String,
        params: crate::api::schema::RoomTaskCreateParams,
    ) -> String {
        use crate::company::tasks::TaskMutation;
        let Some(actor) = self.room_actor(&params.room_id, params.caller_pane_id.as_deref()) else {
            return encode_company_error(id, CompanyError::NotSeated);
        };
        // The owner must be a seat in this room: work answerable by nobody, or
        // by a seat in another room, cannot be scheduled or charged.
        let owner_known = self
            .state
            .company
            .room(&params.room_id)
            .is_some_and(|room| room.member(&params.owner_member_id).is_some());
        if !owner_known {
            return encode_company_error(id, CompanyError::MemberNotFound);
        }
        // Without a stated objective the task opens its own, so its activations
        // are charged somewhere rather than to whatever was last discussed.
        let root_id = params
            .root_id
            .unwrap_or_else(|| format!("task-root:{}", params.owner_member_id));
        let mutation = TaskMutation::Create {
            room_id: params.room_id.clone(),
            root_id,
            title: params.title,
            owner_member_id: params.owner_member_id,
            depends_on: params.depends_on,
            artifact_id: None,
        };
        let ctx = self.task_context(&actor, params.client_nonce, None, None);
        self.commit_task_command(id, ctx, mutation)
    }

    pub(super) fn handle_room_task_command(
        &mut self,
        id: String,
        params: crate::api::schema::RoomTaskCommandParams,
        kind: TaskCommandKind,
    ) -> String {
        use crate::company::tasks::TaskMutation;
        let Some(actor) = self.room_actor(&params.room_id, params.caller_pane_id.as_deref()) else {
            return encode_company_error(id, CompanyError::NotSeated);
        };
        let evidence = evidence_from_api(params.evidence);
        let mutation = match kind {
            TaskCommandKind::Claim => {
                // A claim names the process that will do the work, so only a
                // seated agent can make one; the host has no incarnation.
                let Some(pane_id) = params.caller_pane_id.as_deref() else {
                    return encode_company_error(id, CompanyError::NotSeated);
                };
                let Ok(agent) = self.agent_info_for_target(pane_id) else {
                    return encode_company_error(id, CompanyError::NotSeated);
                };
                let instance_id = self.ensure_collaboration_agent(&agent).instance_id;
                TaskMutation::Claim {
                    task_id: params.task_id.clone(),
                    instance_id,
                    lease_ms: params.lease_ms.unwrap_or(DEFAULT_TASK_LEASE_MS),
                }
            }
            TaskCommandKind::Start => TaskMutation::Start {
                task_id: params.task_id.clone(),
                epoch: params.epoch.unwrap_or_default(),
            },
            TaskCommandKind::Submit => TaskMutation::Submit {
                task_id: params.task_id.clone(),
                epoch: params.epoch.unwrap_or_default(),
                artifact: None,
                evidence,
            },
            TaskCommandKind::Verify => TaskMutation::Verify {
                task_id: params.task_id.clone(),
                verifier: match &actor {
                    Actor::Host => crate::company::HOST_HANDLE.to_string(),
                    Actor::Member { member_id } => member_id.clone(),
                },
                rule_version: 1,
                acceptance: evidence,
            },
            TaskCommandKind::Fail => TaskMutation::Fail {
                task_id: params.task_id.clone(),
                epoch: params.epoch,
                reason: params.reason.unwrap_or_else(|| "unspecified".into()),
            },
        };
        let ctx = self.task_context(
            &actor,
            params.client_nonce,
            Some(params.expected_revision),
            Some(params.task_id),
        );
        self.commit_task_command(id, ctx, mutation)
    }

    /// Builds the command envelope every task mutation carries.
    fn task_context(
        &self,
        actor: &Actor,
        client_nonce: Option<String>,
        expected_revision: Option<u64>,
        task_id: Option<String>,
    ) -> crate::company::tasks::CommandContext {
        use crate::company::tasks::{CommandContext, TaskActor};
        let now = current_unix_ms();
        CommandContext {
            actor: match actor {
                Actor::Host => TaskActor::Host,
                Actor::Member { member_id } => TaskActor::Member {
                    member_id: member_id.clone(),
                },
            },
            // A caller that offers no key still gets one, so a command is
            // never rejected for lacking it; only a caller-supplied key makes
            // a retry recognisable.
            idempotency_key: client_nonce
                .unwrap_or_else(|| format!("auto:{}:{}", task_id.as_deref().unwrap_or("new"), now)),
            expected_revision,
            caused_by: None,
            now_unix_ms: now,
        }
    }

    /// Commits one mutation and answers with the task it touched.
    ///
    /// A rejection is an error to the caller but still a durable fact: the
    /// journal already holds its receipt by the time this returns.
    fn commit_task_command(
        &mut self,
        id: String,
        ctx: crate::company::tasks::CommandContext,
        mutation: crate::company::tasks::TaskMutation,
    ) -> String {
        use crate::company::tasks::CommittedOutcome;
        let task_id = mutation_task_id(&mutation);
        match self.apply_task_mutation(ctx, mutation) {
            Ok(CommittedOutcome::Applied(result)) => {
                // A create only learns its task id from the event it produced,
                // so the answer is read back from the command's own result
                // rather than from what the caller asked for.
                let task_id =
                    task_id.or_else(|| result.events.first().map(|event| event.task_id.clone()));
                let task = task_id
                    .as_deref()
                    .and_then(|task_id| self.state.company.tasks().task(task_id))
                    .map(task_info);
                match task {
                    Some(task) => responses::encode_success(id, ResponseResult::RoomTask { task }),
                    None => responses::encode_error(
                        id,
                        "task_not_found",
                        "the command applied but its task could not be read back".to_string(),
                    ),
                }
            }
            Ok(CommittedOutcome::Rejected(rejection)) => {
                responses::encode_error(id, rejection.code(), rejection.error.message())
            }
            Err(err) => responses::encode_error(id, err.code(), err.message()),
        }
    }
}

/// How long a claim holds by default: long enough for a real attempt, short
/// enough that a dead worker frees the task without an operator.
const DEFAULT_TASK_LEASE_MS: u64 = 10 * 60 * 1000;

/// The task a mutation acts on, so the caller can be answered with it.
fn mutation_task_id(mutation: &crate::company::tasks::TaskMutation) -> Option<String> {
    use crate::company::tasks::TaskMutation;
    match mutation {
        TaskMutation::Claim { task_id, .. }
        | TaskMutation::Start { task_id, .. }
        | TaskMutation::Submit { task_id, .. }
        | TaskMutation::Verify { task_id, .. }
        | TaskMutation::Fail { task_id, .. }
        | TaskMutation::Block { task_id, .. }
        | TaskMutation::Unblock { task_id, .. }
        | TaskMutation::AddDependency { task_id, .. } => Some(task_id.clone()),
        // A create names its task only after the ledger assigns an id.
        TaskMutation::Create { .. }
        | TaskMutation::AdvanceArtifact { .. }
        | TaskMutation::ProposeDecision { .. }
        | TaskMutation::AcceptDecision { .. } => None,
    }
}

/// Which lifecycle command an API request stands for.
///
/// One enum rather than seven near-identical handlers: the difference between
/// them is which mutation they build, not how they are authorised or answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskCommandKind {
    Claim,
    Start,
    Submit,
    Verify,
    Fail,
}

fn evidence_from_api(
    evidence: Vec<crate::api::schema::RoomEvidenceRefValue>,
) -> Vec<crate::company::tasks::EvidenceRef> {
    use crate::api::schema::RoomEvidenceKindValue;
    use crate::company::tasks::{EvidenceKind, EvidenceRef};
    evidence
        .into_iter()
        .map(|item| EvidenceRef {
            kind: match item.kind {
                RoomEvidenceKindValue::RawPty => EvidenceKind::RawPty,
                RoomEvidenceKindValue::Transcript => EvidenceKind::Transcript,
                RoomEvidenceKindValue::Command => EvidenceKind::Command,
                RoomEvidenceKindValue::File => EvidenceKind::File,
            },
            handle: item.handle,
            digest: item.digest,
            artifact_version: item.artifact_version,
        })
        .collect()
}

pub(crate) fn task_info(task: &crate::company::tasks::Task) -> crate::api::schema::RoomTaskInfo {
    use crate::api::schema::{RoomTaskInfo, RoomTaskStateValue};
    use crate::company::tasks::TaskState;
    RoomTaskInfo {
        task_id: task.task_id.clone(),
        room_id: task.room_id.clone(),
        title: task.title.clone(),
        root_id: task.root_id.clone(),
        owner_member_id: task.owner_member_id.clone(),
        state: match task.state {
            TaskState::Ready => RoomTaskStateValue::Ready,
            TaskState::Leased => RoomTaskStateValue::Leased,
            TaskState::Running => RoomTaskStateValue::Running,
            TaskState::Blocked => RoomTaskStateValue::Blocked,
            TaskState::Submitted => RoomTaskStateValue::Submitted,
            TaskState::Verified => RoomTaskStateValue::Verified,
            TaskState::Failed => RoomTaskStateValue::Failed,
            TaskState::Cancelled => RoomTaskStateValue::Cancelled,
        },
        revision: task.revision,
        depends_on: task.depends_on.clone(),
        // Only open blockers: a cleared one is history, and showing it would
        // make a running task look stuck.
        blocked_reasons: task
            .open_blockers()
            .map(|blocker| blocker.reason.clone())
            .collect(),
        lease_holder_instance_id: task.lease.as_ref().map(|lease| lease.instance_id.clone()),
        lease_expires_at_unix_ms: task.lease.as_ref().map(|lease| lease.expires_at_unix_ms),
        epoch: task.lease.as_ref().map(|lease| lease.epoch),
        attempts: task.attempts,
        verified: task.verification.is_some(),
    }
}

fn encode_company_error(id: String, err: CompanyError) -> String {
    responses::encode_error(id, err.code(), err.message())
}

pub(crate) fn room_info(room: &Room) -> RoomInfo {
    RoomInfo {
        room_id: room.room_id.clone(),
        workspace_id: room.workspace_id.clone(),
        name: room.name.clone(),
        objective: room.objective.clone(),
        lifecycle: match room.lifecycle {
            RoomLifecycle::Active => RoomLifecycleValue::Active,
            RoomLifecycle::Paused => RoomLifecycleValue::Paused,
            RoomLifecycle::Archived => RoomLifecycleValue::Archived,
        },
        members: room
            .members
            .iter()
            .map(|member| RoomMemberInfo {
                member_id: member.member_id.clone(),
                handle: member.handle.clone(),
                role: member.role.clone(),
                orchestrator: member.is_orchestrator,
                may_broadcast: member.may_broadcast,
                bound_instance_id: member.bound_instance_id.clone(),
            })
            .collect(),
        event_count: room.events.len() as u64,
        record_count: room.records.len() as u64,
        revision: room.revision,
    }
}

fn event_info(event: &RoomEvent) -> RoomEventInfo {
    RoomEventInfo {
        event_id: event.event_id.clone(),
        sequence: event.sequence,
        actor: match &event.actor {
            Actor::Host => RoomActorInfo::Host,
            Actor::Member { member_id } => RoomActorInfo::Member {
                member_id: member_id.clone(),
            },
        },
        body: event.body.clone(),
        recipients: event.recipients.clone(),
        root_id: event.root_id.clone(),
        created_at_unix_ms: event.created_at_unix_ms,
    }
}

fn record_status(status: RecordStatus) -> RoomRecordStatusValue {
    match status {
        RecordStatus::Proposed => RoomRecordStatusValue::Proposed,
        RecordStatus::Accepted => RoomRecordStatusValue::Accepted,
        RecordStatus::Disputed => RoomRecordStatusValue::Disputed,
        RecordStatus::Stale => RoomRecordStatusValue::Stale,
        RecordStatus::Superseded => RoomRecordStatusValue::Superseded,
    }
}

fn record_info(record: &KnowledgeRecord) -> RoomRecordInfo {
    RoomRecordInfo {
        record_id: record.record_id.clone(),
        revision: record.revision,
        title: record.title.clone(),
        summary: record.summary.clone(),
        body: record.body.clone(),
        tags: record.tags.clone(),
        status: record_status(record.status),
        updated_at_unix_ms: record.updated_at_unix_ms,
    }
}

fn budget_info(budget: &RootBudget) -> RoomBudgetInfo {
    RoomBudgetInfo {
        root_id: budget.root_id.clone(),
        allowance: budget.allowance,
        used: budget.used,
        remaining: budget.remaining(),
        paused: budget.paused,
    }
}

fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{Method, Request, RoomPostParams};

    /// An agent that holds no seat must not be able to act at all, least of
    /// all as the host: falling back to the host would hand any pane the
    /// operator's authority, including the right to wake the whole room.
    #[tokio::test]
    async fn an_unseated_caller_cannot_act_as_a_member_or_as_the_host() {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(
            &crate::config::Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("api")];
        app.state.active = Some(0);
        let room = app
            .state
            .company
            .create_room(
                &crate::company::Actor::Host,
                app.state.workspaces[0].id.clone(),
                "Fitness".into(),
                String::new(),
                1,
            )
            .expect("room");
        app.state
            .company
            .add_member(
                &crate::company::Actor::Host,
                &room.room_id,
                "Codex".into(),
                None,
                false,
            )
            .expect("seat");

        let response = app.handle_api_request(Request {
            id: "t".into(),
            method: Method::RoomPost(RoomPostParams {
                room_id: room.room_id.clone(),
                body: "@all status".into(),
                caller_pane_id: Some("w1:p1".into()),
                recipients: Vec::new(),
                root_id: None,
                client_nonce: None,
            }),
        });
        assert!(
            response.contains("room_not_seated"),
            "expected a refusal, got {response}"
        );

        // The room recorded nothing, so the refusal is not merely cosmetic.
        assert!(app
            .state
            .company
            .room(&room.room_id)
            .expect("room")
            .events
            .is_empty());
    }
}
