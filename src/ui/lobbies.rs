use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::widgets::{
    action_button_row_rects, centered_popup_rect, follow_selection_scroll, modal_stack_areas,
    panel_contrast_fg, render_action_button, render_modal_header, render_panel_shell,
    ActionButtonSpec,
};
use crate::api::schema::AgentMessageState;
use crate::app::state::{LobbyThreadEntry, Palette};
use crate::app::AppState;

const POPUP_WIDTH: u16 = 96;
const POPUP_HEIGHT: u16 = 30;

pub(crate) fn lobby_browser_rect(area: Rect) -> Option<Rect> {
    centered_popup_rect(area, POPUP_WIDTH, POPUP_HEIGHT)
}

/// Geometry for the lobby browser, computed once and shared by render and
/// mouse so a click or wheel event always lands on the pane it appears over.
pub(crate) struct LobbyBrowserLayout {
    pub popup: Rect,
    pub header: Rect,
    /// Selectable lobby rows, excluding the column title.
    pub lobby_list: Rect,
    pub members: Rect,
    pub thread: Rect,
    pub footer: Rect,
    pub close: Rect,
}

pub(crate) fn lobby_browser_layout(area: Rect) -> Option<LobbyBrowserLayout> {
    let popup = lobby_browser_rect(area)?;
    let inner = Rect::new(
        popup.x.saturating_add(1),
        popup.y.saturating_add(1),
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );
    if inner.height < 8 {
        return None;
    }
    let areas = modal_stack_areas(inner, 1, 1, 1, 1);
    let body = areas.content;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(45), Constraint::Min(1)])
        .split(body);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(rows[0]);

    // Both column blocks carry a title, and a titled block reserves its top row
    // (see `column_titles_reserve_their_own_row`), so selectable rows start one
    // below the column rect.
    let lobby_list = Block::default()
        .borders(Borders::RIGHT)
        .title(LOBBIES_TITLE)
        .inner(columns[0]);
    let close = action_button_row_rects(
        areas.actions.unwrap_or(inner),
        &[ActionButtonSpec {
            hint: Some("esc"),
            label: "close",
        }],
        2,
        0,
    )
    .first()
    .copied()
    .unwrap_or(inner);

    Some(LobbyBrowserLayout {
        popup,
        header: areas.header,
        lobby_list,
        members: columns[1],
        thread: rows[1],
        footer: areas.footer.unwrap_or(inner),
        close,
    })
}

/// Index of the lobby row under the pointer, accounting for scroll.
pub(crate) fn lobby_row_at(
    layout: &LobbyBrowserLayout,
    scroll: usize,
    count: usize,
    col: u16,
    row: u16,
) -> Option<usize> {
    let list = layout.lobby_list;
    if col < list.x || col >= list.x.saturating_add(list.width) {
        return None;
    }
    if row < list.y || row >= list.y.saturating_add(list.height) {
        return None;
    }
    let index = scroll + (row - list.y) as usize;
    (index < count).then_some(index)
}

/// First visible lobby row, so the highlighted lobby cannot scroll off.
pub(crate) fn lobby_list_scroll(
    layout: &LobbyBrowserLayout,
    selected: usize,
    scroll: usize,
) -> usize {
    follow_selection_scroll(selected, scroll, layout.lobby_list.height as usize)
}

const LOBBIES_TITLE: &str = " lobbies ";

/// `area` must match the rect the mouse layer measures against
/// (`sidebar_rect ∪ terminal_area`), so clicks and wheel events land on the
/// pane they appear to be over.
pub(super) fn render_lobby_browser(app: &AppState, frame: &mut Frame, area: Rect) {
    let full = frame.area();
    super::dim_background(frame, full);
    let Some(layout) = lobby_browser_layout(area) else {
        return;
    };
    if render_panel_shell(
        frame,
        layout.popup,
        app.palette.accent,
        app.palette.panel_bg,
    )
    .is_none()
    {
        return;
    }

    render_modal_header(frame, layout.header, "agent lobbies", &app.palette);
    render_lobby_list(app, frame, &layout);
    render_members(app, frame, layout.members);
    render_thread(app, frame, layout.thread);
    if let Some(notice) = app.lobby_browser.notice.as_deref() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                super::text::truncate_end(&format!(" {notice}"), layout.footer.width as usize),
                Style::default().fg(app.palette.yellow),
            ))),
            layout.footer,
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ↑↓", Style::default().fg(app.palette.accent)),
                Span::styled(" lobby   ", Style::default().fg(app.palette.overlay0)),
                Span::styled("u/d", Style::default().fg(app.palette.accent)),
                Span::styled(" scroll   ", Style::default().fg(app.palette.overlay0)),
                Span::styled("J/K", Style::default().fg(app.palette.accent)),
                Span::styled(" pick turn   ", Style::default().fg(app.palette.overlay0)),
                Span::styled("f/F", Style::default().fg(app.palette.accent)),
                Span::styled(
                    " forward turn/all   ",
                    Style::default().fg(app.palette.overlay0),
                ),
                Span::styled("x/D", Style::default().fg(app.palette.accent)),
                Span::styled(" leave / delete", Style::default().fg(app.palette.overlay0)),
            ])),
            layout.footer,
        );
    }
    render_action_button(
        frame,
        layout.close,
        Some("esc"),
        "close",
        Style::default()
            .fg(panel_contrast_fg(&app.palette))
            .bg(app.palette.accent)
            .add_modifier(Modifier::BOLD),
    );
}

fn render_lobby_list(app: &AppState, frame: &mut Frame, layout: &LobbyBrowserLayout) {
    let column = Rect::new(
        layout.lobby_list.x,
        layout.lobby_list.y.saturating_sub(1),
        layout.lobby_list.width.saturating_add(1),
        layout.lobby_list.height.saturating_add(1),
    );
    frame.render_widget(
        Block::default()
            .borders(Borders::RIGHT)
            .title(LOBBIES_TITLE),
        column,
    );
    if app.lobby_browser.lobbies.is_empty() {
        frame.render_widget(
            Paragraph::new(
                " no lobbies yet\n\n Right-click an agent pane and choose\n Call another agent.",
            ),
            layout.lobby_list,
        );
        return;
    }
    let scroll = lobby_list_scroll(layout, app.lobby_browser.selected, app.lobby_browser.scroll);
    let lines = app
        .lobby_browser
        .lobbies
        .iter()
        .enumerate()
        .skip(scroll)
        .take(layout.lobby_list.height as usize)
        .map(|(index, lobby)| {
            let selected = index == app.lobby_browser.selected;
            let style = if selected {
                Style::default()
                    .fg(panel_contrast_fg(&app.palette))
                    .bg(app.palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.palette.text)
            };
            let active = lobby.members.iter().filter(|m| m.active).count();
            // A space is the workspace's standing room; a pair lobby is ad hoc.
            let kind = if lobby.workspace_id.is_some() {
                "space"
            } else {
                "direct"
            };
            Line::styled(
                format!(
                    " {}  ({kind}, {active}/{} live) ",
                    lobby.label,
                    lobby.members.len()
                ),
                style,
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), layout.lobby_list);
}

fn render_members(app: &AppState, frame: &mut Frame, area: Rect) {
    let title = match app
        .lobby_browser
        .lobbies
        .get(app.lobby_browser.selected)
        .and_then(|lobby| lobby.workspace_id.as_deref())
    {
        Some(workspace) => format!(" members of space {workspace} "),
        None => " members ".to_string(),
    };
    let block = Block::default().title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(lobby) = app.lobby_browser.lobbies.get(app.lobby_browser.selected) else {
        return;
    };
    let lines = lobby
        .members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let label = member
                .party
                .name
                .as_deref()
                .or(member.party.agent.as_deref())
                .unwrap_or(&member.party.instance_id);
            // An unnamed agent already displays as its kind; printing the kind
            // again would read "claude claude".
            let kind = member.party.agent.as_deref().filter(|kind| *kind != label);
            let status = if member.active { "active" } else { "offline" };
            let status_style = if member.active {
                Style::default().fg(app.palette.green)
            } else {
                Style::default().fg(app.palette.overlay0)
            };
            let at_cursor = app.lobby_browser.member_cursor == Some(index);
            let mut spans = vec![Span::styled(
                format!("{} {label}", if at_cursor { "▶" } else { " •" }),
                if at_cursor {
                    Style::default()
                        .fg(app.palette.text)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(app.palette.text)
                },
            )];
            if let Some(kind) = kind {
                spans.push(Span::styled(
                    format!("  {kind}"),
                    Style::default().fg(app.palette.overlay0),
                ));
            }
            spans.push(Span::styled(
                "  ",
                Style::default().fg(app.palette.overlay0),
            ));
            spans.push(Span::styled(status, status_style));
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// What each state means to someone watching, rather than the protocol word
/// alone: `injected` in particular does not mean the work was done.
fn state_label(state: AgentMessageState) -> &'static str {
    match state {
        AgentMessageState::Pending => "queued",
        AgentMessageState::Observed => "read",
        AgentMessageState::Injected => "delivered",
        AgentMessageState::Acknowledged => "acknowledged",
        AgentMessageState::Revoked => "revoked",
    }
}

fn state_color(state: AgentMessageState, palette: &Palette) -> Color {
    match state {
        AgentMessageState::Pending => palette.overlay0,
        AgentMessageState::Observed => palette.blue,
        AgentMessageState::Injected => palette.yellow,
        AgentMessageState::Acknowledged => palette.green,
        AgentMessageState::Revoked => palette.red,
    }
}

/// Two lines per turn: who sent it to whom with its state, then the body.
fn thread_lines<'a>(
    entry: &'a LobbyThreadEntry,
    palette: &Palette,
    width: usize,
    at_cursor: bool,
) -> Vec<Line<'a>> {
    let arrow = if entry.is_reply { "↩" } else { "→" };
    // The marker shows where a "this message and below" forward would begin.
    let header = format!(
        "{} {} {arrow} {} ",
        if at_cursor { "▶" } else { " " },
        entry.from,
        entry.to
    );
    // An orphaned turn is a dead end, not progress, so it must not read as
    // "delivered".
    let state = if entry.recipient_gone {
        "orphaned"
    } else {
        state_label(entry.state)
    };
    let pad = width
        .saturating_sub(super::text::display_width(&header))
        .saturating_sub(state.chars().count() + 1);
    vec![
        Line::from(vec![
            Span::styled(
                header,
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled(
                state,
                Style::default().fg(if entry.recipient_gone {
                    palette.red
                } else {
                    state_color(entry.state, palette)
                }),
            ),
        ]),
        Line::from(Span::styled(
            format!(
                "   {}",
                super::text::truncate_end(&entry.body.replace('\n', " "), width.saturating_sub(4))
            ),
            Style::default().fg(palette.subtext0),
        )),
    ]
}

fn render_thread(app: &AppState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::TOP)
        .title(" conversation ")
        .border_style(Style::default().fg(app.palette.surface_dim));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.lobby_browser.lobbies.is_empty() {
        return;
    }
    if app.lobby_browser.thread.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " nothing sent in this lobby yet",
                Style::default().fg(app.palette.overlay0),
            ))),
            inner,
        );
        return;
    }

    let width = inner.width as usize;
    let cursor = app.lobby_browser.thread_cursor;
    let mut lines = app
        .lobby_browser
        .thread
        .iter()
        .enumerate()
        .flat_map(|(index, entry)| thread_lines(entry, &app.palette, width, cursor == Some(index)))
        .collect::<Vec<_>>();
    // `thread_scroll` counts turns back from the newest, so an untouched
    // browser always shows the latest exchange.
    let visible = inner.height as usize;
    let drop_from_end = app.lobby_browser.thread_scroll.saturating_mul(2);
    lines.truncate(lines.len().saturating_sub(drop_from_end));
    let first = lines.len().saturating_sub(visible);
    let lines = lines.split_off(first);
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::LobbyThreadEntry;
    use ratatui::{backend::TestBackend, Terminal};

    fn rendered_text(app: &AppState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 32)).expect("test terminal");
        terminal
            .draw(|frame| render_lobby_browser(app, frame, frame.area()))
            .expect("render");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn browser_renders_lobbies_and_member_activity() {
        let mut app = AppState::test_new();
        app.lobby_browser.lobbies = vec![crate::api::schema::AgentLobby {
            lobby_id: "lobby_1".into(),
            label: "review agents".into(),
            owner_instance_id: Some("agent_1".into()),
            workspace_id: None,
            members: vec![
                crate::api::schema::AgentLobbyMember {
                    party: crate::api::schema::AgentMessageParty {
                        instance_id: "agent_1".into(),
                        terminal_id: "term_1".into(),
                        name: Some("author".into()),
                        agent: Some("codex".into()),
                        pane_id: "w1:p1".into(),
                    },
                    active: true,
                },
                crate::api::schema::AgentLobbyMember {
                    party: crate::api::schema::AgentMessageParty {
                        instance_id: "agent_2".into(),
                        terminal_id: "term_2".into(),
                        name: Some("reviewer".into()),
                        agent: Some("claude".into()),
                        pane_id: "w1:p2".into(),
                    },
                    active: false,
                },
            ],
            created_at_unix_ms: 1,
            revision: 2,
        }];
        let text = rendered_text(&app);
        assert!(text.contains("agent lobbies"));
        assert!(text.contains("review agents"));
        assert!(text.contains("author"));
        assert!(text.contains("active"));
        assert!(text.contains("reviewer"));
        assert!(text.contains("offline"));
    }

    #[test]
    fn browser_empty_state_explains_one_click_connection() {
        let text = rendered_text(&AppState::test_new());
        assert!(text.contains("no lobbies yet"));
        assert!(text.contains("Call another agent"));
    }

    fn entry(from: &str, to: &str, state: AgentMessageState, body: &str) -> LobbyThreadEntry {
        LobbyThreadEntry {
            message_id: format!("msg_{from}_{to}"),
            from: from.into(),
            to: to.into(),
            state,
            body: body.into(),
            is_reply: false,
            recipient_gone: false,
        }
    }

    fn app_with_thread(thread: Vec<LobbyThreadEntry>) -> AppState {
        let mut app = AppState::test_new();
        app.lobby_browser.lobbies = vec![crate::api::schema::AgentLobby {
            lobby_id: "lobby_1".into(),
            label: "review agents".into(),
            owner_instance_id: Some("agent_1".into()),
            workspace_id: None,
            members: vec![crate::api::schema::AgentLobbyMember {
                party: crate::api::schema::AgentMessageParty {
                    instance_id: "agent_1".into(),
                    terminal_id: "term_1".into(),
                    name: Some("author".into()),
                    agent: Some("codex".into()),
                    pane_id: "w1:p1".into(),
                },
                active: true,
            }],
            created_at_unix_ms: 1,
            revision: 2,
        }];
        app.lobby_browser.thread = thread;
        app
    }

    #[test]
    fn conversation_shows_each_turn_with_a_plain_language_state() {
        let text = rendered_text(&app_with_thread(vec![
            entry(
                "author",
                "reviewer",
                AgentMessageState::Acknowledged,
                "Review the parser boundary",
            ),
            entry(
                "author",
                "builder",
                AgentMessageState::Pending,
                "Rebuild once review lands",
            ),
        ]));
        assert!(text.contains("conversation"));
        assert!(text.contains("author → reviewer"));
        assert!(text.contains("Review the parser boundary"));
        assert!(text.contains("acknowledged"));
        assert!(text.contains("author → builder"));
        // `injected` would read as "done" to an operator; it is not.
        assert!(text.contains("queued"));
    }

    #[test]
    fn conversation_marks_replies_and_revoked_bodies() {
        let mut reply = entry(
            "reviewer",
            "author",
            AgentMessageState::Injected,
            "gates still hold",
        );
        reply.is_reply = true;
        let text = rendered_text(&app_with_thread(vec![
            entry(
                "author",
                "reviewer",
                AgentMessageState::Revoked,
                "(revoked by sender)",
            ),
            reply,
        ]));
        assert!(text.contains("reviewer ↩ author"));
        assert!(text.contains("(revoked by sender)"));
        assert!(text.contains("revoked"));
        assert!(text.contains("delivered"));
    }

    fn app_with_many_lobbies(count: usize) -> AppState {
        let mut app = AppState::test_new();
        app.lobby_browser.lobbies = (0..count)
            .map(|index| crate::api::schema::AgentLobby {
                lobby_id: format!("lobby_{index}"),
                label: format!("lobby number {index}"),
                owner_instance_id: None,
                workspace_id: None,
                members: Vec::new(),
                created_at_unix_ms: 1,
                revision: 1,
            })
            .collect();
        app
    }

    #[test]
    fn lobby_list_scrolls_so_the_selection_cannot_leave_the_viewport() {
        let area = Rect::new(0, 0, 100, 32);
        let layout = lobby_browser_layout(area).expect("layout");
        let visible = layout.lobby_list.height as usize;
        let count = visible + 8;
        let mut app = app_with_many_lobbies(count);

        // Selecting the last lobby must scroll it into view, not render from 0.
        app.lobby_browser.selected = count - 1;
        app.lobby_browser.scroll = lobby_list_scroll(
            &layout,
            app.lobby_browser.selected,
            app.lobby_browser.scroll,
        );
        let text = rendered_text(&app);
        assert!(text.contains(&format!("lobby number {}", count - 1)));
        assert!(!text.contains("lobby number 0 "));

        // And scrolling back keeps the earlier rows reachable.
        app.lobby_browser.selected = 0;
        app.lobby_browser.scroll = lobby_list_scroll(
            &layout,
            app.lobby_browser.selected,
            app.lobby_browser.scroll,
        );
        let text = rendered_text(&app);
        assert!(text.contains("lobby number 0"));
    }

    #[test]
    fn lobby_row_hit_test_follows_the_same_scroll_as_the_render() {
        let area = Rect::new(0, 0, 100, 32);
        let layout = lobby_browser_layout(area).expect("layout");
        let visible = layout.lobby_list.height as usize;
        let count = visible + 8;
        let scroll = lobby_list_scroll(&layout, count - 1, 0);

        // The top visible row maps to the first scrolled-to lobby, not index 0.
        assert_eq!(
            lobby_row_at(
                &layout,
                scroll,
                count,
                layout.lobby_list.x + 1,
                layout.lobby_list.y
            ),
            Some(scroll)
        );
        // The column title row is above the selectable rows.
        assert_eq!(
            lobby_row_at(
                &layout,
                scroll,
                count,
                layout.lobby_list.x + 1,
                layout.lobby_list.y - 1
            ),
            None
        );
    }

    #[test]
    fn member_row_does_not_repeat_the_kind_for_an_unnamed_agent() {
        let mut app = AppState::test_new();
        app.lobby_browser.lobbies = vec![crate::api::schema::AgentLobby {
            lobby_id: "lobby_1".into(),
            label: "review agents".into(),
            owner_instance_id: Some("agent_1".into()),
            workspace_id: None,
            members: vec![crate::api::schema::AgentLobbyMember {
                party: crate::api::schema::AgentMessageParty {
                    instance_id: "agent_1".into(),
                    terminal_id: "term_1".into(),
                    name: None,
                    agent: Some("claude".into()),
                    pane_id: "w1:p1".into(),
                },
                active: true,
            }],
            created_at_unix_ms: 1,
            revision: 1,
        }];
        let text = rendered_text(&app);
        assert!(text.contains("claude"));
        assert!(!text.contains("claude  claude"));
    }

    #[test]
    fn browser_uses_the_shared_modal_shell_with_a_close_affordance() {
        let text = rendered_text(&AppState::test_new());
        assert!(text.contains("agent lobbies"));
        assert!(text.contains("esc close"));
    }

    #[test]
    fn column_titles_reserve_their_own_row() {
        let area = Rect::new(0, 0, 20, 5);
        let right = Block::default().borders(Borders::RIGHT).title(" lobbies ");
        let none = Block::default().title(" members ");
        assert_eq!(
            right.inner(area).y,
            1,
            "titled block must reserve a title row"
        );
        assert_eq!(
            none.inner(area).y,
            1,
            "titled block must reserve a title row"
        );
    }

    #[test]
    fn an_orphaned_turn_reads_as_orphaned_not_delivered() {
        let mut orphan = entry(
            "author",
            "reviewer",
            AgentMessageState::Injected,
            "review the parser boundary",
        );
        // The recipient incarnation is gone, so this can never be acked.
        orphan.recipient_gone = true;
        let text = rendered_text(&app_with_thread(vec![orphan]));
        assert!(text.contains("orphaned"));
        // "delivered" would imply the exchange is still progressing.
        assert!(!text.contains("delivered"));
    }

    #[test]
    fn conversation_explains_an_empty_lobby() {
        let text = rendered_text(&app_with_thread(Vec::new()));
        assert!(text.contains("nothing sent in this lobby yet"));
    }

    #[test]
    fn conversation_shows_the_newest_turn_and_scrolls_back_from_it() {
        let thread = (0..20)
            .map(|index| {
                entry(
                    "author",
                    "reviewer",
                    AgentMessageState::Acknowledged,
                    &format!("turn number {index}"),
                )
            })
            .collect::<Vec<_>>();
        let mut app = app_with_thread(thread);
        let text = rendered_text(&app);
        assert!(text.contains("turn number 19"));
        assert!(!text.contains("turn number 0 "));

        app.lobby_browser.thread_scroll = 10;
        let text = rendered_text(&app);
        assert!(!text.contains("turn number 19"));
        assert!(text.contains("turn number 9"));
    }

    #[test]
    fn thread_rect_sits_inside_the_popup_below_the_member_panes() {
        let area = Rect::new(0, 0, 120, 40);
        let popup = lobby_browser_rect(area).expect("popup");
        let thread = lobby_browser_layout(area).expect("layout").thread;
        assert!(thread.y > popup.y);
        assert!(thread.y + thread.height <= popup.y + popup.height);
        assert!(thread.x >= popup.x);
    }
}
