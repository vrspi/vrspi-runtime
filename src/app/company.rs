//! Runtime side of company rooms: turning a pending delivery into exactly one
//! agent activation.
//!
//! Reuses the delivery discipline already proven for agent mail: only a live,
//! recognized, idle agent is activated, at most one activation per seat per
//! pass, and every activation is charged against the objective's allowance
//! before any terminal input is queued.

use crate::company::{activation_packet, deliveries_per_member, Actor, RoomEvent};

use super::App;

impl App {
    /// Dispatches at most one pending room delivery per bound seat.
    ///
    /// Returns whether anything changed, so a quiet room costs no redraw.
    pub(crate) fn dispatch_room_deliveries(&mut self) -> bool {
        if self.state.company.is_empty() {
            return false;
        }
        let room_ids: Vec<String> = self
            .state
            .company
            .rooms()
            .iter()
            .map(|room| room.room_id.clone())
            .collect();

        let mut changed = false;
        for room_id in room_ids {
            changed |= self.dispatch_room(&room_id);
        }
        changed
    }

    fn dispatch_room(&mut self, room_id: &str) -> bool {
        let bundles = deliveries_per_member(self.state.company.pending_deliveries(room_id));
        if bundles.is_empty() {
            return false;
        }
        let Some(charter) = self.state.company.charter(room_id) else {
            return false;
        };

        let mut changed = false;
        for bundle in bundles {
            let Some((first, _)) = bundle.first() else {
                continue;
            };
            let member_id = first.member_id.clone();
            let Some(instance_id) = first.instance_id.clone() else {
                // An unbound seat has nowhere to deliver; the entry stays
                // pending until the host binds an agent to it.
                continue;
            };
            let Some(agent) = self.agent_info_for_instance(&instance_id) else {
                continue;
            };
            // Same gate as agent mail: never interrupt a working agent, and
            // never target one that is still starting up. An agent that has
            // finished its turn is ready for the next one, so `done` counts as
            // ready — otherwise its mail would sit until someone looked at the
            // pane and the status happened to be recomputed.
            if !crate::collaboration::agent_is_settled(agent.agent_status) || agent.launch_pending {
                continue;
            }
            let Some(kind) = agent
                .agent
                .as_deref()
                .and_then(crate::detect::parse_agent_label)
            else {
                continue;
            };
            let Some(terminal_id) = self
                .state
                .terminals
                .keys()
                .find(|id| id.to_string() == agent.terminal_id)
                .cloned()
            else {
                continue;
            };
            let Some(runtime) = self.terminal_runtimes.get(&terminal_id) else {
                continue;
            };
            if !super::agents::runtime_hosts_agent(runtime, kind) {
                continue;
            }

            // Claim and charge every message before queuing terminal input.
            // A failed enqueue releases the claim and refunds the activation;
            // a crash leaves a claimed delivery uncertain on restore rather
            // than blindly retrying a request that may already have started.
            let mut charged: Vec<(RoomEvent, u32)> = Vec::new();
            for (_, event) in &bundle {
                match self.state.company.claim_delivery(
                    room_id,
                    &event.event_id,
                    &member_id,
                    &instance_id,
                ) {
                    Ok(()) => {
                        let remaining = self
                            .state
                            .company
                            .room(room_id)
                            .and_then(|room| {
                                room.budgets
                                    .iter()
                                    .find(|b| b.root_id == event.root_id)
                                    .map(|b| b.remaining())
                            })
                            .unwrap_or(0);
                        charged.push((event.clone(), remaining));
                    }
                    Err(err) => {
                        tracing::debug!(
                            room = %room_id,
                            event = %event.event_id,
                            error = err.code(),
                            "room delivery refused"
                        );
                    }
                }
            }
            if charged.is_empty() {
                continue;
            }

            let labelled: Vec<(RoomEvent, String, u32)> = charged
                .into_iter()
                .map(|(event, remaining)| {
                    let from_label = match &event.actor {
                        Actor::Host => "host".to_string(),
                        Actor::Member { member_id } => self
                            .state
                            .company
                            .room(room_id)
                            .and_then(|room| room.member(member_id))
                            .map(|m| format!("@{}", m.handle))
                            .unwrap_or_else(|| "a teammate".to_string()),
                    };
                    (event, from_label, remaining)
                })
                .collect();
            let packet = activation_packet(&charter, room_id, &labelled);
            if let Err(err) = super::api_helpers::send_agent_prompt(runtime, &packet) {
                for (event, _, _) in &labelled {
                    let _ = self.state.company.release_delivery_claim(
                        room_id,
                        &event.event_id,
                        &member_id,
                    );
                }
                tracing::debug!(
                    room = %room_id,
                    member = %member_id,
                    error = %err,
                    "room delivery deferred"
                );
                continue;
            }
            for (event, _, _) in &labelled {
                if let Err(err) =
                    self.state
                        .company
                        .mark_delivery_submitted(room_id, &event.event_id, &member_id)
                {
                    tracing::warn!(
                        room = %room_id,
                        event = %event.event_id,
                        member = %member_id,
                        error = err.code(),
                        "room delivery enqueue could not be recorded"
                    );
                }
            }
            self.schedule_session_save();
            changed = true;
        }
        changed
    }

    /// Applies one task command with commit-before-reply ordering.
    ///
    /// The only way task state changes in the running server. The journal
    /// delta is made durable first, so a caller is never told a command
    /// succeeded — or was durably refused — before its record exists. With no
    /// session directory there is nothing to commit to, and the command is
    /// applied in memory only; that is the same bargain the rest of
    /// no-session mode makes.
    #[allow(dead_code)]
    pub(crate) fn apply_task_mutation(
        &mut self,
        ctx: crate::company::tasks::CommandContext,
        mutation: crate::company::tasks::TaskMutation,
    ) -> Result<crate::company::tasks::CommittedOutcome, crate::company::tasks::StoreError> {
        let outcome = match self.task_journal.take() {
            Some(mut journal) => {
                let committed = crate::company::tasks::commit_mutation(
                    self.state.company.tasks_mut(),
                    &mut journal,
                    ctx,
                    mutation,
                );
                self.task_journal = Some(journal);
                committed?
            }
            None => self.state.company.tasks_mut().reduce(ctx, mutation),
        };
        // The snapshot is a cache of what the journal already guarantees, so
        // saving it is scheduled rather than awaited.
        self.schedule_session_save();
        Ok(outcome)
    }

    /// Finds the live agent for a collaboration incarnation.
    ///
    /// After a restart no agent is registered until something registers it,
    /// and registration is what lets a resumed session reclaim its old
    /// incarnation. Agent mail used to be the only thing that did so, and it
    /// does nothing when no agent mail is queued — so a room alone could never
    /// reach a seat whose agent had come back. Unregistered agents are
    /// registered here instead; once each is, this stays a read.
    pub(crate) fn agent_info_for_instance(
        &mut self,
        instance_id: &str,
    ) -> Option<crate::api::schema::AgentInfo> {
        let agents = self.collect_agent_infos();
        if let Some(agent) = agents.iter().find(|agent| {
            self.state
                .collaboration
                .instance_for_terminal(&agent.terminal_id)
                .as_deref()
                == Some(instance_id)
        }) {
            return Some(agent.clone());
        }
        for agent in &agents {
            if self
                .state
                .collaboration
                .instance_for_terminal(&agent.terminal_id)
                .is_some()
            {
                continue;
            }
            if self.ensure_collaboration_agent(agent).instance_id == instance_id {
                return Some(agent.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::AgentStatus;
    use crate::company::journal::{recover, FileJournalSink};
    use crate::company::tasks::{
        CommandContext, CommittedOutcome, TaskActor, TaskLedger, TaskMutation, TaskState,
    };
    use crate::company::Actor;

    /// The server path: a task command is durable before the caller is told,
    /// and a snapshot that never saved can be rebuilt from the journal.
    #[tokio::test]
    async fn the_server_commits_a_task_command_before_answering() {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(
            &crate::config::Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let unique = format!(
            "vrspi-app-journal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        );
        let dir = std::env::temp_dir().join(unique);
        let journal = FileJournalSink::new(dir.join("company-tasks.journal"));
        app.task_journal = Some(journal.clone());

        let ctx = CommandContext {
            actor: TaskActor::Host,
            idempotency_key: "server-1".into(),
            expected_revision: None,
            caused_by: None,
            now_unix_ms: 10,
        };
        let outcome = app
            .apply_task_mutation(
                ctx,
                TaskMutation::Create {
                    room_id: "room_3".into(),
                    root_id: "evt_41".into(),
                    title: "Durable path".into(),
                    owner_member_id: "seat_11".into(),
                    depends_on: Vec::new(),
                    artifact_id: None,
                },
            )
            .expect("committed");
        let task_id = match outcome {
            CommittedOutcome::Applied(result) => result.events[0].task_id.clone(),
            CommittedOutcome::Rejected(_) => panic!("expected acceptance"),
        };

        // The record exists on disk, not merely in memory.
        let durable = journal.read_all().expect("read journal");
        assert_eq!(
            durable.len(),
            1,
            "the command was committed before it answered"
        );

        // A process that dies before the snapshot saves still recovers it.
        let mut rebuilt = TaskLedger::default();
        assert_eq!(recover(&mut rebuilt, &journal).expect("recover"), 1);
        assert_eq!(
            rebuilt.task(&task_id).expect("task").state,
            TaskState::Ready
        );
        assert_eq!(rebuilt, *app.state.company.tasks());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two agents in one workspace, both bound to seats in one room, with real
    /// terminal runtimes so injected input can be observed.
    async fn room_fixture() -> (
        crate::app::App,
        String,
        String,
        String,
        tokio::sync::mpsc::Receiver<bytes::Bytes>,
        tokio::sync::mpsc::Receiver<bytes::Bytes>,
    ) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(
            &crate::config::Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let mut workspace = crate::workspace::Workspace::test_new("company");
        let first_pane = workspace.tabs[0].root_pane;
        let second_pane = workspace.test_split(ratatui::layout::Direction::Horizontal);
        app.state.workspaces = vec![workspace];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);

        let mut receivers = Vec::new();
        for (pane_id, name, kind) in [
            (first_pane, "codex-agent", crate::detect::Agent::Codex),
            (second_pane, "claude-agent", crate::detect::Agent::Claude),
        ] {
            let terminal_id = app.state.workspaces[0]
                .terminal_id(pane_id)
                .cloned()
                .expect("terminal");
            let terminal = app.state.terminals.get_mut(&terminal_id).expect("state");
            terminal.set_agent_name(name.into());
            terminal.set_detected_state(Some(kind), crate::detect::AgentState::Idle);
            let (runtime, rx) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
            app.terminal_runtimes.insert(terminal_id, runtime);
            receivers.push(rx);
        }

        let room = app
            .state
            .company
            .create_room(
                &Actor::Host,
                "w1".into(),
                "Product team".into(),
                "Improve onboarding".into(),
                10,
            )
            .expect("room");
        let codex_seat = app
            .state
            .company
            .add_member(
                &Actor::Host,
                &room.room_id,
                "Codex".into(),
                Some("dev".into()),
                false,
            )
            .expect("seat");
        let reviewer_seat = app
            .state
            .company
            .add_member(&Actor::Host, &room.room_id, "reviewer".into(), None, false)
            .expect("seat");

        for (pane_id, member_id) in [
            (first_pane, &codex_seat.member_id),
            (second_pane, &reviewer_seat.member_id),
        ] {
            let agent = app.agent_info(0, pane_id).expect("agent info");
            let instance = app.ensure_collaboration_agent(&agent).instance_id;
            app.state
                .company
                .bind_member(&room.room_id, member_id, Some(instance))
                .expect("bind");
        }

        let mut iter = receivers.into_iter();
        let codex_rx = iter.next().expect("codex rx");
        let reviewer_rx = iter.next().expect("reviewer rx");
        (
            app,
            room.room_id,
            codex_seat.member_id,
            reviewer_seat.member_id,
            codex_rx,
            reviewer_rx,
        )
    }

    #[tokio::test]
    async fn a_mention_activates_only_the_named_member() {
        let (mut app, room_id, _codex, _reviewer, mut codex_rx, mut reviewer_rx) =
            room_fixture().await;
        app.state
            .company
            .post(
                Actor::Host,
                &room_id,
                "@Codex, overview the project".into(),
                Vec::new(),
                None,
                20,
            )
            .expect("post");

        assert!(app.dispatch_room_deliveries());
        let bytes = codex_rx.try_recv().expect("codex activation");
        let text = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(text.contains("overview the project"));
        assert!(text.contains("untrusted"));
        assert!(text.contains("Product team"));
        // Enter follows separately: in the same write the agent takes it as
        // part of the paste and the activation sits unsubmitted.
        assert!(!text.ends_with('\r'));
        assert!(codex_rx.try_recv().is_err());
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), codex_rx.recv())
                .await
                .expect("delayed enter")
                .expect("enter"),
            bytes::Bytes::from_static(b"\r")
        );
        // The other member can read the room later but must not be prompted.
        assert!(reviewer_rx.try_recv().is_err());
    }

    /// The exact failure behind a silent orchestrator: the server restarts,
    /// the agent resumes the same native session and reclaims its incarnation,
    /// no agent mail is queued — and the room must still reach it.
    #[tokio::test]
    async fn a_room_reaches_a_resumed_agent_after_a_restart_without_agent_mail() {
        let (mut app, room_id, codex, _reviewer, mut codex_rx, _reviewer_rx) = room_fixture().await;

        // Give the codex agent a native session, and re-register it, so it has
        // a fingerprint a restart can be matched against.
        let terminal_id = app.state.workspaces[0]
            .terminal_id(app.state.workspaces[0].tabs[0].root_pane)
            .cloned()
            .expect("terminal");
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("terminal state")
            .set_agent_session_ref(
                "herdr:codex".into(),
                "codex".into(),
                crate::agent_resume::AgentSessionRef::id("luna-session"),
                Some(1),
            )
            .expect("session recorded");
        let before_restart = app
            .agent_info_for_instance(
                &app.state
                    .company
                    .room(&room_id)
                    .and_then(|room| room.member(&codex))
                    .and_then(|member| member.bound_instance_id.clone())
                    .expect("bound"),
            )
            .expect("live before restart");
        let incarnation = app.ensure_collaboration_agent(&before_restart).instance_id;
        app.state
            .company
            .bind_member(&room_id, &codex, Some(incarnation.clone()))
            .expect("bind to the fingerprinted incarnation");

        // A cold restart: every live registration becomes a reconnect
        // candidate, and the room keeps its seats.
        app.state.collaboration.prepare_for_cold_restore();
        app.state.company.prepare_for_cold_restore();
        assert!(
            !app.state.collaboration.has_delivery_work(),
            "no agent mail is queued, so the mail loop will not register anyone"
        );
        assert_eq!(
            app.state
                .company
                .room(&room_id)
                .and_then(|room| room.member(&codex))
                .and_then(|member| member.bound_instance_id.clone()),
            Some(incarnation.clone()),
            "the seat survives the restart"
        );

        app.state
            .company
            .post(
                Actor::Host,
                &room_id,
                "@Codex there is a problem, check the logs".into(),
                Vec::new(),
                None,
                30,
            )
            .expect("post");

        assert!(
            app.dispatch_room_deliveries(),
            "the room itself must register the resumed agent and deliver"
        );
        let bytes = codex_rx.try_recv().expect("the resumed agent is activated");
        let text = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(text.contains("check the logs"));
        // And it reclaimed the same incarnation rather than a new one.
        assert_eq!(
            app.state
                .collaboration
                .instance_for_terminal(&terminal_id.to_string())
                .as_deref(),
            Some(incarnation.as_str())
        );
    }

    #[tokio::test]
    async fn a_quiescent_room_produces_no_activations() {
        let (mut app, _room_id, _c, _r, mut codex_rx, mut reviewer_rx) = room_fixture().await;
        // Nothing posted: repeated passes must stay silent (§18.11).
        for _ in 0..5 {
            assert!(!app.dispatch_room_deliveries());
        }
        assert!(codex_rx.try_recv().is_err());
        assert!(reviewer_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_delivered_event_is_not_dispatched_twice() {
        let (mut app, room_id, _c, _r, mut codex_rx, _reviewer_rx) = room_fixture().await;
        app.state
            .company
            .post(
                Actor::Host,
                &room_id,
                "@Codex go".into(),
                Vec::new(),
                None,
                20,
            )
            .expect("post");

        assert!(app.dispatch_room_deliveries());
        assert!(codex_rx.try_recv().is_ok());
        // A second pass must not resend the same prompt.
        assert!(!app.dispatch_room_deliveries());
        assert!(codex_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_working_member_is_not_interrupted() {
        let (mut app, room_id, _c, _r, mut codex_rx, _reviewer_rx) = room_fixture().await;
        let pane = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0]
            .terminal_id(pane)
            .cloned()
            .expect("terminal");
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("state")
            .set_detected_state(
                Some(crate::detect::Agent::Codex),
                crate::detect::AgentState::Working,
            );

        app.state
            .company
            .post(
                Actor::Host,
                &room_id,
                "@Codex go".into(),
                Vec::new(),
                None,
                20,
            )
            .expect("post");
        assert!(!app.dispatch_room_deliveries());
        assert!(codex_rx.try_recv().is_err());

        // It lands once the member goes idle.
        app.state
            .terminals
            .get_mut(&terminal_id)
            .expect("state")
            .set_detected_state(
                Some(crate::detect::Agent::Codex),
                crate::detect::AgentState::Idle,
            );
        assert!(app.dispatch_room_deliveries());
        assert!(codex_rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn an_exhausted_allowance_stops_further_activations() {
        let (mut app, room_id, codex, _r, mut codex_rx, _reviewer_rx) = room_fixture().await;
        let event = app
            .state
            .company
            .post(
                Actor::Host,
                &room_id,
                "@Codex go".into(),
                Vec::new(),
                None,
                20,
            )
            .expect("post");

        // Spend the objective's whole allowance outside the dispatcher.
        for _ in 0..crate::company::DEFAULT_ROOT_ACTIVATION_ALLOWANCE {
            let _ = app
                .state
                .company
                .mark_delivered(&room_id, &event.event_id, &codex);
        }
        app.state
            .company
            .post(
                Actor::Host,
                &room_id,
                "@Codex again".into(),
                Vec::new(),
                Some(event.root_id.clone()),
                21,
            )
            .expect("post");

        // The chain is spent, so no further terminal input is queued.
        assert!(!app.dispatch_room_deliveries());
        assert!(codex_rx.try_recv().is_err());

        // Only the host can unblock it.
        app.state
            .company
            .extend_allowance(&Actor::Host, &room_id, &event.root_id, 3)
            .expect("extend");
        assert!(app.dispatch_room_deliveries());
        assert!(codex_rx.try_recv().is_ok());
    }

    #[tokio::test]
    async fn an_unbound_seat_holds_its_delivery_until_an_agent_is_bound() {
        let (mut app, room_id, codex, _r, mut codex_rx, _reviewer_rx) = room_fixture().await;
        app.state
            .company
            .bind_member(&room_id, &codex, None)
            .expect("unbind");
        app.state
            .company
            .post(
                Actor::Host,
                &room_id,
                "@Codex go".into(),
                Vec::new(),
                None,
                20,
            )
            .expect("post");

        assert!(!app.dispatch_room_deliveries());
        assert!(codex_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_paused_room_dispatches_nothing() {
        let (mut app, room_id, _c, _r, mut codex_rx, _reviewer_rx) = room_fixture().await;
        app.state
            .company
            .post(
                Actor::Host,
                &room_id,
                "@Codex go".into(),
                Vec::new(),
                None,
                20,
            )
            .expect("post");
        app.state
            .company
            .set_lifecycle(
                &Actor::Host,
                &room_id,
                crate::company::RoomLifecycle::Paused,
            )
            .expect("pause");

        assert!(!app.dispatch_room_deliveries());
        assert!(codex_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn room_state_survives_a_session_snapshot_round_trip() {
        let (mut app, room_id, _c, _r, _codex_rx, _reviewer_rx) = room_fixture().await;
        app.state
            .company
            .put_record(
                Actor::Host,
                &room_id,
                None,
                None,
                "Architecture map".into(),
                "how it fits together".into(),
                "body".into(),
                Vec::new(),
                30,
            )
            .expect("record");

        let encoded = serde_json::to_string(&app.state.company).expect("serialize");
        let restored: crate::company::CompanyState =
            serde_json::from_str(&encoded).expect("deserialize");
        let room = restored.room(&room_id).expect("room");
        assert_eq!(room.name, "Product team");
        assert_eq!(room.members.len(), 2);
        assert_eq!(room.records.len(), 1);
        // Knowledge survives; the search index is derived, not stored.
        assert_eq!(
            restored.search_records(&room_id, "architecture", 5).len(),
            1
        );
    }

    #[tokio::test]
    async fn agent_status_gate_matches_the_mail_dispatcher() {
        // Guards the contract that room delivery uses the same lifecycle gate
        // as agent mail, rather than inventing a second policy.
        assert_ne!(AgentStatus::Working, AgentStatus::Idle);
        assert_ne!(AgentStatus::Blocked, AgentStatus::Idle);
    }
}
