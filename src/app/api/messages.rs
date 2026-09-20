use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::schema::{
    AgentLobbyConnectParams, AgentLobbyListParams, AgentLobbyRemoveMemberParams,
    AgentLobbyRenameParams, AgentLobbyTargetParams, AgentMessageListParams, AgentMessageSendParams,
    AgentMessageTargetParams, AgentSelfParams, EventData, EventEnvelope, EventKind, ResponseResult,
};
use crate::collaboration::LobbyMutation;

use super::{responses, App};

impl App {
    pub(super) fn handle_agent_self(&mut self, id: String, params: AgentSelfParams) -> String {
        let agent = match self.agent_info_for_target(&params.caller_pane_id) {
            Ok(agent) => agent,
            Err(err) => return responses::encode_error_body(id, self.agent_target_error_body(err)),
        };
        let party = self.ensure_collaboration_agent(&agent);
        responses::encode_success(
            id,
            ResponseResult::AgentSelf {
                agent,
                instance_id: party.instance_id,
            },
        )
    }

    pub(super) fn handle_agent_message_send(
        &mut self,
        id: String,
        params: AgentMessageSendParams,
    ) -> String {
        let caller = match self.agent_info_for_target(&params.caller_pane_id) {
            Ok(agent) => agent,
            Err(err) => return responses::encode_error_body(id, self.agent_target_error_body(err)),
        };
        let target = match self.agent_info_for_target(&params.target) {
            Ok(agent) => agent,
            Err(err) => return responses::encode_error_body(id, self.agent_target_error_body(err)),
        };
        let sender = self.ensure_collaboration_agent(&caller);
        let recipient = self.ensure_collaboration_agent(&target);
        match self.state.collaboration.send(
            sender,
            recipient,
            params.body,
            params.reply_to,
            params.client_nonce,
            current_unix_ms(),
        ) {
            Ok(message) => {
                self.schedule_session_save();
                let message_id = message.message_id.clone();
                let _ = self.dispatch_collaboration_messages();
                let message = self
                    .state
                    .collaboration
                    .message(&message_id)
                    .unwrap_or(message);
                responses::encode_success(id, ResponseResult::AgentMessage { message })
            }
            Err(err) => responses::encode_error(id, err.code(), err.message()),
        }
    }

    pub(super) fn handle_agent_lobby_connect(
        &mut self,
        id: String,
        params: AgentLobbyConnectParams,
    ) -> String {
        let caller = match self.agent_info_for_target(&params.caller_pane_id) {
            Ok(agent) => agent,
            Err(err) => return responses::encode_error_body(id, self.agent_target_error_body(err)),
        };
        let target = match self.agent_info_for_target(&params.target) {
            Ok(agent) => agent,
            Err(err) => return responses::encode_error_body(id, self.agent_target_error_body(err)),
        };
        let first = self.ensure_collaboration_agent(&caller);
        let second = self.ensure_collaboration_agent(&target);
        match self
            .state
            .collaboration
            .connect(first, second, current_unix_ms())
        {
            Ok(lobby) => {
                self.schedule_session_save();
                self.emit_lobby_updated(lobby.clone());
                responses::encode_success(id, ResponseResult::AgentLobby { lobby })
            }
            Err(err) => responses::encode_error(id, err.code(), err.message()),
        }
    }

    pub(super) fn handle_agent_lobby_list(
        &mut self,
        id: String,
        params: AgentLobbyListParams,
    ) -> String {
        if let Err(err) = self.agent_info_for_target(&params.caller_pane_id) {
            return responses::encode_error_body(id, self.agent_target_error_body(err));
        }
        responses::encode_success(
            id,
            ResponseResult::AgentLobbyList {
                lobbies: self.state.collaboration.lobbies(),
            },
        )
    }

    pub(super) fn handle_agent_lobby_leave(
        &mut self,
        id: String,
        params: AgentLobbyTargetParams,
    ) -> String {
        let caller = match self.message_caller(&params.caller_pane_id) {
            Ok(caller) => caller,
            Err(response) => return responses::encode_error_body(id, response),
        };
        let result = self
            .state
            .collaboration
            .leave_lobby(&caller, &params.lobby_id);
        self.encode_lobby_mutation(id, result)
    }

    pub(super) fn handle_agent_lobby_remove(
        &mut self,
        id: String,
        params: AgentLobbyRemoveMemberParams,
    ) -> String {
        let caller = match self.message_caller(&params.caller_pane_id) {
            Ok(caller) => caller,
            Err(response) => return responses::encode_error_body(id, response),
        };
        let result = self.state.collaboration.remove_lobby_member(
            &caller,
            &params.lobby_id,
            &params.member_instance_id,
        );
        self.encode_lobby_mutation(id, result)
    }

    pub(super) fn handle_agent_lobby_delete(
        &mut self,
        id: String,
        params: AgentLobbyTargetParams,
    ) -> String {
        let caller = match self.message_caller(&params.caller_pane_id) {
            Ok(caller) => caller,
            Err(response) => return responses::encode_error_body(id, response),
        };
        let result = self
            .state
            .collaboration
            .delete_lobby(&caller, &params.lobby_id);
        self.encode_lobby_mutation(id, result)
    }

    pub(super) fn handle_agent_lobby_rename(
        &mut self,
        id: String,
        params: AgentLobbyRenameParams,
    ) -> String {
        let caller = match self.message_caller(&params.caller_pane_id) {
            Ok(caller) => caller,
            Err(response) => return responses::encode_error_body(id, response),
        };
        match self
            .state
            .collaboration
            .rename_lobby(&caller, &params.lobby_id, params.label)
        {
            Ok(lobby) => {
                self.schedule_session_save();
                self.emit_lobby_updated(lobby.clone());
                responses::encode_success(id, ResponseResult::AgentLobby { lobby })
            }
            Err(err) => responses::encode_error(id, err.code(), err.message()),
        }
    }

    pub(super) fn handle_agent_message_list(
        &mut self,
        id: String,
        params: AgentMessageListParams,
    ) -> String {
        let caller = match self.agent_info_for_target(&params.caller_pane_id) {
            Ok(agent) => agent,
            Err(err) => return responses::encode_error_body(id, self.agent_target_error_body(err)),
        };
        let caller = self.ensure_collaboration_agent(&caller);
        let page = self.state.collaboration.list(
            &caller,
            params.mailbox,
            params.after_sequence,
            params.limit,
            params.unacknowledged_only,
            current_unix_ms(),
        );
        self.schedule_session_save();
        responses::encode_success(
            id,
            ResponseResult::AgentMessageList {
                messages: page.messages,
                latest_sequence: self.state.collaboration.latest_sequence(),
                has_more: page.has_more,
                next_after_sequence: page.next_after_sequence,
            },
        )
    }

    pub(super) fn handle_agent_message_get(
        &mut self,
        id: String,
        params: AgentMessageTargetParams,
    ) -> String {
        let caller = match self.message_caller(&params.caller_pane_id) {
            Ok(caller) => caller,
            Err(response) => return responses::encode_error_body(id, response),
        };
        match self
            .state
            .collaboration
            .get(&caller, &params.message_id, current_unix_ms())
        {
            Ok(message) => {
                self.schedule_session_save();
                responses::encode_success(id, ResponseResult::AgentMessage { message })
            }
            Err(err) => responses::encode_error(id, err.code(), err.message()),
        }
    }

    pub(super) fn handle_agent_message_ack(
        &mut self,
        id: String,
        params: AgentMessageTargetParams,
    ) -> String {
        let caller = match self.message_caller(&params.caller_pane_id) {
            Ok(caller) => caller,
            Err(response) => return responses::encode_error_body(id, response),
        };
        match self
            .state
            .collaboration
            .acknowledge(&caller, &params.message_id, current_unix_ms())
        {
            Ok(message) => {
                self.schedule_session_save();
                responses::encode_success(id, ResponseResult::AgentMessage { message })
            }
            Err(err) => responses::encode_error(id, err.code(), err.message()),
        }
    }

    pub(super) fn handle_agent_message_revoke(
        &mut self,
        id: String,
        params: AgentMessageTargetParams,
    ) -> String {
        let caller = match self.message_caller(&params.caller_pane_id) {
            Ok(caller) => caller,
            Err(response) => return responses::encode_error_body(id, response),
        };
        match self
            .state
            .collaboration
            .revoke(&caller, &params.message_id, current_unix_ms())
        {
            Ok(message) => {
                self.schedule_session_save();
                responses::encode_success(id, ResponseResult::AgentMessage { message })
            }
            Err(err) => responses::encode_error(id, err.code(), err.message()),
        }
    }

    fn message_caller(
        &mut self,
        caller_pane_id: &str,
    ) -> Result<crate::api::schema::AgentMessageParty, crate::api::schema::ErrorBody> {
        let caller = self
            .agent_info_for_target(caller_pane_id)
            .map_err(|err| self.agent_target_error_body(err))?;
        Ok(self.ensure_collaboration_agent(&caller))
    }

    pub(crate) fn ensure_collaboration_agent(
        &mut self,
        agent: &crate::api::schema::AgentInfo,
    ) -> crate::api::schema::AgentMessageParty {
        let (party, updates) = self.state.collaboration.ensure_agent_with_updates(agent);
        self.schedule_session_save();
        for lobby in updates {
            self.emit_lobby_updated(lobby);
        }
        party
    }

    pub(crate) fn emit_lobby_updated(&mut self, lobby: crate::api::schema::AgentLobby) {
        self.emit_event(EventEnvelope {
            event: EventKind::AgentLobbyUpdated,
            data: EventData::AgentLobbyUpdated { lobby },
        });
    }

    fn encode_lobby_mutation(
        &mut self,
        id: String,
        result: Result<LobbyMutation, crate::collaboration::CollaborationError>,
    ) -> String {
        match result {
            Ok(LobbyMutation::Updated(lobby)) => {
                self.schedule_session_save();
                self.emit_lobby_updated(lobby.clone());
                responses::encode_success(id, ResponseResult::AgentLobby { lobby })
            }
            Ok(LobbyMutation::Deleted { lobby_id }) => {
                self.schedule_session_save();
                self.emit_event(EventEnvelope {
                    event: EventKind::AgentLobbyDeleted,
                    data: EventData::AgentLobbyDeleted { lobby_id },
                });
                responses::encode_success(id, ResponseResult::Ok {})
            }
            Err(err) => responses::encode_error(id, err.code(), err.message()),
        }
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Direction;

    use super::*;
    use crate::api::schema::{AgentMessageBox, AgentMessageState, ErrorResponse, SuccessResponse};
    use crate::app::Mode;
    use crate::config::Config;
    use crate::detect::{Agent, AgentState};
    use crate::workspace::Workspace;

    fn app_with_two_agents() -> (App, String, String) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let mut workspace = Workspace::test_new("collaboration");
        let first = workspace.tabs[0].root_pane;
        let second = workspace.test_split(Direction::Horizontal);
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.mode = Mode::Terminal;

        for (pane_id, name, agent) in [
            (first, "author", Agent::Codex),
            (second, "reviewer", Agent::Claude),
        ] {
            let terminal_id = app.state.workspaces[0]
                .terminal_id(pane_id)
                .cloned()
                .expect("pane should have terminal");
            let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
            terminal.set_agent_name(name.into());
            terminal.set_detected_state(Some(agent), AgentState::Idle);
        }
        let first = app.public_pane_id(0, first).unwrap();
        let second = app.public_pane_id(0, second).unwrap();
        (app, first, second)
    }

    #[test]
    fn api_agents_exchange_observe_and_acknowledge_messages() {
        let (mut app, author, reviewer) = app_with_two_agents();
        let sent = app.handle_agent_message_send(
            "send".into(),
            AgentMessageSendParams {
                caller_pane_id: author.clone(),
                target: "reviewer".into(),
                body: "review auth.rs".into(),
                reply_to: None,
                client_nonce: Some("turn-1".into()),
            },
        );
        let sent: SuccessResponse = serde_json::from_str(&sent).unwrap();
        let ResponseResult::AgentMessage { message: sent } = sent.result else {
            panic!("expected sent message");
        };
        assert_eq!(sent.state, AgentMessageState::Pending);

        let inbox = app.handle_agent_message_list(
            "inbox".into(),
            AgentMessageListParams {
                caller_pane_id: reviewer.clone(),
                mailbox: AgentMessageBox::Inbox,
                after_sequence: 0,
                limit: None,
                unacknowledged_only: false,
            },
        );
        let inbox: SuccessResponse = serde_json::from_str(&inbox).unwrap();
        let ResponseResult::AgentMessageList { messages, .. } = inbox.result else {
            panic!("expected inbox");
        };
        assert_eq!(messages[0].state, AgentMessageState::Observed);

        let ack = app.handle_agent_message_ack(
            "ack".into(),
            AgentMessageTargetParams {
                caller_pane_id: reviewer,
                message_id: sent.message_id.clone(),
            },
        );
        let ack: SuccessResponse = serde_json::from_str(&ack).unwrap();
        let ResponseResult::AgentMessage { message: ack } = ack.result else {
            panic!("expected acknowledged message");
        };
        assert_eq!(ack.state, AgentMessageState::Acknowledged);

        let revoked = app.handle_agent_message_revoke(
            "revoke".into(),
            AgentMessageTargetParams {
                caller_pane_id: author,
                message_id: sent.message_id,
            },
        );
        let revoked: ErrorResponse = serde_json::from_str(&revoked).unwrap();
        assert_eq!(revoked.error.code, "message_already_acknowledged");
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn self_rejects_a_caller_that_is_not_an_agent() {
        let (mut app, author, _) = app_with_two_agents();
        let author_terminal = app.agent_info_for_target(&author).unwrap().terminal_id;
        app.state
            .terminals
            .values_mut()
            .find(|terminal| terminal.id.to_string() == author_terminal)
            .unwrap()
            .clear_agent_runtime_identity_after_respawn();
        let response = app.handle_agent_self(
            "self".into(),
            AgentSelfParams {
                caller_pane_id: author,
            },
        );
        let response: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(response.error.code, "agent_not_found");
    }

    #[test]
    fn lobby_connect_is_idempotent_and_lists_members() {
        let (mut app, author, reviewer) = app_with_two_agents();
        let connect = |app: &mut App| {
            let response = app.handle_agent_lobby_connect(
                "connect".into(),
                AgentLobbyConnectParams {
                    caller_pane_id: author.clone(),
                    target: reviewer.clone(),
                },
            );
            let response: SuccessResponse = serde_json::from_str(&response).unwrap();
            let ResponseResult::AgentLobby { lobby } = response.result else {
                panic!("expected lobby");
            };
            lobby
        };
        let first = connect(&mut app);
        let second = connect(&mut app);
        assert_eq!(first.lobby_id, second.lobby_id);
        assert_eq!(first.members.len(), 2);

        let listed = app.handle_agent_lobby_list(
            "list".into(),
            AgentLobbyListParams {
                caller_pane_id: author,
            },
        );
        let listed: SuccessResponse = serde_json::from_str(&listed).unwrap();
        let ResponseResult::AgentLobbyList { lobbies } = listed.result else {
            panic!("expected lobby list");
        };
        assert_eq!(lobbies.len(), 1);
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn lobby_lifecycle_routes_permissions_and_emits_live_events() {
        let (mut app, author, reviewer) = app_with_two_agents();
        let connected = app.handle_agent_lobby_connect(
            "connect".into(),
            AgentLobbyConnectParams {
                caller_pane_id: author.clone(),
                target: reviewer.clone(),
            },
        );
        let connected: SuccessResponse = serde_json::from_str(&connected).unwrap();
        let ResponseResult::AgentLobby { lobby } = connected.result else {
            panic!("expected lobby");
        };

        let denied = app.handle_agent_lobby_delete(
            "denied".into(),
            AgentLobbyTargetParams {
                caller_pane_id: reviewer.clone(),
                lobby_id: lobby.lobby_id.clone(),
            },
        );
        let denied: ErrorResponse = serde_json::from_str(&denied).unwrap();
        assert_eq!(denied.error.code, "agent_lobby_not_owner");

        let renamed = app.handle_agent_lobby_rename(
            "rename".into(),
            AgentLobbyRenameParams {
                caller_pane_id: author.clone(),
                lobby_id: lobby.lobby_id.clone(),
                label: "space review".into(),
            },
        );
        let renamed: SuccessResponse = serde_json::from_str(&renamed).unwrap();
        let ResponseResult::AgentLobby { lobby: renamed } = renamed.result else {
            panic!("expected renamed lobby");
        };
        assert_eq!(renamed.label, "space review");

        let removed = app.handle_agent_lobby_remove(
            "remove".into(),
            AgentLobbyRemoveMemberParams {
                caller_pane_id: author.clone(),
                lobby_id: lobby.lobby_id.clone(),
                member_instance_id: renamed.members[1].party.instance_id.clone(),
            },
        );
        let removed: SuccessResponse = serde_json::from_str(&removed).unwrap();
        assert!(matches!(removed.result, ResponseResult::AgentLobby { .. }));

        let deleted = app.handle_agent_lobby_delete(
            "delete".into(),
            AgentLobbyTargetParams {
                caller_pane_id: author,
                lobby_id: lobby.lobby_id.clone(),
            },
        );
        let deleted: SuccessResponse = serde_json::from_str(&deleted).unwrap();
        assert_eq!(deleted.result, ResponseResult::Ok {});

        let events = app.event_hub.events_after(0);
        assert!(events.iter().any(|(_, event)| {
            event.event == EventKind::AgentLobbyUpdated
                && matches!(event.data, EventData::AgentLobbyUpdated { .. })
        }));
        assert!(events.iter().any(|(_, event)| {
            event.event == EventKind::AgentLobbyDeleted
                && matches!(event.data, EventData::AgentLobbyDeleted { .. })
        }));
    }

    #[test]
    fn collaboration_routes_through_adversarial_public_pane_identities() {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state = crate::app::state::AppState::test_with_adversarial_identity_state();
        app.state.mode = Mode::Terminal;

        let pane_ids = app.state.workspaces[0]
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.pane_ids())
            .take(2)
            .collect::<Vec<_>>();
        assert_eq!(pane_ids.len(), 2);
        for (pane_id, name, agent) in [
            (pane_ids[0], "author", Agent::Codex),
            (pane_ids[1], "reviewer", Agent::Claude),
        ] {
            let terminal_id = app.state.workspaces[0]
                .terminal_id(pane_id)
                .cloned()
                .expect("adversarial pane should have a terminal");
            let terminal = app
                .state
                .terminals
                .get_mut(&terminal_id)
                .expect("adversarial terminal should exist");
            terminal.set_agent_name(name.into());
            terminal.set_detected_state(Some(agent), AgentState::Idle);
        }

        let author = app
            .public_pane_id(0, pane_ids[0])
            .expect("author should have a public pane id");
        let reviewer = app
            .public_pane_id(0, pane_ids[1])
            .expect("reviewer should have a public pane id");
        assert_ne!(author, reviewer);

        let sent = app.handle_agent_message_send(
            "send".into(),
            AgentMessageSendParams {
                caller_pane_id: author,
                target: "reviewer".into(),
                body: "check adversarial routing".into(),
                reply_to: None,
                client_nonce: None,
            },
        );
        let sent: SuccessResponse = serde_json::from_str(&sent).unwrap();
        assert!(matches!(sent.result, ResponseResult::AgentMessage { .. }));

        let inbox = app.handle_agent_message_list(
            "inbox".into(),
            AgentMessageListParams {
                caller_pane_id: reviewer,
                mailbox: AgentMessageBox::Inbox,
                after_sequence: 0,
                limit: None,
                unacknowledged_only: false,
            },
        );
        let inbox: SuccessResponse = serde_json::from_str(&inbox).unwrap();
        let ResponseResult::AgentMessageList { messages, .. } = inbox.result else {
            panic!("expected inbox response");
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].recipient.name.as_deref(), Some("reviewer"));
        app.state.assert_invariants_for_test();
    }
}
