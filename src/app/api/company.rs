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
