use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::api::schema::{
    AgentInfo, AgentLobby, AgentLobbyMember, AgentMessage, AgentMessageBox, AgentMessageParty,
    AgentMessageState, AgentStatus,
};

pub(crate) const MAX_MESSAGE_BODY_BYTES: usize = 16 * 1024;
pub(crate) const MAX_MESSAGES: usize = 1024;
pub(crate) const MAX_LIST_LIMIT: usize = 100;
pub(crate) const MAX_PENDING_MESSAGES_PER_PEER: usize = 64;
pub(crate) const MAX_REPLY_CHAIN_DEPTH: usize = 16;
pub(crate) const MAX_LOBBY_LABEL_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ActiveAgent {
    #[serde(default)]
    agent_label: Option<String>,
    #[serde(default)]
    session_fingerprint: Option<String>,
    party: AgentMessageParty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CollaborationState {
    #[serde(default = "default_next_id")]
    next_instance_id: u64,
    #[serde(default = "default_next_id")]
    next_message_sequence: u64,
    #[serde(default = "default_next_id")]
    next_lobby_id: u64,
    #[serde(default)]
    active_by_terminal: HashMap<String, ActiveAgent>,
    #[serde(default)]
    messages: Vec<AgentMessage>,
    #[serde(default)]
    lobbies: Vec<AgentLobby>,
    #[serde(default)]
    delivery_gates: HashMap<String, DeliveryGate>,
    #[serde(default)]
    delivery_records: HashMap<String, DeliveryRecord>,
    #[serde(default)]
    reconnect_candidates: Vec<ActiveAgent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DeliveryGate {
    message_id: String,
    baseline_state_change_seq: u64,
    activity_seen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DeliveryRecord {
    recipient_instance_id: String,
    terminal_id: String,
    terminal_generation: u32,
    phase: DeliveryPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeliveryPhase {
    Claimed,
    Submitted,
    Uncertain,
}

impl Default for CollaborationState {
    fn default() -> Self {
        Self {
            next_instance_id: 1,
            next_message_sequence: 1,
            next_lobby_id: 1,
            active_by_terminal: HashMap::new(),
            messages: Vec::new(),
            lobbies: Vec::new(),
            delivery_gates: HashMap::new(),
            delivery_records: HashMap::new(),
            reconnect_candidates: Vec::new(),
        }
    }
}

fn default_next_id() -> u64 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CollaborationError {
    BodyEmpty,
    BodyTooLarge,
    NonceTooLarge,
    MailboxFull,
    PeerMailboxFull,
    MessageNotFound,
    NotRecipient,
    NotSender,
    AlreadyAcknowledged,
    ReplyNotVisible,
    ReplyChainTooDeep,
    SameAgent,
    DeliveryStarted,
    LobbyNotFound,
    LobbyNotMember,
    LobbyNotOwner,
    LobbyMemberNotFound,
    LobbyCannotRemoveSelf,
    LobbyLabelEmpty,
    LobbyLabelTooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LobbyMutation {
    Updated(AgentLobby),
    Deleted { lobby_id: String },
}

impl CollaborationError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::BodyEmpty => "message_body_empty",
            Self::BodyTooLarge => "message_body_too_large",
            Self::NonceTooLarge => "message_nonce_too_large",
            Self::MailboxFull => "message_mailbox_full",
            Self::PeerMailboxFull => "message_peer_mailbox_full",
            Self::MessageNotFound => "message_not_found",
            Self::NotRecipient => "message_not_recipient",
            Self::NotSender => "message_not_sender",
            Self::AlreadyAcknowledged => "message_already_acknowledged",
            Self::ReplyNotVisible => "message_reply_not_visible",
            Self::ReplyChainTooDeep => "message_reply_chain_too_deep",
            Self::SameAgent => "agent_same_instance",
            Self::DeliveryStarted => "message_delivery_started",
            Self::LobbyNotFound => "agent_lobby_not_found",
            Self::LobbyNotMember => "agent_lobby_not_member",
            Self::LobbyNotOwner => "agent_lobby_not_owner",
            Self::LobbyMemberNotFound => "agent_lobby_member_not_found",
            Self::LobbyCannotRemoveSelf => "agent_lobby_cannot_remove_self",
            Self::LobbyLabelEmpty => "agent_lobby_label_empty",
            Self::LobbyLabelTooLarge => "agent_lobby_label_too_large",
        }
    }

    pub(crate) fn message(&self) -> &'static str {
        match self {
            Self::BodyEmpty => "message body must not be empty",
            Self::BodyTooLarge => "message body exceeds 16384 bytes",
            Self::NonceTooLarge => "message client nonce exceeds 128 bytes",
            Self::MailboxFull => "message mailbox is full; acknowledge or revoke older messages",
            Self::PeerMailboxFull => {
                "too many unacknowledged messages are retained between this sender and recipient"
            }
            Self::MessageNotFound => "message was not found for this agent",
            Self::NotRecipient => "only the recipient can acknowledge this message",
            Self::NotSender => "only the sender can revoke this message",
            Self::AlreadyAcknowledged => "an acknowledged message cannot be revoked",
            Self::ReplyNotVisible => "reply_to must name a message visible to the sender",
            Self::ReplyChainTooDeep => "message reply chain exceeds the maximum depth of 16",
            Self::SameAgent => "source and target must be different agent instances",
            Self::DeliveryStarted => "message delivery has already started and cannot be revoked",
            Self::LobbyNotFound => "collaboration lobby was not found",
            Self::LobbyNotMember => "only a lobby member can leave this lobby",
            Self::LobbyNotOwner => "only the lobby owner can perform this operation",
            Self::LobbyMemberNotFound => "the requested lobby member was not found",
            Self::LobbyCannotRemoveSelf => "use lobby leave to remove the current agent",
            Self::LobbyLabelEmpty => "lobby label must not be empty",
            Self::LobbyLabelTooLarge => "lobby label exceeds 128 bytes",
        }
    }
}

/// One page of a mailbox listing.
///
/// The mailbox is bounded and a single response is capped, so callers need to
/// know whether to continue rather than guessing from a full page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessagePage {
    pub messages: Vec<AgentMessage>,
    /// More matches exist after this page.
    pub has_more: bool,
    /// Pass back as `after_sequence` to fetch the next page.
    pub next_after_sequence: Option<u64>,
}

impl CollaborationState {
    /// Convenience wrapper for tests that do not care which lobbies changed.
    /// Production paths use [`Self::ensure_agent_with_updates`] so the
    /// pane-scaled delivery loop does not clone unchanged lobbies.
    #[cfg(test)]
    pub(crate) fn ensure_agent(&mut self, agent: &AgentInfo) -> AgentMessageParty {
        self.ensure_agent_with_updates(agent).0
    }

    /// Registers or refreshes an agent and reports only lobbies whose visible
    /// member record changed. This avoids cloning every lobby in the
    /// pane-scaled delivery loop when nothing changed.
    pub(crate) fn ensure_agent_with_updates(
        &mut self,
        agent: &AgentInfo,
    ) -> (AgentMessageParty, Vec<AgentLobby>) {
        let agent_label = agent.agent.clone();
        let session_fingerprint = agent_session_fingerprint(agent);
        let existing_party =
            if let Some(active) = self.active_by_terminal.get_mut(&agent.terminal_id) {
                let same_agent = active.agent_label == agent_label;
                let compatible_session = active.session_fingerprint == session_fingerprint
                    || active.session_fingerprint.is_none()
                    || session_fingerprint.is_none();
                if same_agent && compatible_session {
                    if active.session_fingerprint.is_none() {
                        active.session_fingerprint.clone_from(&session_fingerprint);
                    }
                    active.party.name.clone_from(&agent.name);
                    active.party.agent.clone_from(&agent.agent);
                    active.party.pane_id.clone_from(&agent.pane_id);
                    Some(active.party.clone())
                } else {
                    None
                }
            } else {
                None
            };
        if let Some(party) = existing_party {
            let updates = self.refresh_lobby_member(&party);
            return (party, updates);
        }

        if let Some(fingerprint) = session_fingerprint.as_deref() {
            let mut matches = self
                .reconnect_candidates
                .iter()
                .enumerate()
                .filter(|(_, candidate)| {
                    candidate.agent_label == agent_label
                        && candidate.session_fingerprint.as_deref() == Some(fingerprint)
                })
                .map(|(index, _)| index);
            if let Some(index) = matches.next().filter(|_| matches.next().is_none()) {
                let mut restored = self.reconnect_candidates.remove(index);
                restored.party.terminal_id.clone_from(&agent.terminal_id);
                restored.party.name.clone_from(&agent.name);
                restored.party.agent.clone_from(&agent.agent);
                restored.party.pane_id.clone_from(&agent.pane_id);
                let party = restored.party.clone();
                self.active_by_terminal
                    .insert(agent.terminal_id.clone(), restored);
                let updates = self.refresh_lobby_member(&party);
                return (party, updates);
            }
        }

        let instance_id = format!("agent_{:x}", self.next_instance_id);
        self.next_instance_id = self.next_instance_id.saturating_add(1);
        let party = AgentMessageParty {
            instance_id,
            terminal_id: agent.terminal_id.clone(),
            name: agent.name.clone(),
            agent: agent.agent.clone(),
            pane_id: agent.pane_id.clone(),
        };
        self.active_by_terminal.insert(
            agent.terminal_id.clone(),
            ActiveAgent {
                agent_label,
                session_fingerprint,
                party: party.clone(),
            },
        );
        let updates = self.refresh_lobby_member(&party);
        (party, updates)
    }

    pub(crate) fn retire_terminal(&mut self, terminal_id: &str) -> Vec<AgentLobby> {
        let Some(retired) = self.active_by_terminal.remove(terminal_id) else {
            return Vec::new();
        };
        self.delivery_gates.remove(&retired.party.instance_id);
        let mut updates = Vec::new();
        for lobby in &mut self.lobbies {
            if let Some(member) = lobby
                .members
                .iter_mut()
                .find(|member| member.party.instance_id == retired.party.instance_id)
            {
                if member.active {
                    member.active = false;
                    lobby.revision = lobby.revision.saturating_add(1);
                    updates.push(lobby.clone());
                }
            }
        }
        updates
    }

    /// Converts a serialized live state into a cold-start state. Runtime
    /// bindings cannot survive a stopped process; uniquely identified resumed
    /// native sessions may reclaim their old incarnation later.
    pub(crate) fn prepare_for_cold_restore(&mut self) {
        self.reconnect_candidates
            .extend(self.active_by_terminal.drain().map(|(_, active)| active));
        self.delivery_gates.clear();
        for lobby in &mut self.lobbies {
            let had_active_member = lobby.members.iter().any(|member| member.active);
            for member in &mut lobby.members {
                member.active = false;
            }
            if had_active_member {
                lobby.revision = lobby.revision.saturating_add(1);
            }
        }
        for (message_id, record) in &mut self.delivery_records {
            if record.phase == DeliveryPhase::Claimed {
                record.phase = DeliveryPhase::Uncertain;
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|message| message.message_id == *message_id)
                {
                    message.delivery_uncertain = true;
                }
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.lobbies.is_empty()
    }

    fn refresh_lobby_member(&mut self, party: &AgentMessageParty) -> Vec<AgentLobby> {
        let mut updates = Vec::new();
        for lobby in &mut self.lobbies {
            if let Some(member) = lobby
                .members
                .iter_mut()
                .find(|member| member.party.instance_id == party.instance_id)
            {
                let changed = member.party != *party || !member.active;
                member.party.clone_from(party);
                member.active = true;
                if changed {
                    lobby.revision = lobby.revision.saturating_add(1);
                    updates.push(lobby.clone());
                }
            }
        }
        updates
    }

    pub(crate) fn connect(
        &mut self,
        first: AgentMessageParty,
        second: AgentMessageParty,
        now_unix_ms: u64,
    ) -> Result<AgentLobby, CollaborationError> {
        if first.instance_id == second.instance_id {
            return Err(CollaborationError::SameAgent);
        }
        let mut ids = [first.instance_id.as_str(), second.instance_id.as_str()];
        ids.sort_unstable();
        if let Some(lobby) = self.lobbies.iter().find(|lobby| {
            lobby.workspace_id.is_none()
                && lobby.members.len() == 2
                && lobby
                    .members
                    .iter()
                    .all(|member| ids.contains(&member.party.instance_id.as_str()))
        }) {
            return Ok(lobby.clone());
        }
        self.connect_many(vec![first, second], None, now_unix_ms)
    }

    pub(crate) fn connect_many(
        &mut self,
        mut parties: Vec<AgentMessageParty>,
        label: Option<String>,
        now_unix_ms: u64,
    ) -> Result<AgentLobby, CollaborationError> {
        let owner_instance_id = parties.first().map(|party| party.instance_id.clone());
        parties.sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
        parties.dedup_by(|left, right| left.instance_id == right.instance_id);
        if parties.len() < 2 {
            return Err(CollaborationError::SameAgent);
        }
        let ids = parties
            .iter()
            .map(|party| party.instance_id.as_str())
            .collect::<Vec<_>>();
        if let Some(lobby) = self.lobbies.iter().find(|lobby| {
            lobby.workspace_id.is_none()
                && lobby.members.len() == ids.len()
                && lobby
                    .members
                    .iter()
                    .all(|member| ids.contains(&member.party.instance_id.as_str()))
        }) {
            return Ok(lobby.clone());
        }
        let lobby_id = format!("lobby_{:x}", self.next_lobby_id);
        self.next_lobby_id = self.next_lobby_id.saturating_add(1);
        let label = label.unwrap_or_else(|| {
            parties
                .iter()
                .map(party_display_name)
                .collect::<Vec<_>>()
                .join(" ↔ ")
        });
        let lobby = AgentLobby {
            lobby_id,
            label,
            owner_instance_id,
            workspace_id: None,
            members: parties
                .into_iter()
                .map(|party| AgentLobbyMember {
                    active: self
                        .active_by_terminal
                        .values()
                        .any(|active| active.party.instance_id == party.instance_id),
                    party,
                })
                .collect(),
            created_at_unix_ms: now_unix_ms,
            revision: 1,
        };
        self.lobbies.push(lobby.clone());
        Ok(lobby)
    }

    /// Creates or refreshes the single collaboration space associated with a
    /// workspace. Former members remain as offline history; current agents are
    /// added or refreshed without changing the stable lobby ID.
    pub(crate) fn sync_workspace_agents(
        &mut self,
        workspace_id: String,
        label: String,
        mut parties: Vec<AgentMessageParty>,
        now_unix_ms: u64,
    ) -> Result<AgentLobby, CollaborationError> {
        parties.sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
        parties.dedup_by(|left, right| left.instance_id == right.instance_id);
        if parties.is_empty() {
            return Err(CollaborationError::SameAgent);
        }

        if let Some(lobby) = self
            .lobbies
            .iter_mut()
            .find(|lobby| lobby.workspace_id.as_deref() == Some(workspace_id.as_str()))
        {
            let mut changed = false;
            for party in parties {
                let active = self
                    .active_by_terminal
                    .values()
                    .any(|candidate| candidate.party.instance_id == party.instance_id);
                if let Some(member) = lobby
                    .members
                    .iter_mut()
                    .find(|member| member.party.instance_id == party.instance_id)
                {
                    if member.party != party || member.active != active {
                        member.party = party;
                        member.active = active;
                        changed = true;
                    }
                } else {
                    lobby.members.push(AgentLobbyMember { party, active });
                    changed = true;
                }
            }
            if lobby.label != label {
                lobby.label = label;
                changed = true;
            }
            if lobby.owner_instance_id.is_none() {
                lobby.owner_instance_id = lobby
                    .members
                    .iter()
                    .find(|member| member.active)
                    .or_else(|| lobby.members.first())
                    .map(|member| member.party.instance_id.clone());
                changed = true;
            }
            if changed {
                lobby.revision = lobby.revision.saturating_add(1);
            }
            return Ok(lobby.clone());
        }

        let owner_instance_id = parties.first().map(|party| party.instance_id.clone());
        let lobby_id = format!("lobby_{:x}", self.next_lobby_id);
        self.next_lobby_id = self.next_lobby_id.saturating_add(1);
        let lobby = AgentLobby {
            lobby_id,
            label,
            owner_instance_id,
            workspace_id: Some(workspace_id),
            members: parties
                .into_iter()
                .map(|party| AgentLobbyMember {
                    active: self
                        .active_by_terminal
                        .values()
                        .any(|active| active.party.instance_id == party.instance_id),
                    party,
                })
                .collect(),
            created_at_unix_ms: now_unix_ms,
            revision: 1,
        };
        self.lobbies.push(lobby.clone());
        Ok(lobby)
    }

    pub(crate) fn rename_lobby(
        &mut self,
        caller: &AgentMessageParty,
        lobby_id: &str,
        label: String,
    ) -> Result<AgentLobby, CollaborationError> {
        let label = label.trim();
        if label.is_empty() {
            return Err(CollaborationError::LobbyLabelEmpty);
        }
        if label.len() > MAX_LOBBY_LABEL_BYTES {
            return Err(CollaborationError::LobbyLabelTooLarge);
        }
        let lobby = self.lobby_for_owner_mut(caller, lobby_id)?;
        if lobby.label != label {
            lobby.label = label.to_string();
            lobby.revision = lobby.revision.saturating_add(1);
        }
        Ok(lobby.clone())
    }

    pub(crate) fn remove_lobby_member(
        &mut self,
        caller: &AgentMessageParty,
        lobby_id: &str,
        member_instance_id: &str,
    ) -> Result<LobbyMutation, CollaborationError> {
        if caller.instance_id == member_instance_id {
            return Err(CollaborationError::LobbyCannotRemoveSelf);
        }
        let lobby = self.lobby_for_owner_mut(caller, lobby_id)?;
        let Some(index) = lobby
            .members
            .iter()
            .position(|member| member.party.instance_id == member_instance_id)
        else {
            return Err(CollaborationError::LobbyMemberNotFound);
        };
        lobby.members.remove(index);
        lobby.revision = lobby.revision.saturating_add(1);
        Ok(LobbyMutation::Updated(lobby.clone()))
    }

    pub(crate) fn leave_lobby(
        &mut self,
        caller: &AgentMessageParty,
        lobby_id: &str,
    ) -> Result<LobbyMutation, CollaborationError> {
        let Some(index) = self
            .lobbies
            .iter()
            .position(|lobby| lobby.lobby_id == lobby_id)
        else {
            return Err(CollaborationError::LobbyNotFound);
        };
        let lobby = &mut self.lobbies[index];
        let Some(member_index) = lobby
            .members
            .iter()
            .position(|member| member.party.instance_id == caller.instance_id)
        else {
            return Err(CollaborationError::LobbyNotMember);
        };
        let was_owner = lobby_owner_instance_id(lobby) == Some(caller.instance_id.as_str());
        lobby.members.remove(member_index);
        if lobby.members.is_empty() {
            self.lobbies.remove(index);
            return Ok(LobbyMutation::Deleted {
                lobby_id: lobby_id.to_string(),
            });
        }
        if was_owner {
            lobby.owner_instance_id = lobby
                .members
                .iter()
                .find(|member| member.active)
                .or_else(|| lobby.members.first())
                .map(|member| member.party.instance_id.clone());
        }
        lobby.revision = lobby.revision.saturating_add(1);
        Ok(LobbyMutation::Updated(lobby.clone()))
    }

    pub(crate) fn delete_lobby(
        &mut self,
        caller: &AgentMessageParty,
        lobby_id: &str,
    ) -> Result<LobbyMutation, CollaborationError> {
        let Some(index) = self
            .lobbies
            .iter()
            .position(|lobby| lobby.lobby_id == lobby_id)
        else {
            return Err(CollaborationError::LobbyNotFound);
        };
        if lobby_owner_instance_id(&self.lobbies[index]) != Some(caller.instance_id.as_str()) {
            return Err(CollaborationError::LobbyNotOwner);
        }
        self.lobbies.remove(index);
        Ok(LobbyMutation::Deleted {
            lobby_id: lobby_id.to_string(),
        })
    }

    fn lobby_for_owner_mut(
        &mut self,
        caller: &AgentMessageParty,
        lobby_id: &str,
    ) -> Result<&mut AgentLobby, CollaborationError> {
        let lobby = self
            .lobbies
            .iter_mut()
            .find(|lobby| lobby.lobby_id == lobby_id)
            .ok_or(CollaborationError::LobbyNotFound)?;
        if lobby_owner_instance_id(lobby) != Some(caller.instance_id.as_str()) {
            return Err(CollaborationError::LobbyNotOwner);
        }
        if lobby.owner_instance_id.is_none() {
            lobby.owner_instance_id = Some(caller.instance_id.clone());
        }
        Ok(lobby)
    }

    pub(crate) fn lobbies(&self) -> Vec<AgentLobby> {
        self.lobbies.clone()
    }

    /// Every message exchanged between members of `lobby`, oldest first.
    ///
    /// This is the lobby's conversation: mail is addressed agent-to-agent, so a
    /// lobby thread is the set of messages whose sender and recipient are both
    /// members. Read-only, and never marks anything observed — the operator
    /// watching a lobby is not the recipient consuming their mail.
    pub(crate) fn lobby_messages(&self, lobby_id: &str) -> Vec<AgentMessage> {
        let Some(lobby) = self.lobbies.iter().find(|lobby| lobby.lobby_id == lobby_id) else {
            return Vec::new();
        };
        let members = lobby
            .members
            .iter()
            .map(|member| member.party.instance_id.as_str())
            .collect::<Vec<_>>();
        self.messages
            .iter()
            .filter(|message| {
                members.contains(&message.sender.instance_id.as_str())
                    && members.contains(&message.recipient.instance_id.as_str())
            })
            .cloned()
            .collect()
    }

    /// Instance ID currently bound to a terminal, if any.
    ///
    /// Read-only: unlike `ensure_agent` this never registers a new
    /// incarnation, so a dispatcher can match seats without side effects.
    pub(crate) fn instance_for_terminal(&self, terminal_id: &str) -> Option<String> {
        self.active_by_terminal
            .get(terminal_id)
            .map(|active| active.party.instance_id.clone())
    }

    /// Incarnation this agent holds, or would reclaim on its next
    /// registration.
    ///
    /// Read-only mirror of the reconnect rule in
    /// [`Self::ensure_agent_with_updates`]: a resumed session reclaims an old
    /// incarnation only when exactly one candidate matches its agent kind and
    /// session fingerprint. Used where a caller needs to know who an agent is
    /// without registering it.
    pub(crate) fn instance_for_agent(&self, agent: &AgentInfo) -> Option<String> {
        if let Some(instance_id) = self.instance_for_terminal(&agent.terminal_id) {
            return Some(instance_id);
        }
        let fingerprint = agent_session_fingerprint(agent)?;
        let mut matches = self.reconnect_candidates.iter().filter(|candidate| {
            candidate.agent_label == agent.agent
                && candidate.session_fingerprint.as_deref() == Some(fingerprint.as_str())
        });
        let first = matches.next()?;
        matches
            .next()
            .is_none()
            .then(|| first.party.instance_id.clone())
    }

    /// Whether an incarnation is still live.
    ///
    /// A message addressed to a retired incarnation can never be acknowledged,
    /// because a replacement agent deliberately inherits no mail. Callers use
    /// this to say so instead of showing it as merely delivered.
    pub(crate) fn instance_is_active(&self, instance_id: &str) -> bool {
        self.active_by_terminal
            .values()
            .any(|active| active.party.instance_id == instance_id)
    }

    /// Whether the live agents on two terminals already share an active lobby.
    ///
    /// Read-only: unlike [`Self::ensure_agent`] this never registers a new
    /// incarnation, so callers can ask about connection status without
    /// materializing identities for agents nobody has messaged yet.
    pub(crate) fn terminals_share_active_lobby(&self, first: &str, second: &str) -> bool {
        let Some(first) = self.active_by_terminal.get(first) else {
            return false;
        };
        let Some(second) = self.active_by_terminal.get(second) else {
            return false;
        };
        self.parties_share_active_lobby(&first.party.instance_id, &second.party.instance_id)
    }

    pub(crate) fn has_delivery_work(&self) -> bool {
        !self.delivery_gates.is_empty()
            || self.messages.iter().any(|message| {
                message.state == AgentMessageState::Pending
                    && !self.delivery_records.contains_key(&message.message_id)
                    && self.parties_share_active_lobby(
                        &message.sender.instance_id,
                        &message.recipient.instance_id,
                    )
            })
    }

    pub(crate) fn send(
        &mut self,
        sender: AgentMessageParty,
        recipient: AgentMessageParty,
        body: String,
        reply_to: Option<String>,
        client_nonce: Option<String>,
        now_unix_ms: u64,
    ) -> Result<AgentMessage, CollaborationError> {
        if sender.instance_id == recipient.instance_id {
            return Err(CollaborationError::SameAgent);
        }
        if body.is_empty() {
            return Err(CollaborationError::BodyEmpty);
        }
        if body.len() > MAX_MESSAGE_BODY_BYTES {
            return Err(CollaborationError::BodyTooLarge);
        }
        if client_nonce.as_ref().is_some_and(|nonce| nonce.len() > 128) {
            return Err(CollaborationError::NonceTooLarge);
        }
        if let Some(nonce) = client_nonce.as_deref() {
            if let Some(message) = self.messages.iter().find(|message| {
                message.sender.instance_id == sender.instance_id
                    && message.client_nonce.as_deref() == Some(nonce)
            }) {
                return Ok(message.clone());
            }
        }
        if let Some(reply_id) = reply_to.as_deref() {
            let Some(reply) = self.messages.iter().find(|message| {
                message.message_id == reply_id && message_visible_to(message, &sender.instance_id)
            }) else {
                return Err(CollaborationError::ReplyNotVisible);
            };
            if self.reply_chain_depth(reply) >= MAX_REPLY_CHAIN_DEPTH {
                return Err(CollaborationError::ReplyChainTooDeep);
            }
        }
        let pending_for_peer = self.messages.iter().filter(|message| {
            message.sender.instance_id == sender.instance_id
                && message.recipient.instance_id == recipient.instance_id
                && !matches!(
                    message.state,
                    AgentMessageState::Acknowledged | AgentMessageState::Revoked
                )
        });
        if pending_for_peer.count() >= MAX_PENDING_MESSAGES_PER_PEER {
            return Err(CollaborationError::PeerMailboxFull);
        }
        self.make_capacity()?;

        let sequence = self.next_message_sequence;
        self.next_message_sequence = self.next_message_sequence.saturating_add(1);
        let message = AgentMessage {
            message_id: format!("msg_{sequence:x}"),
            sequence,
            sender,
            recipient,
            body: Some(body),
            reply_to,
            state: AgentMessageState::Pending,
            created_at_unix_ms: now_unix_ms,
            observed_at_unix_ms: None,
            injected_at_unix_ms: None,
            acknowledged_at_unix_ms: None,
            revoked_at_unix_ms: None,
            delivery_uncertain: false,
            client_nonce,
        };
        self.messages.push(message.clone());
        Ok(message)
    }

    pub(crate) fn list(
        &mut self,
        caller: &AgentMessageParty,
        mailbox: AgentMessageBox,
        after_sequence: u64,
        limit: Option<u32>,
        unacknowledged_only: bool,
        now_unix_ms: u64,
    ) -> MessagePage {
        let limit = limit.unwrap_or(50).clamp(1, MAX_LIST_LIMIT as u32) as usize;
        let mut selected = Vec::new();
        let mut has_more = false;
        for message in &mut self.messages {
            if message.sequence <= after_sequence
                || !mailbox_matches(message, &caller.instance_id, mailbox)
                || (unacknowledged_only && message.state == AgentMessageState::Acknowledged)
            {
                continue;
            }
            if selected.len() == limit {
                // One match beyond the page proves there is more to fetch,
                // without observing it — observation is a recipient action and
                // must not be a side effect of paging.
                has_more = true;
                break;
            }
            observe_if_recipient(message, &caller.instance_id, now_unix_ms);
            selected.push(message.clone());
        }
        let next_after_sequence = selected.last().map(|message| message.sequence);
        MessagePage {
            messages: selected,
            has_more,
            next_after_sequence,
        }
    }

    pub(crate) fn get(
        &mut self,
        caller: &AgentMessageParty,
        message_id: &str,
        now_unix_ms: u64,
    ) -> Result<AgentMessage, CollaborationError> {
        let message = self
            .messages
            .iter_mut()
            .find(|message| {
                message.message_id == message_id && message_visible_to(message, &caller.instance_id)
            })
            .ok_or(CollaborationError::MessageNotFound)?;
        observe_if_recipient(message, &caller.instance_id, now_unix_ms);
        Ok(message.clone())
    }

    pub(crate) fn acknowledge(
        &mut self,
        caller: &AgentMessageParty,
        message_id: &str,
        now_unix_ms: u64,
    ) -> Result<AgentMessage, CollaborationError> {
        let result = {
            let message = self
                .messages
                .iter_mut()
                .find(|message| message.message_id == message_id)
                .ok_or(CollaborationError::MessageNotFound)?;
            if message.recipient.instance_id != caller.instance_id {
                return Err(CollaborationError::NotRecipient);
            }
            if message.state == AgentMessageState::Revoked {
                return Ok(message.clone());
            }
            message.observed_at_unix_ms.get_or_insert(now_unix_ms);
            message.acknowledged_at_unix_ms.get_or_insert(now_unix_ms);
            message.state = AgentMessageState::Acknowledged;
            message.clone()
        };
        self.delivery_records.remove(message_id);
        Ok(result)
    }

    pub(crate) fn revoke(
        &mut self,
        caller: &AgentMessageParty,
        message_id: &str,
        now_unix_ms: u64,
    ) -> Result<AgentMessage, CollaborationError> {
        let message = self
            .messages
            .iter_mut()
            .find(|message| message.message_id == message_id)
            .ok_or(CollaborationError::MessageNotFound)?;
        if message.sender.instance_id != caller.instance_id {
            return Err(CollaborationError::NotSender);
        }
        if message.state == AgentMessageState::Acknowledged {
            return Err(CollaborationError::AlreadyAcknowledged);
        }
        if message.state == AgentMessageState::Injected || message.delivery_uncertain {
            return Err(CollaborationError::DeliveryStarted);
        }
        if message.state != AgentMessageState::Revoked {
            message.body = None;
            message.revoked_at_unix_ms = Some(now_unix_ms);
            message.state = AgentMessageState::Revoked;
        }
        Ok(message.clone())
    }

    pub(crate) fn latest_sequence(&self) -> u64 {
        self.next_message_sequence.saturating_sub(1)
    }

    pub(crate) fn note_recipient_state(
        &mut self,
        instance_id: &str,
        state: AgentStatus,
        state_change_seq: u64,
    ) {
        let should_release = self
            .delivery_gates
            .get_mut(instance_id)
            .is_some_and(|gate| {
                if !agent_is_settled(state) && state_change_seq > gate.baseline_state_change_seq {
                    gate.activity_seen = true;
                }
                gate.activity_seen
                    && agent_is_settled(state)
                    && state_change_seq > gate.baseline_state_change_seq
            });
        if should_release {
            self.delivery_gates.remove(instance_id);
        }
    }

    pub(crate) fn delivery_candidate(&self, recipient: &AgentMessageParty) -> Option<AgentMessage> {
        if self.delivery_gates.contains_key(&recipient.instance_id) {
            return None;
        }
        self.messages
            .iter()
            .find(|message| {
                message.recipient.instance_id == recipient.instance_id
                    && message.state == AgentMessageState::Pending
                    && !self.delivery_records.contains_key(&message.message_id)
                    && self.parties_share_active_lobby(
                        &message.sender.instance_id,
                        &message.recipient.instance_id,
                    )
            })
            .cloned()
    }

    pub(crate) fn claim_delivery(
        &mut self,
        message_id: &str,
        recipient_instance_id: &str,
        terminal_id: &str,
        terminal_generation: u32,
    ) -> Result<(), CollaborationError> {
        let message = self
            .messages
            .iter()
            .find(|message| message.message_id == message_id)
            .ok_or(CollaborationError::MessageNotFound)?;
        if message.recipient.instance_id != recipient_instance_id {
            return Err(CollaborationError::NotRecipient);
        }
        if message.state != AgentMessageState::Pending || message.delivery_uncertain {
            return Err(CollaborationError::DeliveryStarted);
        }
        self.delivery_records.insert(
            message_id.to_string(),
            DeliveryRecord {
                recipient_instance_id: recipient_instance_id.to_string(),
                terminal_id: terminal_id.to_string(),
                terminal_generation,
                phase: DeliveryPhase::Claimed,
            },
        );
        Ok(())
    }

    pub(crate) fn release_delivery_claim(&mut self, message_id: &str) {
        if self
            .delivery_records
            .get(message_id)
            .is_some_and(|record| record.phase == DeliveryPhase::Claimed)
        {
            self.delivery_records.remove(message_id);
        }
    }

    pub(crate) fn mark_injected(
        &mut self,
        message_id: &str,
        recipient_instance_id: &str,
        baseline_state_change_seq: u64,
        now_unix_ms: u64,
    ) -> Result<AgentMessage, CollaborationError> {
        let message = self
            .messages
            .iter_mut()
            .find(|message| message.message_id == message_id)
            .ok_or(CollaborationError::MessageNotFound)?;
        if message.recipient.instance_id != recipient_instance_id {
            return Err(CollaborationError::NotRecipient);
        }
        if message.state != AgentMessageState::Pending {
            return Ok(message.clone());
        }
        message.observed_at_unix_ms.get_or_insert(now_unix_ms);
        message.injected_at_unix_ms.get_or_insert(now_unix_ms);
        message.state = AgentMessageState::Injected;
        if let Some(record) = self.delivery_records.get_mut(message_id) {
            record.phase = DeliveryPhase::Submitted;
        }
        self.delivery_gates.insert(
            recipient_instance_id.to_string(),
            DeliveryGate {
                message_id: message_id.to_string(),
                baseline_state_change_seq,
                activity_seen: false,
            },
        );
        Ok(message.clone())
    }

    pub(crate) fn message(&self, message_id: &str) -> Option<AgentMessage> {
        self.messages
            .iter()
            .find(|message| message.message_id == message_id)
            .cloned()
    }

    fn parties_share_active_lobby(&self, first: &str, second: &str) -> bool {
        self.lobbies.iter().any(|lobby| {
            let first_active = lobby
                .members
                .iter()
                .any(|member| member.active && member.party.instance_id == first);
            let second_active = lobby
                .members
                .iter()
                .any(|member| member.active && member.party.instance_id == second);
            first_active && second_active
        })
    }

    fn make_capacity(&mut self) -> Result<(), CollaborationError> {
        if self.messages.len() < MAX_MESSAGES {
            return Ok(());
        }
        if let Some(index) = self.messages.iter().position(|message| {
            matches!(
                message.state,
                AgentMessageState::Acknowledged | AgentMessageState::Revoked
            )
        }) {
            let removed = self.messages.remove(index);
            self.delivery_records.remove(&removed.message_id);
            Ok(())
        } else {
            Err(CollaborationError::MailboxFull)
        }
    }

    fn reply_chain_depth(&self, message: &AgentMessage) -> usize {
        let mut depth = 1;
        let mut current = message;
        while depth < MAX_REPLY_CHAIN_DEPTH {
            let Some(reply_to) = current.reply_to.as_deref() else {
                return depth;
            };
            let Some(parent) = self
                .messages
                .iter()
                .find(|candidate| candidate.message_id == reply_to)
            else {
                return depth;
            };
            depth += 1;
            current = parent;
        }
        depth
    }
}

/// Whether an agent has settled and can be handed new input.
///
/// `Done` is as ready as `Idle`: the agent finished its turn and is sitting at
/// its prompt. Treating only `Idle` as ready left mail queued behind a
/// finished agent until something else made the runtime recompute its state —
/// in practice, until the operator opened that pane.
pub(crate) fn agent_is_settled(state: AgentStatus) -> bool {
    matches!(state, AgentStatus::Idle | AgentStatus::Done)
}

pub(crate) fn party_display_name(party: &AgentMessageParty) -> String {
    party
        .name
        .clone()
        .or_else(|| party.agent.clone())
        .unwrap_or_else(|| party.instance_id.clone())
}

fn lobby_owner_instance_id(lobby: &AgentLobby) -> Option<&str> {
    lobby.owner_instance_id.as_deref().or_else(|| {
        lobby
            .members
            .first()
            .map(|member| member.party.instance_id.as_str())
    })
}

fn agent_session_fingerprint(agent: &AgentInfo) -> Option<String> {
    agent.agent_session.as_ref().map(|session| {
        format!(
            "session:{}:{}:{:?}:{}",
            session.source, session.agent, session.kind, session.value
        )
    })
}

fn message_visible_to(message: &AgentMessage, instance_id: &str) -> bool {
    message.sender.instance_id == instance_id || message.recipient.instance_id == instance_id
}

fn mailbox_matches(message: &AgentMessage, instance_id: &str, mailbox: AgentMessageBox) -> bool {
    match mailbox {
        AgentMessageBox::Inbox => message.recipient.instance_id == instance_id,
        AgentMessageBox::Outbox => message.sender.instance_id == instance_id,
        AgentMessageBox::All => message_visible_to(message, instance_id),
    }
}

fn observe_if_recipient(message: &mut AgentMessage, instance_id: &str, now_unix_ms: u64) {
    if message.recipient.instance_id == instance_id && message.state == AgentMessageState::Pending {
        message.observed_at_unix_ms = Some(now_unix_ms);
        message.state = AgentMessageState::Observed;
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_resumed_session_is_recognised_before_it_registers() {
        // The room browser asks who an agent is without registering it, and
        // must give the same answer registration would.
        let mut state = CollaborationState::default();
        let mut luna = agent("term_a", "w1:p1", "luna", "codex");
        luna.agent_session = Some(crate::api::schema::AgentSessionInfo {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Id,
            value: "luna-session".into(),
        });
        let original = state.ensure_agent(&luna).instance_id;
        state.prepare_for_cold_restore();

        // After the restart the agent is back in a new terminal, unregistered.
        luna.terminal_id = "term_b".into();
        assert!(state.instance_for_terminal("term_b").is_none());
        assert_eq!(
            state.instance_for_agent(&luna).as_deref(),
            Some(original.as_str())
        );
        // And registration agrees.
        assert_eq!(state.ensure_agent(&luna).instance_id, original);

        // A different session is not mistaken for it.
        let mut stranger = agent("term_c", "w1:p2", "other", "codex");
        stranger.agent_session = Some(crate::api::schema::AgentSessionInfo {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Id,
            value: "someone-else".into(),
        });
        assert!(state.instance_for_agent(&stranger).is_none());
    }
    use super::*;
    use crate::api::schema::AgentStatus;

    fn agent(terminal: &str, pane: &str, name: &str, kind: &str) -> AgentInfo {
        AgentInfo {
            terminal_id: terminal.into(),
            name: Some(name.into()),
            agent: Some(kind.into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            display_agent: None,
            agent_status: AgentStatus::Idle,
            screen_detection_skipped: false,
            state_labels: HashMap::new(),
            tokens: HashMap::new(),
            agent_session: None,
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            pane_id: pane.into(),
            focused: false,
            launch_pending: false,
            interactive_ready: true,
            state_change_seq: 1,
            cwd: None,
            foreground_cwd: None,
            revision: 0,
        }
    }

    #[test]
    fn pane_moves_and_renames_keep_the_incarnation_but_release_rotates_it() {
        let mut state = CollaborationState::default();
        let first = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let moved = state.ensure_agent(&agent("term_1", "w2:p9", "lead", "codex"));
        assert_eq!(first.instance_id, moved.instance_id);
        assert_eq!(moved.pane_id, "w2:p9");

        state.retire_terminal("term_1");
        let replacement = state.ensure_agent(&agent("term_1", "w2:p9", "lead", "codex"));
        assert_ne!(first.instance_id, replacement.instance_id);
    }

    #[test]
    fn a_late_session_report_does_not_rotate_the_live_agent() {
        let mut state = CollaborationState::default();
        let mut info = agent("term_1", "w1:p1", "author", "codex");
        let first = state.ensure_agent(&info);
        info.agent_session = Some(crate::api::schema::AgentSessionInfo {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            kind: crate::agent_resume::AgentSessionRefKind::Id,
            value: "session-1".into(),
        });
        let anchored = state.ensure_agent(&info);
        assert_eq!(first.instance_id, anchored.instance_id);

        info.agent_session.as_mut().unwrap().value = "session-2".into();
        let replacement = state.ensure_agent(&info);
        assert_ne!(first.instance_id, replacement.instance_id);
    }

    #[test]
    fn recipient_observes_and_acknowledges_while_sender_can_revoke_before_ack() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        let message = state
            .send(
                sender.clone(),
                recipient.clone(),
                "review".into(),
                None,
                None,
                10,
            )
            .expect("send should succeed");
        let inbox = state
            .list(&recipient, AgentMessageBox::Inbox, 0, None, false, 20)
            .messages;
        assert_eq!(inbox[0].state, AgentMessageState::Observed);

        let revoked = state
            .revoke(&sender, &message.message_id, 30)
            .expect("sender can revoke before ack");
        assert_eq!(revoked.state, AgentMessageState::Revoked);
        assert!(revoked.body.is_none());
        assert_eq!(
            state
                .acknowledge(&recipient, &message.message_id, 40)
                .expect("ack of tombstone is idempotent")
                .state,
            AgentMessageState::Revoked
        );
    }

    #[test]
    fn client_nonce_deduplicates_uncertain_retries() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        let first = state
            .send(
                sender.clone(),
                recipient.clone(),
                "review".into(),
                None,
                Some("retry-1".into()),
                10,
            )
            .expect("send should succeed");
        let retry = state
            .send(
                sender,
                recipient,
                "different retry body".into(),
                None,
                Some("retry-1".into()),
                20,
            )
            .expect("retry should return original");
        assert_eq!(retry.message_id, first.message_id);
        assert_eq!(state.messages.len(), 1);
    }

    /// The crash window: the runtime queues bytes into the recipient's
    /// terminal and only then marks the message injected. A crash in between
    /// leaves the terminal holding text the runtime still thinks is pending.
    #[test]
    fn a_crash_after_claiming_delivery_leaves_the_message_uncertain_not_pending() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        state
            .connect(sender.clone(), recipient.clone(), 1)
            .expect("lobby");
        let message = state
            .send(
                sender,
                recipient.clone(),
                "do the thing".into(),
                None,
                None,
                2,
            )
            .expect("message");

        // Claim, then "crash" before mark_injected by restoring cold.
        state
            .claim_delivery(&message.message_id, &recipient.instance_id, "term_2", 7)
            .expect("claim");
        let encoded = serde_json::to_string(&state).expect("serialize");
        let mut restored: CollaborationState = serde_json::from_str(&encoded).expect("deserialize");
        restored.prepare_for_cold_restore();

        let recovered = restored.message(&message.message_id).expect("message");
        assert!(
            recovered.delivery_uncertain,
            "a claim that never completed must be marked uncertain"
        );

        // The whole point: it must not be silently re-injected, because the
        // terminal may already hold it.
        let rebound = restored.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        assert!(
            restored.delivery_candidate(&rebound).is_none(),
            "an uncertain message must not be automatically retried"
        );

        // And a second claim must be refused rather than duplicating delivery.
        // Two independent refusals guard the retry, and both matter:
        // the uncertain flag blocks a re-claim for the original incarnation,
        assert_eq!(
            restored.claim_delivery(&message.message_id, &recipient.instance_id, "term_2", 8),
            Err(CollaborationError::DeliveryStarted)
        );
        // and the agent that comes back after a cold restart is a new
        // incarnation, which was never this message's recipient.
        assert_ne!(rebound.instance_id, recipient.instance_id);
        assert_eq!(
            restored.claim_delivery(&message.message_id, &rebound.instance_id, "term_2", 8),
            Err(CollaborationError::NotRecipient)
        );
    }

    #[test]
    fn a_completed_delivery_is_not_marked_uncertain_by_a_later_restart() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        state
            .connect(sender.clone(), recipient.clone(), 1)
            .expect("lobby");
        let message = state
            .send(sender, recipient.clone(), "done deal".into(), None, None, 2)
            .expect("message");
        state
            .claim_delivery(&message.message_id, &recipient.instance_id, "term_2", 7)
            .expect("claim");
        state
            .mark_injected(&message.message_id, &recipient.instance_id, 3, 12)
            .expect("inject");

        state.prepare_for_cold_restore();
        let recovered = state.message(&message.message_id).expect("message");
        assert!(
            !recovered.delivery_uncertain,
            "a delivery that completed must not become uncertain"
        );
        assert_eq!(recovered.state, AgentMessageState::Injected);
    }

    #[test]
    fn a_capped_listing_reports_that_more_remain_and_where_to_resume() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        for index in 0..5 {
            state
                .send(
                    sender.clone(),
                    recipient.clone(),
                    format!("message {index}"),
                    None,
                    None,
                    1,
                )
                .expect("send");
        }

        let page = state.list(&recipient, AgentMessageBox::Inbox, 0, Some(2), false, 10);
        assert_eq!(page.messages.len(), 2);
        assert!(page.has_more, "a capped page must say more remain");
        let cursor = page.next_after_sequence.expect("cursor");

        // Resuming from the cursor continues without repeating or skipping.
        let second = state.list(
            &recipient,
            AgentMessageBox::Inbox,
            cursor,
            Some(2),
            false,
            10,
        );
        assert_eq!(second.messages.len(), 2);
        assert!(second.has_more);
        assert_eq!(second.messages[0].body.as_deref(), Some("message 2"));

        let last = state.list(
            &recipient,
            AgentMessageBox::Inbox,
            second.next_after_sequence.expect("cursor"),
            Some(2),
            false,
            10,
        );
        assert_eq!(last.messages.len(), 1);
        assert!(!last.has_more, "the final page must not claim more remain");
    }

    #[test]
    fn paging_does_not_observe_messages_beyond_the_page() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        for index in 0..3 {
            state
                .send(
                    sender.clone(),
                    recipient.clone(),
                    format!("message {index}"),
                    None,
                    None,
                    1,
                )
                .expect("send");
        }

        let page = state.list(&recipient, AgentMessageBox::Inbox, 0, Some(1), false, 10);
        assert_eq!(page.messages.len(), 1);
        assert!(page.has_more);

        // Observation is a recipient action; peeking past the page must not
        // mark the next message read, which would suppress its auto-delivery.
        let rest = state.list(&recipient, AgentMessageBox::All, 0, Some(10), false, 10);
        let unread = rest
            .messages
            .iter()
            .filter(|m| m.state == AgentMessageState::Pending)
            .count();
        assert_eq!(unread, 0, "listing all marks them observed");
        assert_eq!(rest.messages.len(), 3);
    }

    #[test]
    fn message_limits_and_party_permissions_are_enforced() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        assert_eq!(
            state.send(
                sender.clone(),
                recipient.clone(),
                "x".repeat(MAX_MESSAGE_BODY_BYTES + 1),
                None,
                None,
                10,
            ),
            Err(CollaborationError::BodyTooLarge)
        );

        let message = state
            .send(
                sender.clone(),
                recipient.clone(),
                "review".into(),
                None,
                None,
                20,
            )
            .unwrap();
        assert_eq!(
            state.acknowledge(&sender, &message.message_id, 30),
            Err(CollaborationError::NotRecipient)
        );
        assert_eq!(
            state.revoke(&recipient, &message.message_id, 30),
            Err(CollaborationError::NotSender)
        );
    }

    #[test]
    fn peer_quota_is_ordered_and_released_by_terminal_message_states() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        let other = state.ensure_agent(&agent("term_3", "w1:p3", "tester", "pi"));

        let mut first = None;
        for index in 0..MAX_PENDING_MESSAGES_PER_PEER {
            let message = state
                .send(
                    sender.clone(),
                    recipient.clone(),
                    format!("message {index}"),
                    None,
                    (index == 0).then(|| "retry-safe".to_string()),
                    index as u64,
                )
                .expect("messages up to the peer quota should be retained");
            first.get_or_insert(message);
        }

        let first = first.expect("the quota is nonzero");
        let retry = state
            .send(
                sender.clone(),
                recipient.clone(),
                "uncertain retry".into(),
                None,
                Some("retry-safe".into()),
                100,
            )
            .expect("an idempotent retry must bypass the quota");
        assert_eq!(retry.message_id, first.message_id);
        assert_eq!(
            state.send(
                sender.clone(),
                recipient.clone(),
                "one too many".into(),
                None,
                None,
                101,
            ),
            Err(CollaborationError::PeerMailboxFull)
        );

        state
            .send(
                recipient.clone(),
                sender.clone(),
                "the reverse direction has its own quota".into(),
                None,
                None,
                102,
            )
            .expect("the quota is ordered by sender and recipient");
        state
            .send(
                sender.clone(),
                other,
                "an unrelated recipient remains reachable".into(),
                None,
                None,
                103,
            )
            .expect("one noisy peer must not block unrelated mail");

        state
            .acknowledge(&recipient, &first.message_id, 104)
            .expect("the recipient can release quota capacity");
        state
            .send(
                sender,
                recipient,
                "capacity restored".into(),
                None,
                None,
                105,
            )
            .expect("terminal message states must not consume peer quota");
    }

    #[test]
    fn reply_chains_stop_at_the_bounded_depth() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        let mut latest = state
            .send(
                sender.clone(),
                recipient.clone(),
                "root".into(),
                None,
                None,
                1,
            )
            .expect("the root message should be accepted");

        for depth in 1..MAX_REPLY_CHAIN_DEPTH {
            let (reply_sender, reply_recipient) = if depth % 2 == 0 {
                (sender.clone(), recipient.clone())
            } else {
                (recipient.clone(), sender.clone())
            };
            latest = state
                .send(
                    reply_sender,
                    reply_recipient,
                    format!("reply {depth}"),
                    Some(latest.message_id),
                    None,
                    depth as u64 + 1,
                )
                .expect("replies through the maximum depth should be accepted");
        }

        let (reply_sender, reply_recipient) = if MAX_REPLY_CHAIN_DEPTH.is_multiple_of(2) {
            (sender, recipient)
        } else {
            (recipient, sender)
        };
        assert_eq!(
            state.send(
                reply_sender,
                reply_recipient,
                "reply too deep".into(),
                Some(latest.message_id),
                None,
                100,
            ),
            Err(CollaborationError::ReplyChainTooDeep)
        );
        assert_eq!(state.messages.len(), MAX_REPLY_CHAIN_DEPTH);
    }

    #[test]
    fn live_handoff_serialization_preserves_mail_and_identity_counters() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        let sent = state
            .send(
                sender.clone(),
                recipient.clone(),
                "review".into(),
                None,
                None,
                10,
            )
            .unwrap();
        state.connect(sender, recipient.clone(), 11).unwrap();
        state
            .mark_injected(&sent.message_id, &recipient.instance_id, 3, 12)
            .unwrap();

        let encoded = serde_json::to_string(&state).unwrap();
        let mut restored: CollaborationState = serde_json::from_str(&encoded).unwrap();
        let inbox = restored
            .list(&recipient, AgentMessageBox::Inbox, 0, None, false, 20)
            .messages;
        assert_eq!(inbox[0].message_id, sent.message_id);
        assert_eq!(inbox[0].state, AgentMessageState::Injected);
        assert_eq!(restored.lobbies().len(), 1);
        assert!(restored.delivery_candidate(&recipient).is_none());
        assert_eq!(restored.latest_sequence(), 1);
        let next = restored.ensure_agent(&agent("term_3", "w1:p3", "tester", "pi"));
        assert_eq!(next.instance_id, "agent_3");
    }

    #[test]
    fn direct_lobby_connect_is_idempotent_and_replacement_does_not_inherit_it() {
        let mut state = CollaborationState::default();
        let first = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let second = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        let lobby = state.connect(first.clone(), second.clone(), 10).unwrap();
        let reversed = state.connect(second, first, 20).unwrap();
        assert_eq!(lobby.lobby_id, reversed.lobby_id);
        assert_eq!(state.lobbies().len(), 1);

        state.retire_terminal("term_2");
        assert!(!state.lobbies()[0].members[1].active);
        let replacement = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        assert!(!state.lobbies()[0]
            .members
            .iter()
            .any(|member| member.party.instance_id == replacement.instance_id));
    }

    #[test]
    fn lobby_lifecycle_enforces_owner_and_preserves_mail() {
        let mut state = CollaborationState::default();
        let owner = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let member = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        let third = state.ensure_agent(&agent("term_3", "w1:p3", "tester", "pi"));
        let lobby = state
            .connect_many(
                vec![owner.clone(), member.clone(), third.clone()],
                Some("review".into()),
                10,
            )
            .unwrap();
        let message = state
            .send(
                member.clone(),
                owner.clone(),
                "keep this history".into(),
                None,
                None,
                11,
            )
            .unwrap();

        assert_eq!(
            state.rename_lobby(&member, &lobby.lobby_id, "renamed".into()),
            Err(CollaborationError::LobbyNotOwner)
        );
        let renamed = state
            .rename_lobby(&owner, &lobby.lobby_id, "  design review  ".into())
            .unwrap();
        assert_eq!(renamed.label, "design review");
        assert_eq!(renamed.revision, 2);

        let LobbyMutation::Updated(after_remove) = state
            .remove_lobby_member(&owner, &lobby.lobby_id, &third.instance_id)
            .unwrap()
        else {
            panic!("removing one of several members must keep the lobby");
        };
        assert_eq!(after_remove.members.len(), 2);
        assert!(state.message(&message.message_id).is_some());

        let LobbyMutation::Updated(after_owner_leave) =
            state.leave_lobby(&owner, &lobby.lobby_id).unwrap()
        else {
            panic!("one remaining member must keep the lobby");
        };
        assert_eq!(
            after_owner_leave.owner_instance_id,
            Some(member.instance_id.clone())
        );
        assert!(state.message(&message.message_id).is_some());

        assert!(matches!(
            state.delete_lobby(&member, &lobby.lobby_id),
            Ok(LobbyMutation::Deleted { .. })
        ));
        assert!(state.message(&message.message_id).is_some());
    }

    #[test]
    fn workspace_sync_reuses_one_space_and_adds_agents() {
        let mut state = CollaborationState::default();
        let first = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let second = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        let first_space = state
            .sync_workspace_agents(
                "w1".into(),
                "project".into(),
                vec![first.clone(), second.clone()],
                10,
            )
            .unwrap();
        let third = state.ensure_agent(&agent("term_3", "w1:p3", "tester", "pi"));
        let updated = state
            .sync_workspace_agents(
                "w1".into(),
                "project agents".into(),
                vec![first, second, third],
                20,
            )
            .unwrap();

        assert_eq!(updated.lobby_id, first_space.lobby_id);
        assert_eq!(updated.workspace_id.as_deref(), Some("w1"));
        assert_eq!(updated.members.len(), 3);
        assert_eq!(state.lobbies().len(), 1);
        assert!(updated.revision > first_space.revision);
    }

    #[test]
    fn automatic_delivery_requires_a_lobby_and_one_idle_cycle_per_message() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        let first = state
            .send(
                sender.clone(),
                recipient.clone(),
                "first".into(),
                None,
                None,
                10,
            )
            .unwrap();
        let second = state
            .send(
                sender.clone(),
                recipient.clone(),
                "second".into(),
                None,
                None,
                11,
            )
            .unwrap();
        assert!(state.delivery_candidate(&recipient).is_none());

        state
            .connect(sender.clone(), recipient.clone(), 12)
            .unwrap();
        assert_eq!(
            state.delivery_candidate(&recipient).unwrap().message_id,
            first.message_id
        );
        state
            .mark_injected(&first.message_id, &recipient.instance_id, 5, 20)
            .unwrap();
        assert!(state.delivery_candidate(&recipient).is_none());
        assert_eq!(
            state.revoke(&sender, &first.message_id, 21),
            Err(CollaborationError::DeliveryStarted)
        );

        state.note_recipient_state(&recipient.instance_id, AgentStatus::Working, 6);
        assert!(state.delivery_candidate(&recipient).is_none());
        state.note_recipient_state(&recipient.instance_id, AgentStatus::Idle, 7);
        assert_eq!(
            state.delivery_candidate(&recipient).unwrap().message_id,
            second.message_id
        );
    }

    #[test]
    fn manual_observation_suppresses_automatic_injection() {
        let mut state = CollaborationState::default();
        let sender = state.ensure_agent(&agent("term_1", "w1:p1", "author", "codex"));
        let recipient = state.ensure_agent(&agent("term_2", "w1:p2", "reviewer", "claude"));
        state.connect(sender.clone(), recipient.clone(), 1).unwrap();
        state
            .send(
                sender,
                recipient.clone(),
                "read manually".into(),
                None,
                None,
                2,
            )
            .unwrap();
        let messages = state
            .list(&recipient, AgentMessageBox::Inbox, 0, None, false, 3)
            .messages;
        assert_eq!(messages[0].state, AgentMessageState::Observed);
        assert!(state.delivery_candidate(&recipient).is_none());
    }
}
