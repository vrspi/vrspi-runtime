//! Company room browser: snapshotting server state for display, and turning
//! operator input into runtime mutations.
//!
//! The browser holds a snapshot rather than reading company state during
//! render, so drawing never inspects rooms and a quiet room costs no redraw.

use crate::api::schema::AgentStatus;
use crate::api::schema::Method;
use crate::app::state::{
    Mode, RoomAgentCandidate, RoomBindState, RoomBrowserState, RoomEventRow, RoomFormField,
    RoomFormKind, RoomFormState, RoomMemberRow, RoomRecordRow, RoomSnapshot, RoomTab,
    SeatReadiness,
};
use crate::company::Actor;

use super::App;

/// Turns visible in the conversation pane. Older history stays on the server
/// and is reachable through `room events`.
const MAX_VISIBLE_EVENTS: usize = 60;

impl App {
    /// Opens the browser, taking a fresh snapshot.
    pub(crate) fn open_room_browser(&mut self) {
        self.state.room_browser.selected = 0;
        self.state.room_browser.scroll = 0;
        self.state.room_browser.content_scroll = 0;
        self.state.room_browser.clear_overlays();
        self.state.room_browser.notice = None;
        self.refresh_room_browser();
        self.state.mode = Mode::CompanyRooms;
    }

    /// Opens the browser straight into the room creator for one workspace.
    ///
    /// This is what the workspace context menu asks for, so the workspace is
    /// already chosen and the operator only names the room.
    pub(crate) fn open_room_creator(&mut self, ws_idx: usize) {
        let Some(workspace_id) = self
            .state
            .workspaces
            .get(ws_idx)
            .map(|workspace| workspace.id.clone())
        else {
            return;
        };
        if self.state.mode != Mode::CompanyRooms {
            self.open_room_browser();
        }
        // Land on the room the new one will join, so creating from a workspace
        // that already has rooms does not look like it replaced them.
        self.select_first_room_in_workspace(&workspace_id);
        self.state.room_browser.clear_overlays();
        self.state.room_browser.notice = None;
        self.state.room_browser.form = Some(RoomFormState {
            kind: RoomFormKind::CreateRoom { workspace_id },
            primary: String::new(),
            secondary: String::new(),
            orchestrator: false,
            focus: RoomFormField::Primary,
        });
    }

    /// Opens the room creator for the workspace the selected room belongs to,
    /// falling back to the active workspace when there is no room yet.
    pub(crate) fn open_room_creator_for_selection(&mut self) {
        let workspace_id = self
            .state
            .room_browser
            .selected_room()
            .map(|room| room.workspace_id.clone())
            .or_else(|| {
                self.state
                    .active
                    .and_then(|ws_idx| self.state.workspaces.get(ws_idx))
                    .map(|workspace| workspace.id.clone())
            });
        let Some(workspace_id) = workspace_id else {
            self.state.room_browser.notice = Some("open a workspace first".into());
            return;
        };
        self.state.room_browser.clear_overlays();
        self.state.room_browser.notice = None;
        self.state.room_browser.form = Some(RoomFormState {
            kind: RoomFormKind::CreateRoom { workspace_id },
            primary: String::new(),
            secondary: String::new(),
            orchestrator: false,
            focus: RoomFormField::Primary,
        });
    }

    fn select_first_room_in_workspace(&mut self, workspace_id: &str) {
        if let Some(index) = self
            .state
            .room_browser
            .rooms
            .iter()
            .position(|room| room.workspace_id == workspace_id)
        {
            self.state.room_browser.selected = index;
            self.state.room_browser.content_scroll = 0;
        }
    }

    /// Opens the seat form for the selected room.
    pub(crate) fn open_room_seat_form(&mut self) {
        let Some(room) = self.state.room_browser.selected_room() else {
            self.state.room_browser.notice = Some("create a room before adding seats".into());
            return;
        };
        let kind = RoomFormKind::AddMember {
            room_id: room.room_id.clone(),
            room_name: room.name.clone(),
        };
        self.state.room_browser.clear_overlays();
        self.state.room_browser.notice = None;
        self.set_room_tab(RoomTab::Members);
        self.state.room_browser.form = Some(RoomFormState {
            kind,
            primary: String::new(),
            secondary: String::new(),
            orchestrator: false,
            focus: RoomFormField::Primary,
        });
    }

    pub(crate) fn cancel_room_form(&mut self) {
        self.state.room_browser.form = None;
        self.state.room_browser.notice = None;
    }

    pub(crate) fn focus_room_form_field(&mut self, field: RoomFormField) {
        if let Some(form) = self.state.room_browser.form.as_mut() {
            form.focus = field;
        }
    }

    pub(crate) fn toggle_room_form_orchestrator(&mut self) {
        if let Some(form) = self.state.room_browser.form.as_mut() {
            if !form.creating_room() {
                form.orchestrator = !form.orchestrator;
            }
        }
    }

    pub(crate) fn edit_room_form_field(&mut self, edit: impl FnOnce(&mut String)) {
        if let Some(form) = self.state.room_browser.form.as_mut() {
            edit(form.field_mut());
            self.state.room_browser.notice = None;
        }
    }

    /// Creates the room, or adds the seat, the open form describes.
    pub(crate) fn submit_room_form(&mut self) {
        let Some(form) = self.state.room_browser.form.clone() else {
            return;
        };
        let primary = form.primary.trim().to_string();
        if primary.is_empty() {
            self.state.room_browser.notice = Some(if form.creating_room() {
                "name the room before creating it".into()
            } else {
                "give the seat a handle before adding it".into()
            });
            return;
        }
        let secondary = form.secondary.trim().to_string();
        let creating = form.creating_room();
        let (label, method) = match form.kind {
            RoomFormKind::CreateRoom { workspace_id } => (
                "tui.room.create",
                Method::RoomCreate(crate::api::schema::RoomCreateParams {
                    workspace_id,
                    name: primary.clone(),
                    objective: secondary,
                }),
            ),
            RoomFormKind::AddMember { room_id, .. } => (
                "tui.room.member.add",
                Method::RoomMemberAdd(crate::api::schema::RoomMemberAddParams {
                    room_id,
                    handle: primary.clone(),
                    role: (!secondary.is_empty()).then_some(secondary),
                    orchestrator: form.orchestrator,
                }),
            ),
        };
        let response = self.dispatch_runtime_mutation(label, method);
        if let Some(error) = crate::app::collaboration_api_error(&response) {
            // Keep the draft so a taken handle can be corrected, not retyped.
            self.state.room_browser.notice = Some(error);
            return;
        }
        self.state.room_browser.form = None;
        self.refresh_room_browser();
        if creating {
            if let Some(index) = self
                .state
                .room_browser
                .rooms
                .iter()
                .position(|room| room.name == primary)
            {
                self.state.room_browser.selected = index;
                self.state.room_browser.content_scroll = 0;
            }
            self.state.room_browser.notice = Some(format!("created {primary}"));
        } else {
            self.state.room_browser.notice = Some(format!("added @{primary}"));
        }
    }

    /// Pauses an active room, or resumes a paused one.
    pub(crate) fn toggle_room_lifecycle(&mut self) {
        let Some(room) = self.state.room_browser.selected_room() else {
            return;
        };
        let room_id = room.room_id.clone();
        let resuming = room.lifecycle != "active";
        let response = self.dispatch_runtime_mutation(
            "tui.room.lifecycle",
            Method::RoomSetLifecycle(crate::api::schema::RoomLifecycleParams {
                room_id,
                lifecycle: if resuming {
                    crate::api::schema::RoomLifecycleValue::Active
                } else {
                    crate::api::schema::RoomLifecycleValue::Paused
                },
            }),
        );
        self.state.room_browser.notice = match crate::app::collaboration_api_error(&response) {
            Some(error) => Some(error),
            None if resuming => Some("room resumed".into()),
            None => Some("room paused; nothing will be activated".into()),
        };
        self.refresh_room_browser();
    }

    /// Opens the agent picker for one seat.
    pub(crate) fn open_room_bind_picker(&mut self, member_index: usize) {
        let Some(room) = self.state.room_browser.selected_room() else {
            return;
        };
        let Some(member) = room.members.get(member_index) else {
            return;
        };
        let bind = RoomBindState {
            room_id: room.room_id.clone(),
            member_id: member.member_id.clone(),
            handle: member.handle.clone(),
            candidates: Vec::new(),
            selected: 0,
            scroll: 0,
        };
        let workspace_id = room.workspace_id.clone();
        let mut candidates = self.room_agent_candidates(&workspace_id);
        // Agents in the room's own workspace are the usual intent, so they
        // come first rather than being hunted for in a mixed list.
        candidates.sort_by_key(|candidate| !candidate.same_workspace);
        self.state.room_browser.clear_overlays();
        self.state.room_browser.notice = None;
        self.state.room_browser.bind = Some(RoomBindState { candidates, ..bind });
    }

    fn room_agent_candidates(&self, workspace_id: &str) -> Vec<RoomAgentCandidate> {
        self.collect_agent_infos()
            .into_iter()
            .map(|agent| {
                let workspace = self
                    .state
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == agent.workspace_id)
                    .map(|workspace| workspace.display_name_from_terminals(&self.state.terminals))
                    .unwrap_or_else(|| agent.workspace_id.clone());
                RoomAgentCandidate {
                    label: super::collaboration::agent_display_label(&agent),
                    place: self.agent_place_label(&agent),
                    workspace,
                    same_workspace: agent.workspace_id == workspace_id,
                    pane_id: agent.pane_id,
                }
            })
            .collect()
    }

    pub(crate) fn move_room_bind_selection(&mut self, delta: isize) {
        let Some(bind) = self.state.room_browser.bind.as_mut() else {
            return;
        };
        if bind.candidates.is_empty() {
            return;
        }
        let last = bind.candidates.len() - 1;
        bind.selected = bind.selected.saturating_add_signed(delta).min(last);
    }

    /// Rows the current tab's pane holds, and how many of them fit on screen.
    fn room_content_metrics(&self) -> Option<(usize, usize)> {
        let chrome = crate::ui::room_chrome(&self.state);
        let layout = crate::ui::room_layout(self.state.onboarding_full_area(), chrome)?;
        let width = layout.content.width as usize;
        let rows = match self.state.room_browser.tab {
            RoomTab::Conversation => {
                crate::ui::conversation_rows(&self.state, width, &self.state.palette).len()
            }
            RoomTab::Memory => {
                crate::ui::memory_rows(&self.state, width, &self.state.palette).len()
            }
            RoomTab::Members => self
                .state
                .room_browser
                .selected_room()
                .map_or(0, |room| room.members.len()),
        };
        Some((rows, layout.content.height as usize))
    }

    /// Scrolls whichever list the current tab shows, by rows.
    ///
    /// `delta` is positive towards older content. The conversation reads
    /// backwards from the newest turn; the seat and memory lists read top-down,
    /// so the same wheel step moves their offsets in opposite directions.
    pub(crate) fn scroll_room_content(&mut self, delta: isize) {
        const ROWS_PER_STEP: isize = 3;
        let Some((rows, visible)) = self.room_content_metrics() else {
            return;
        };
        let max = rows.saturating_sub(visible);
        let step = match self.state.room_browser.tab {
            RoomTab::Conversation => delta * ROWS_PER_STEP,
            RoomTab::Members | RoomTab::Memory => -delta * ROWS_PER_STEP,
        };
        let browser = &mut self.state.room_browser;
        browser.content_scroll = browser.content_scroll.saturating_add_signed(step).min(max);
    }

    /// Opens or closes one turn or record without moving its header.
    ///
    /// The conversation is measured from its newest row, so growing a turn
    /// would otherwise push that turn's header up and out of view — exactly
    /// when the reader wants to start reading it. The offset absorbs the
    /// change instead, and the body opens downward from where it was clicked.
    pub(crate) fn toggle_room_expanded(&mut self, id: &str) {
        let before = self.room_content_metrics();
        self.state.room_browser.toggle_expanded(id);
        let Some((after, visible)) = self.room_content_metrics() else {
            return;
        };
        let max = after.saturating_sub(visible);
        let browser = &mut self.state.room_browser;
        if browser.tab == RoomTab::Conversation {
            if let Some((before, _)) = before {
                let grown = after as isize - before as isize;
                browser.content_scroll = browser.content_scroll.saturating_add_signed(grown);
            }
        }
        browser.content_scroll = browser.content_scroll.min(max);
    }

    pub(crate) fn cancel_room_bind(&mut self) {
        self.state.room_browser.bind = None;
        self.state.room_browser.notice = None;
    }

    /// Seats the highlighted agent.
    pub(crate) fn confirm_room_bind(&mut self) {
        let Some(bind) = self.state.room_browser.bind.clone() else {
            return;
        };
        let Some(candidate) = bind.candidates.get(bind.selected) else {
            self.state.room_browser.notice = Some("no agent to bind".into());
            return;
        };
        let label = candidate.label.clone();
        let response = self.dispatch_runtime_mutation(
            "tui.room.member.bind",
            Method::RoomMemberBind(crate::api::schema::RoomMemberBindParams {
                room_id: bind.room_id,
                member_id: bind.member_id,
                target: Some(candidate.pane_id.clone()),
            }),
        );
        match crate::app::collaboration_api_error(&response) {
            Some(error) => self.state.room_browser.notice = Some(error),
            None => {
                self.state.room_browser.bind = None;
                self.state.room_browser.notice = Some(format!("@{} is now {label}", bind.handle));
            }
        }
        self.refresh_room_browser();
    }

    /// Frees a seat without removing it, so the seat keeps its history.
    pub(crate) fn unbind_room_member(&mut self, member_index: usize) {
        let Some((room_id, member_id, handle)) = self.room_member_target(member_index) else {
            return;
        };
        let response = self.dispatch_runtime_mutation(
            "tui.room.member.unbind",
            Method::RoomMemberBind(crate::api::schema::RoomMemberBindParams {
                room_id,
                member_id,
                target: None,
            }),
        );
        self.state.room_browser.notice = match crate::app::collaboration_api_error(&response) {
            Some(error) => Some(error),
            None => Some(format!("@{handle} has no agent bound")),
        };
        self.refresh_room_browser();
    }

    /// Removes a seat, asking for a second click first.
    ///
    /// Removing a seat drops whatever it had not yet been activated with, so a
    /// stray click must not be enough.
    pub(crate) fn remove_room_member(&mut self, member_index: usize) {
        let Some((room_id, member_id, handle)) = self.room_member_target(member_index) else {
            return;
        };
        if self.state.room_browser.pending_remove.as_deref() != Some(member_id.as_str()) {
            self.state.room_browser.pending_remove = Some(member_id);
            self.state.room_browser.notice = Some(format!("click remove again to drop @{handle}"));
            return;
        }
        let response = self.dispatch_runtime_mutation(
            "tui.room.member.remove",
            Method::RoomMemberRemove(crate::api::schema::RoomMemberTargetParams {
                room_id,
                member_id,
            }),
        );
        self.state.room_browser.pending_remove = None;
        self.state.room_browser.notice = match crate::app::collaboration_api_error(&response) {
            Some(error) => Some(error),
            None => Some(format!("removed @{handle}")),
        };
        self.refresh_room_browser();
    }

    /// Expands or collapses the turn or record the reader is looking at.
    ///
    /// For the conversation that is the newest turn whose header is on screen;
    /// for memory, the record at the top of the pane.
    pub(crate) fn toggle_room_focus_expanded(&mut self) {
        let Some((_, visible)) = self.room_content_metrics() else {
            return;
        };
        let chrome = crate::ui::room_chrome(&self.state);
        let Some(layout) = crate::ui::room_layout(self.state.onboarding_full_area(), chrome) else {
            return;
        };
        let width = layout.content.width as usize;
        let scroll = self.state.room_browser.content_scroll;
        let id = match self.state.room_browser.tab {
            RoomTab::Conversation => {
                let rows = crate::ui::conversation_rows(&self.state, width, &self.state.palette);
                let first = crate::ui::conversation_first_row(rows.len(), scroll, visible);
                let end = (first + visible).min(rows.len());
                let header = (first..end)
                    .rev()
                    .find(|&index| index == 0 || rows[index - 1].event != rows[index].event)
                    .map(|index| rows[index].event);
                header.and_then(|index| {
                    self.state
                        .room_browser
                        .selected_room()
                        .and_then(|room| room.events.get(index))
                        .map(|event| event.event_id.clone())
                })
            }
            RoomTab::Memory => {
                let rows = crate::ui::memory_rows(&self.state, width, &self.state.palette);
                let first = crate::ui::memory_first_row(rows.len(), scroll, visible);
                rows.get(first).and_then(|row| {
                    self.state
                        .room_browser
                        .selected_room()
                        .and_then(|room| room.records.get(row.record))
                        .map(|record| record.record_id.clone())
                })
            }
            RoomTab::Members => None,
        };
        if let Some(id) = id {
            self.toggle_room_expanded(&id);
        }
    }

    /// Deletes the room its right-click menu is open on, after a second click.
    pub(crate) fn delete_selected_room(&mut self) {
        let Some(menu) = self.state.room_browser.row_menu else {
            return;
        };
        let Some(room) = self.state.room_browser.rooms.get(menu.room_index) else {
            self.state.room_browser.row_menu = None;
            return;
        };
        let (room_id, name) = (room.room_id.clone(), room.name.clone());
        if !menu.confirming {
            self.state.room_browser.row_menu = Some(crate::app::state::RoomRowMenu {
                confirming: true,
                ..menu
            });
            self.state.room_browser.notice =
                Some(format!("deleting {name} removes its history; click again"));
            return;
        }
        let response = self.dispatch_runtime_mutation(
            "tui.room.delete",
            Method::RoomDelete(crate::api::schema::RoomTargetParams { room_id }),
        );
        self.state.room_browser.row_menu = None;
        self.state.room_browser.notice = match crate::app::collaboration_api_error(&response) {
            Some(error) => Some(error),
            None => Some(format!("deleted {name}")),
        };
        self.state.room_browser.selected = self.state.room_browser.selected.saturating_sub(1);
        self.refresh_room_browser();
    }

    /// Accepts the room's knowledge record at `index`, as the host.
    pub(crate) fn accept_room_record(&mut self, index: usize) {
        let Some((room_id, record_id, title)) = self.room_record_target(index) else {
            return;
        };
        let response = self.dispatch_runtime_mutation(
            "tui.room.memory.accept",
            Method::RoomMemoryAccept(crate::api::schema::RoomMemoryTargetParams {
                room_id,
                record_id,
                caller_pane_id: None,
            }),
        );
        self.state.room_browser.notice = match crate::app::collaboration_api_error(&response) {
            Some(error) => Some(error),
            None => Some(format!("accepted {title}")),
        };
        self.refresh_room_browser();
    }

    /// Deletes a knowledge record, asking for a second click first.
    pub(crate) fn delete_room_record(&mut self, index: usize) {
        let Some((room_id, record_id, title)) = self.room_record_target(index) else {
            return;
        };
        if self.state.room_browser.pending_remove.as_deref() != Some(record_id.as_str()) {
            self.state.room_browser.pending_remove = Some(record_id);
            self.state.room_browser.notice = Some(format!("click delete again to drop {title}"));
            return;
        }
        let response = self.dispatch_runtime_mutation(
            "tui.room.memory.delete",
            Method::RoomMemoryDelete(crate::api::schema::RoomMemoryTargetParams {
                room_id,
                record_id,
                caller_pane_id: None,
            }),
        );
        self.state.room_browser.pending_remove = None;
        self.state.room_browser.notice = match crate::app::collaboration_api_error(&response) {
            Some(error) => Some(error),
            None => Some(format!("deleted {title}")),
        };
        self.refresh_room_browser();
    }

    fn room_record_target(&self, index: usize) -> Option<(String, String, String)> {
        let room = self.state.room_browser.selected_room()?;
        let record = room.records.get(index)?;
        Some((
            room.room_id.clone(),
            record.record_id.clone(),
            record.title.clone(),
        ))
    }

    fn room_member_target(&self, member_index: usize) -> Option<(String, String, String)> {
        let room = self.state.room_browser.selected_room()?;
        let member = room.members.get(member_index)?;
        Some((
            room.room_id.clone(),
            member.member_id.clone(),
            member.handle.clone(),
        ))
    }

    /// Keeps an open browser current. Returns whether the view changed, so a
    /// quiet room costs no redraw.
    pub(crate) fn refresh_open_room_browser(&mut self) -> bool {
        if self.state.mode != Mode::CompanyRooms {
            return false;
        }
        self.refresh_room_browser()
    }

    /// Re-reads rooms from company state into display rows.
    fn refresh_room_browser(&mut self) -> bool {
        // Who each live agent is, read without registering anyone, so an open
        // browser never mints incarnations or schedules saves on its own.
        let live: std::collections::HashMap<String, (AgentStatus, bool, Option<String>)> = self
            .collect_agent_infos()
            .into_iter()
            .filter_map(|agent| {
                let instance_id = self.state.collaboration.instance_for_agent(&agent)?;
                let place = self.agent_place_label(&agent);
                Some((
                    instance_id,
                    (agent.agent_status, agent.launch_pending, place),
                ))
            })
            .collect();
        let rooms: Vec<RoomSnapshot> = self
            .state
            .company
            .rooms()
            .iter()
            .map(|room| {
                let handle_of = |member_id: &str| {
                    room.member(member_id)
                        .map(|m| format!("@{}", m.handle))
                        .unwrap_or_else(|| member_id.to_string())
                };
                RoomSnapshot {
                    room_id: room.room_id.clone(),
                    name: room.name.clone(),
                    workspace_id: room.workspace_id.clone(),
                    objective: room.objective.clone(),
                    lifecycle: format!("{:?}", room.lifecycle).to_lowercase(),
                    members: room
                        .members
                        .iter()
                        .map(|m| {
                            let counts = room.delivery_counts(&m.member_id);
                            RoomMemberRow {
                                member_id: m.member_id.clone(),
                                handle: m.handle.clone(),
                                role: m.role.clone(),
                                orchestrator: m.is_orchestrator,
                                bound: m.bound_instance_id.is_some(),
                                readiness: seat_readiness(m.bound_instance_id.as_deref(), &live),
                                place: m
                                    .bound_instance_id
                                    .as_deref()
                                    .and_then(|id| live.get(id))
                                    .and_then(|(_, _, place)| place.clone()),
                                waiting: counts.waiting,
                                in_flight: counts.in_flight,
                                needs_attention: counts.needs_attention,
                                lost: counts.lost,
                            }
                        })
                        .collect(),
                    events: room
                        .events
                        .iter()
                        .rev()
                        .take(MAX_VISIBLE_EVENTS)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .map(|event| RoomEventRow {
                            event_id: event.event_id.clone(),
                            from: match &event.actor {
                                Actor::Host => "host".to_string(),
                                Actor::Member { member_id } => handle_of(member_id),
                            },
                            body: event.body.clone(),
                            recipients: event.recipients.iter().map(|id| handle_of(id)).collect(),
                        })
                        .collect(),
                    records: room
                        .records
                        .iter()
                        .map(|record| RoomRecordRow {
                            record_id: record.record_id.clone(),
                            title: record.title.clone(),
                            summary: record.summary.clone(),
                            body: record.body.clone(),
                            author: match &record.author {
                                Actor::Host => "host".to_string(),
                                Actor::Member { member_id } => handle_of(member_id),
                            },
                            accepted: matches!(
                                record.status,
                                crate::company::RecordStatus::Accepted
                            ),
                        })
                        .collect(),
                }
            })
            .collect();

        let selected = if rooms.is_empty() {
            0
        } else {
            self.state.room_browser.selected.min(rooms.len() - 1)
        };
        let changed =
            rooms != self.state.room_browser.rooms || selected != self.state.room_browser.selected;
        if !changed {
            return false;
        }
        // A room that gained a turn should show it rather than keep the
        // operator parked on older history.
        if rooms.get(selected).map(|r| r.events.len())
            != self
                .state
                .room_browser
                .rooms
                .get(selected)
                .map(|r| r.events.len())
        {
            self.state.room_browser.content_scroll = 0;
        }
        self.state.room_browser.rooms = rooms;
        self.state.room_browser.selected = selected;
        true
    }

    pub(crate) fn move_room_selection(&mut self, delta: isize) {
        let browser: &mut RoomBrowserState = &mut self.state.room_browser;
        if browser.rooms.is_empty() {
            return;
        }
        let last = browser.rooms.len() - 1;
        browser.selected = browser.selected.saturating_add_signed(delta).min(last);
        browser.content_scroll = 0;
        browser.notice = None;
    }

    pub(crate) fn set_room_tab(&mut self, tab: RoomTab) {
        self.state.room_browser.tab = tab;
        self.state.room_browser.content_scroll = 0;
    }

    /// Sends the composed message through the JSON API, as the host.
    ///
    /// Routing, mention resolution, and the activation budget are the server's
    /// decision; the browser only reports what it answered.
    pub(crate) fn submit_room_post(&mut self) {
        let Some(body) = self.state.room_browser.composer.clone() else {
            return;
        };
        if body.trim().is_empty() {
            self.state.room_browser.notice = Some("write a message before sending".into());
            return;
        }
        let Some(room_id) = self
            .state
            .room_browser
            .selected_room()
            .map(|room| room.room_id.clone())
        else {
            return;
        };

        let response = self.dispatch_runtime_mutation(
            "tui.room.post",
            Method::RoomPost(crate::api::schema::RoomPostParams {
                room_id,
                body,
                caller_pane_id: None,
                recipients: Vec::new(),
                // The composer submits once per keypress and has nothing to
                // retry, so it needs no key.
                client_nonce: None,
                root_id: None,
            }),
        );
        match crate::app::collaboration_api_error(&response) {
            Some(error) => {
                // Keep the draft so an unknown mention can be corrected rather
                // than retyped.
                self.state.room_browser.notice = Some(error);
            }
            None => {
                self.state.room_browser.composer = None;
                self.state.room_browser.content_scroll = 0;
                self.state.room_browser.notice = Some(room_post_receipt(&response));
            }
        }
        self.refresh_room_browser();
    }
}

/// Whether a seat can be handed mail now, using the same gate as dispatch.
fn seat_readiness(
    bound: Option<&str>,
    live: &std::collections::HashMap<String, (AgentStatus, bool, Option<String>)>,
) -> SeatReadiness {
    let Some(instance_id) = bound else {
        return SeatReadiness::NoAgent;
    };
    match live.get(instance_id) {
        None => SeatReadiness::Offline,
        Some((_, true, _)) => SeatReadiness::Working,
        Some((status, false, _)) if crate::collaboration::agent_is_settled(*status) => {
            SeatReadiness::Ready
        }
        Some((AgentStatus::Blocked, false, _)) => SeatReadiness::Blocked,
        Some(_) => SeatReadiness::Working,
    }
}

/// Describes what the post actually did, so "posted" never implies an
/// activation that did not happen.
fn room_post_receipt(response: &str) -> String {
    let recipients = serde_json::from_str::<crate::api::schema::SuccessResponse>(response)
        .ok()
        .and_then(|response| match response.result {
            crate::api::schema::ResponseResult::RoomEvent { event } => Some(event.recipients),
            _ => None,
        })
        .unwrap_or_default();
    match recipients.len() {
        0 => "posted as a room note; nobody was activated".to_string(),
        1 => "posted; 1 member activated".to_string(),
        n => format!("posted; {n} members activated"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Expanding a turn keeps its header where it was clicked, so the body
    /// opens downward instead of pushing the header out of view.
    #[tokio::test]
    async fn expanding_a_turn_keeps_its_header_in_place() {
        let (mut app, room_id) = app_with_room().await;
        app.state.view.sidebar_rect = ratatui::layout::Rect::new(0, 0, 40, 50);
        app.state.view.terminal_area = ratatui::layout::Rect::new(40, 0, 160, 50);
        let long = "word ".repeat(400);
        for index in 0..12 {
            app.state
                .company
                .post(
                    Actor::Host,
                    &room_id,
                    format!("turn {index} {long}"),
                    Vec::new(),
                    None,
                    30 + index,
                )
                .expect("post");
        }
        app.open_room_browser();

        let header_screen_row = |app: &App, event_id: &str| {
            let chrome = crate::ui::room_chrome(&app.state);
            let layout =
                crate::ui::room_layout(app.state.onboarding_full_area(), chrome).expect("layout");
            let rows = crate::ui::conversation_rows(
                &app.state,
                layout.content.width as usize,
                &app.state.palette,
            );
            let room = app.state.room_browser.selected_room().expect("room");
            let event = room
                .events
                .iter()
                .position(|event| event.event_id == event_id)
                .expect("event");
            let header = rows
                .iter()
                .position(|row| row.event == event)
                .expect("header");
            let first = crate::ui::conversation_first_row(
                rows.len(),
                app.state.room_browser.content_scroll,
                layout.content.height as usize,
            );
            header as isize - first as isize
        };

        // Scroll back a little so an older turn's header is on screen.
        app.scroll_room_content(2);
        let target = app
            .state
            .room_browser
            .selected_room()
            .and_then(|room| room.events.iter().rev().nth(3))
            .map(|event| event.event_id.clone())
            .expect("older turn");
        let before = header_screen_row(&app, &target);

        app.toggle_room_expanded(&target);
        assert!(app.state.room_browser.expanded.contains(&target));
        assert_eq!(
            header_screen_row(&app, &target),
            before,
            "the header must not move when its body opens"
        );

        // And every row of the long body is reachable by scrolling down.
        for _ in 0..200 {
            app.scroll_room_content(-1);
        }
        assert_eq!(app.state.room_browser.content_scroll, 0);
    }

    async fn app_with_room() -> (App, String) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let room = app
            .state
            .company
            .create_room(
                &Actor::Host,
                "w1".into(),
                "Product team".into(),
                "Ship it".into(),
                10,
            )
            .expect("room");
        app.state
            .company
            .add_member(&Actor::Host, &room.room_id, "Codex".into(), None, false)
            .expect("seat");
        app.state
            .company
            .add_member(&Actor::Host, &room.room_id, "reviewer".into(), None, false)
            .expect("seat");
        (app, room.room_id)
    }

    #[tokio::test]
    async fn a_pending_menu_request_opens_the_browser() {
        let (mut app, _room_id) = app_with_room().await;
        app.state.request_open_room_browser = true;

        assert!(std::mem::take(&mut app.state.request_open_room_browser));
        app.open_room_browser();
        assert_eq!(app.state.mode, Mode::CompanyRooms);
        assert_eq!(app.state.room_browser.rooms.len(), 1);
    }

    #[tokio::test]
    async fn opening_the_browser_snapshots_rooms_and_switches_mode() {
        let (mut app, _room_id) = app_with_room().await;
        app.open_room_browser();

        assert_eq!(app.state.mode, Mode::CompanyRooms);
        let room = app.state.room_browser.selected_room().expect("room");
        assert_eq!(room.name, "Product team");
        assert_eq!(room.members.len(), 2);
    }

    #[tokio::test]
    async fn a_quiet_room_costs_no_redraw() {
        let (mut app, _room_id) = app_with_room().await;
        app.open_room_browser();
        // The first refresh already applied; nothing changed since.
        assert!(!app.refresh_open_room_browser());
        assert!(!app.refresh_open_room_browser());
    }

    #[tokio::test]
    async fn the_browser_refreshes_only_while_it_is_open() {
        let (mut app, room_id) = app_with_room().await;
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
        // Closed browser: no snapshot work.
        assert!(!app.refresh_open_room_browser());
        assert!(app.state.room_browser.rooms.is_empty());
    }

    #[tokio::test]
    async fn posting_reports_how_many_members_were_activated() {
        let (mut app, _room_id) = app_with_room().await;
        app.open_room_browser();
        app.state.room_browser.composer = Some("@Codex, overview the project".into());
        app.submit_room_post();

        assert_eq!(
            app.state.room_browser.notice.as_deref(),
            Some("posted; 1 member activated")
        );
        assert!(app.state.room_browser.composer.is_none());
        let room = app.state.room_browser.selected_room().expect("room");
        assert_eq!(room.events.len(), 1);
        assert_eq!(room.events[0].recipients, vec!["@Codex".to_string()]);
    }

    #[tokio::test]
    async fn an_untargeted_post_says_it_woke_nobody() {
        let (mut app, _room_id) = app_with_room().await;
        app.open_room_browser();
        app.state.room_browser.composer = Some("just a note".into());
        app.submit_room_post();

        // Without an orchestrator this is a room note, and the receipt must
        // not imply anyone was prompted.
        assert_eq!(
            app.state.room_browser.notice.as_deref(),
            Some("posted as a room note; nobody was activated")
        );
    }

    #[tokio::test]
    async fn an_unknown_mention_keeps_the_draft_and_explains() {
        let (mut app, _room_id) = app_with_room().await;
        app.open_room_browser();
        app.state.room_browser.composer = Some("@nobody do this".into());
        app.submit_room_post();

        assert!(app
            .state
            .room_browser
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("nobody")));
        // The draft survives so it can be corrected rather than retyped.
        assert_eq!(
            app.state.room_browser.composer.as_deref(),
            Some("@nobody do this")
        );
    }

    #[tokio::test]
    async fn an_empty_draft_is_refused_before_it_reaches_the_runtime() {
        let (mut app, _room_id) = app_with_room().await;
        app.open_room_browser();
        app.state.room_browser.composer = Some("   ".into());
        app.submit_room_post();

        assert_eq!(
            app.state.room_browser.notice.as_deref(),
            Some("write a message before sending")
        );
        assert!(app
            .state
            .room_browser
            .selected_room()
            .expect("room")
            .events
            .is_empty());
    }

    #[tokio::test]
    async fn selection_is_clamped_and_resets_the_content_scroll() {
        let (mut app, _room_id) = app_with_room().await;
        app.state
            .company
            .create_room(
                &Actor::Host,
                "w1".into(),
                "Release team".into(),
                String::new(),
                11,
            )
            .expect("second room");
        app.open_room_browser();
        app.state.room_browser.content_scroll = 5;

        app.move_room_selection(1);
        assert_eq!(app.state.room_browser.selected, 1);
        assert_eq!(app.state.room_browser.content_scroll, 0);

        // Past the end clamps rather than panicking.
        app.move_room_selection(10);
        assert_eq!(app.state.room_browser.selected, 1);
        app.move_room_selection(-10);
        assert_eq!(app.state.room_browser.selected, 0);
    }

    #[tokio::test]
    async fn the_conversation_pane_is_bounded_by_recent_history() {
        let (mut app, room_id) = app_with_room().await;
        for index in 0..(MAX_VISIBLE_EVENTS + 20) {
            app.state
                .company
                .post(
                    Actor::Host,
                    &room_id,
                    format!("@Codex step {index}"),
                    Vec::new(),
                    None,
                    20,
                )
                .expect("post");
        }
        app.open_room_browser();
        let room = app.state.room_browser.selected_room().expect("room");
        // Snapshot cost stays bounded however long the room runs, and it is
        // the newest turns that are kept.
        assert_eq!(room.events.len(), MAX_VISIBLE_EVENTS);
        assert!(room
            .events
            .last()
            .expect("last")
            .body
            .contains(&format!("step {}", MAX_VISIBLE_EVENTS + 19)));
    }
}
