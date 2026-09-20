//! Company room browser.
//!
//! Shows one room at a time: its conversation, its seats, or its knowledge.
//! Rooms live on the server; this renders a snapshot taken outside render, so
//! drawing never inspects company state or allocates per room.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::text::{display_width_u16, truncate_end};
use super::widgets::{
    action_button_row_rects, centered_popup_rect, follow_selection_scroll, modal_stack_areas,
    panel_contrast_fg, render_action_button, render_modal_header, render_panel_shell,
    ActionButtonSpec,
};
use crate::app::state::RoomTab;
use crate::app::AppState;

/// Smallest size the browser asks for; on a larger screen it grows to fill
/// most of it, because an expanded report needs the room to be read in place.
const POPUP_MIN_WIDTH: u16 = 100;
const POPUP_MIN_HEIGHT: u16 = 32;
/// Share of the screen the browser takes when the screen is larger than the
/// minimum, as a percentage.
const POPUP_SCREEN_PERCENT: u16 = 94;
const ROOM_LIST_PERCENT: u16 = 30;

/// Clickable action in the browser. Keys and mouse resolve to the same set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoomAction {
    Post,
    Send,
    Cancel,
    Close,
    /// Opens the room creator for the selected room's workspace.
    NewRoom,
    /// Opens the seat form for the selected room.
    AddSeat,
    /// Creates the room, or adds the seat, the form describes.
    Submit,
    /// Pauses an active room, or resumes a paused one.
    ToggleLifecycle,
}

/// Per-seat action offered on a member row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoomMemberAction {
    Bind,
    Unbind,
    Remove,
}

/// Which overlay the browser is showing. One value drives both the button row
/// and what the body renders, so a click never lands on a stale button set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoomView {
    Browse,
    Compose,
    Form,
    Bind,
}

/// Everything the button row and body geometry depend on, resolved once
/// outside render so the mouse layer measures the same chrome that was drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoomChrome {
    pub view: RoomView,
    pub members_tab: bool,
    pub has_room: bool,
    pub paused: bool,
    pub creating_room: bool,
}

pub(crate) fn room_chrome(app: &AppState) -> RoomChrome {
    let browser = &app.room_browser;
    let view = if browser.bind.is_some() {
        RoomView::Bind
    } else if browser.form.is_some() {
        RoomView::Form
    } else if browser.composer.is_some() {
        RoomView::Compose
    } else {
        RoomView::Browse
    };
    RoomChrome {
        view,
        members_tab: browser.tab == RoomTab::Members,
        has_room: browser.selected_room().is_some(),
        paused: browser
            .selected_room()
            .is_some_and(|room| room.lifecycle != "active"),
        creating_room: browser
            .form
            .as_ref()
            .is_some_and(|form| form.creating_room()),
    }
}

/// Rects of the fields a form offers, so a click focuses the field under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoomFormRects {
    pub primary: Rect,
    pub secondary: Rect,
    /// Absent while creating a room, which has no orchestrator box.
    pub orchestrator: Option<Rect>,
}

/// Geometry, computed once and shared by render and mouse so a click lands on
/// what it appears to land on.
pub(crate) struct RoomLayout {
    pub popup: Rect,
    pub header: Rect,
    pub tabs: Rect,
    /// Selectable room rows, excluding the column title.
    pub room_list: Rect,
    pub content: Rect,
    pub footer: Rect,
    pub buttons: Vec<(Rect, RoomAction)>,
    /// Present only while a form is open.
    pub form: Option<RoomFormRects>,
}

fn button_specs(chrome: RoomChrome) -> Vec<(Option<&'static str>, &'static str, RoomAction, bool)> {
    match chrome.view {
        RoomView::Compose => vec![
            (Some("↵"), "send", RoomAction::Send, true),
            (Some("esc"), "cancel", RoomAction::Cancel, false),
        ],
        RoomView::Form => vec![
            (
                Some("↵"),
                if chrome.creating_room {
                    "create room"
                } else {
                    "add seat"
                },
                RoomAction::Submit,
                true,
            ),
            (Some("esc"), "cancel", RoomAction::Cancel, false),
        ],
        RoomView::Bind => vec![(Some("esc"), "cancel", RoomAction::Cancel, false)],
        RoomView::Browse => {
            let mut specs = Vec::new();
            if chrome.has_room {
                specs.push((Some("p"), "post", RoomAction::Post, true));
            }
            specs.push((Some("n"), "new room", RoomAction::NewRoom, !chrome.has_room));
            if chrome.has_room && chrome.members_tab {
                specs.push((Some("s"), "add seat", RoomAction::AddSeat, false));
            }
            if chrome.has_room {
                specs.push((
                    None,
                    if chrome.paused { "resume" } else { "pause" },
                    RoomAction::ToggleLifecycle,
                    false,
                ));
            }
            specs.push((Some("esc"), "close", RoomAction::Close, false));
            specs
        }
    }
}

pub(crate) fn room_layout(area: Rect, chrome: RoomChrome) -> Option<RoomLayout> {
    let share = |extent: u16| {
        u16::try_from(u32::from(extent) * u32::from(POPUP_SCREEN_PERCENT) / 100).unwrap_or(extent)
    };
    let popup = centered_popup_rect(
        area,
        POPUP_MIN_WIDTH.max(share(area.width)),
        POPUP_MIN_HEIGHT.max(share(area.height)),
    )?;
    let inner = Rect::new(
        popup.x.saturating_add(1),
        popup.y.saturating_add(1),
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );
    if inner.height < 10 {
        return None;
    }
    // header(2) gap tabs(1) gap content(min) gap footer(1) gap actions(1)
    let areas = modal_stack_areas(inner, 2, 1, 1, 1);
    let body = areas.content;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(body);
    let tabs = rows[0];
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(ROOM_LIST_PERCENT),
            Constraint::Percentage(100 - ROOM_LIST_PERCENT),
        ])
        .split(rows[2]);
    let room_list = Block::default()
        .borders(Borders::RIGHT)
        .title(ROOMS_TITLE)
        .inner(columns[0]);

    let specs = button_specs(chrome);
    let rects = action_button_row_rects(
        areas.actions.unwrap_or(inner),
        &specs
            .iter()
            .map(|(hint, label, _, _)| ActionButtonSpec { hint: *hint, label })
            .collect::<Vec<_>>(),
        2,
        0,
    );
    let buttons = rects
        .into_iter()
        .zip(specs.iter().map(|(_, _, action, _)| *action))
        .collect();

    let content = columns[1];
    let form = (chrome.view == RoomView::Form).then(|| form_rects(content, chrome.creating_room));

    Some(RoomLayout {
        popup,
        header: areas.header,
        tabs,
        room_list,
        content,
        footer: areas.footer.unwrap_or(inner),
        buttons,
        form,
    })
}

/// Column where a form's input boxes start, leaving room for the labels.
const FORM_LABEL_WIDTH: u16 = 11;
/// First body row of a form, below its one-line description.
const FORM_FIRST_ROW: u16 = 2;

fn form_rects(content: Rect, creating_room: bool) -> RoomFormRects {
    let x = content.x.saturating_add(FORM_LABEL_WIDTH);
    let width = content.width.saturating_sub(FORM_LABEL_WIDTH).max(1);
    let field = |offset: u16| Rect::new(x, content.y.saturating_add(offset), width, 1);
    RoomFormRects {
        primary: field(FORM_FIRST_ROW),
        secondary: field(FORM_FIRST_ROW + 2),
        orchestrator: (!creating_room).then(|| {
            Rect::new(
                content.x,
                content.y.saturating_add(FORM_FIRST_ROW + 4),
                content.width,
                1,
            )
        }),
    }
}

/// Rows the bind picker can show at once.
pub(crate) fn bind_list_rect(content: Rect) -> Rect {
    Rect::new(
        content.x,
        content.y.saturating_add(BIND_FIRST_ROW),
        content.width,
        content.height.saturating_sub(BIND_FIRST_ROW),
    )
}

pub(crate) fn bind_list_scroll(layout: &RoomLayout, selected: usize, scroll: usize) -> usize {
    follow_selection_scroll(
        selected,
        scroll,
        bind_list_rect(layout.content).height as usize,
    )
}

/// Bind-picker row under the pointer, accounting for scroll.
pub(crate) fn bind_row_at(
    layout: &RoomLayout,
    scroll: usize,
    count: usize,
    col: u16,
    row: u16,
) -> Option<usize> {
    let list = bind_list_rect(layout.content);
    if col < list.x || col >= list.right() || row < list.y || row >= list.bottom() {
        return None;
    }
    let index = scroll + (row - list.y) as usize;
    (index < count).then_some(index)
}

/// First body row of the bind picker, below its one-line description.
pub(crate) const BIND_FIRST_ROW: u16 = 2;

/// Member row under the pointer, in the members tab.
///
/// `scroll` is how many seats are hidden above the first drawn row, so a
/// scrolled list still acts on the seat the operator is pointing at.
pub(crate) fn member_row_at(
    layout: &RoomLayout,
    scroll: usize,
    count: usize,
    col: u16,
    row: u16,
) -> Option<usize> {
    let list = layout.content;
    if col < list.x || col >= list.right() || row < list.y || row >= list.bottom() {
        return None;
    }
    let index = scroll + (row - list.y) as usize;
    (index < count).then_some(index)
}

fn member_action_label(action: RoomMemberAction, confirming: bool) -> &'static str {
    match action {
        RoomMemberAction::Bind => " bind ",
        RoomMemberAction::Unbind => " unbind ",
        RoomMemberAction::Remove => {
            if confirming {
                " remove? "
            } else {
                " remove "
            }
        }
    }
}

/// Right-aligned per-seat buttons for one member row.
///
/// Render and hit-testing both call this, so a click on "remove" cannot land
/// on the row that merely looks like it.
pub(crate) fn member_action_rects(
    content: Rect,
    row_index: usize,
    bound: bool,
    confirming: bool,
) -> Vec<(Rect, RoomMemberAction, &'static str)> {
    let Some(y) = u16::try_from(row_index)
        .ok()
        .map(|offset| content.y.saturating_add(offset))
        .filter(|y| *y < content.bottom())
    else {
        return Vec::new();
    };
    let bind = if bound {
        RoomMemberAction::Unbind
    } else {
        RoomMemberAction::Bind
    };
    let mut rects = Vec::new();
    let mut right = content.right();
    for action in [RoomMemberAction::Remove, bind] {
        let label = member_action_label(action, confirming);
        let width = display_width_u16(label);
        let Some(x) = right.checked_sub(width) else {
            return Vec::new();
        };
        // Never let a button overlap the handle and role columns.
        if x < content.x.saturating_add(member_text_width(content)) {
            return Vec::new();
        }
        rects.push((Rect::new(x, y, width, 1), action, label));
        right = x.saturating_sub(1);
    }
    rects
}

/// Width reserved for a member's handle, role, and bound state.
const MEMBER_TEXT_WIDTH: u16 = 48;

/// Room kept free on the right of a seat row for its bind and remove buttons.
const MEMBER_BUTTONS_WIDTH: u16 = 19;
/// Widest the place column grows, so a long tab name cannot crowd the row.
const MEMBER_PLACE_MAX_WIDTH: u16 = 24;

/// Width of the place column for this pane: whatever is left between the
/// fixed columns and the buttons, up to a cap. Zero on a narrow pane, where
/// the column is dropped rather than squeezing everything else.
pub(crate) fn member_place_width(content: Rect) -> u16 {
    content
        .width
        .saturating_sub(MEMBER_TEXT_WIDTH + MEMBER_BUTTONS_WIDTH)
        .min(MEMBER_PLACE_MAX_WIDTH)
}

/// Width taken by a seat's text, which the buttons must stay clear of.
fn member_text_width(content: Rect) -> u16 {
    MEMBER_TEXT_WIDTH + member_place_width(content)
}

/// Member-row action under the pointer.
pub(crate) fn member_action_at(
    layout: &RoomLayout,
    row_index: usize,
    bound: bool,
    confirming: bool,
    col: u16,
    row: u16,
) -> Option<RoomMemberAction> {
    member_action_rects(layout.content, row_index, bound, confirming)
        .into_iter()
        .find(|(rect, _, _)| {
            col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
        })
        .map(|(_, action, _)| action)
}

/// Index of the room row under the pointer, accounting for scroll.
pub(crate) fn room_row_at(
    layout: &RoomLayout,
    scroll: usize,
    count: usize,
    col: u16,
    row: u16,
) -> Option<usize> {
    let list = layout.room_list;
    if col < list.x || col >= list.x.saturating_add(list.width) {
        return None;
    }
    if row < list.y || row >= list.y.saturating_add(list.height) {
        return None;
    }
    let index = scroll + (row - list.y) as usize;
    (index < count).then_some(index)
}

/// Tab under the pointer, when the pointer is on the tab strip.
pub(crate) fn room_tab_at(layout: &RoomLayout, col: u16, row: u16) -> Option<RoomTab> {
    if row != layout.tabs.y {
        return None;
    }
    let mut x = layout.tabs.x.saturating_add(1);
    for tab in RoomTab::ALL {
        let width = display_width_u16(tab.label()).saturating_add(3);
        if col >= x && col < x.saturating_add(width) {
            return Some(tab);
        }
        x = x.saturating_add(width);
    }
    None
}

pub(crate) fn room_list_scroll(layout: &RoomLayout, selected: usize, scroll: usize) -> usize {
    follow_selection_scroll(selected, scroll, layout.room_list.height as usize)
}

const ROOMS_TITLE: &str = " rooms ";

/// Items a room's right-click menu offers.
pub(crate) const ROOM_MENU_DELETE: &str = "Delete room";
pub(crate) const ROOM_MENU_DELETE_CONFIRM: &str = "Confirm delete";

/// Rect of the open room menu, so render and mouse agree on where it is.
pub(crate) fn room_menu_rect(menu: &crate::app::state::RoomRowMenu, popup: Rect) -> Rect {
    let width = display_width_u16(ROOM_MENU_DELETE_CONFIRM).saturating_add(4);
    let x = menu.x.min(popup.right().saturating_sub(width));
    let y = menu.y.min(popup.bottom().saturating_sub(3));
    Rect::new(x, y, width, 3)
}

/// Whether the pointer is on the menu's single item.
pub(crate) fn room_menu_item_at(rect: Rect, col: u16, row: u16) -> bool {
    row == rect.y.saturating_add(1) && col > rect.x && col < rect.right().saturating_sub(1)
}

/// `area` must match the rect the mouse layer measures against
/// (`sidebar_rect ∪ terminal_area`).
pub(super) fn render_room_browser(app: &AppState, frame: &mut Frame, area: Rect) {
    let chrome = room_chrome(app);
    let full = frame.area();
    super::dim_background(frame, full);
    let Some(layout) = room_layout(area, chrome) else {
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

    render_modal_header(frame, layout.header, "company rooms", &app.palette);
    let subtitle = Rect::new(
        layout.header.x,
        layout.header.y.saturating_add(1),
        layout.header.width,
        1,
    );
    let subtitle_line = match app.room_browser.selected_room() {
        Some(room) => Line::from(vec![
            Span::styled(
                truncate_end(&room.name, 28),
                Style::default()
                    .fg(app.palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {}  ", room.workspace_id),
                Style::default().fg(app.palette.overlay0),
            ),
            Span::styled(
                room.lifecycle.clone(),
                Style::default().fg(if room.lifecycle == "active" {
                    app.palette.green
                } else {
                    app.palette.yellow
                }),
            ),
            Span::styled(
                format!("   {}", truncate_end(&room.objective, 40)),
                Style::default().fg(app.palette.subtext0),
            ),
        ]),
        None => Line::from(Span::styled(
            "no rooms yet",
            Style::default().fg(app.palette.overlay0),
        )),
    };
    frame.render_widget(Paragraph::new(subtitle_line), subtitle);

    render_tabs(app, frame, layout.tabs);
    render_room_list(app, frame, &layout);
    match chrome.view {
        RoomView::Compose => render_composer(app, frame, layout.content),
        RoomView::Form => render_form(app, frame, &layout),
        RoomView::Bind => render_bind_picker(app, frame, layout.content),
        RoomView::Browse => match app.room_browser.tab {
            RoomTab::Conversation => render_conversation(app, frame, layout.content),
            RoomTab::Members => render_members(app, frame, layout.content),
            RoomTab::Memory => render_memory(app, frame, layout.content),
        },
    }
    render_footer(app, frame, layout.footer, chrome);

    if let Some(menu) = app.room_browser.row_menu.as_ref() {
        render_room_menu(app, frame, menu, layout.popup);
    }

    for ((rect, _), (hint, label, _, primary)) in layout.buttons.iter().zip(button_specs(chrome)) {
        let style = if primary {
            Style::default()
                .fg(panel_contrast_fg(&app.palette))
                .bg(app.palette.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(app.palette.text)
                .bg(app.palette.surface0)
                .add_modifier(Modifier::BOLD)
        };
        render_action_button(frame, *rect, hint, label, style);
    }
}

fn render_room_menu(
    app: &AppState,
    frame: &mut Frame,
    menu: &crate::app::state::RoomRowMenu,
    popup: Rect,
) {
    let rect = room_menu_rect(menu, popup);
    if render_panel_shell(frame, rect, app.palette.accent, app.palette.panel_bg).is_none() {
        return;
    }
    let label = if menu.confirming {
        ROOM_MENU_DELETE_CONFIRM
    } else {
        ROOM_MENU_DELETE
    };
    // Deleting takes the room's history with it, so the item says so before
    // it will act.
    let style = if menu.confirming {
        Style::default()
            .fg(panel_contrast_fg(&app.palette))
            .bg(app.palette.red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(app.palette.text)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(format!(" {label} "), style))),
        Rect::new(
            rect.x.saturating_add(1),
            rect.y.saturating_add(1),
            rect.width.saturating_sub(2),
            1,
        ),
    );
}

fn render_tabs(app: &AppState, frame: &mut Frame, area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    for tab in RoomTab::ALL {
        let selected = tab == app.room_browser.tab;
        spans.push(Span::styled(
            format!(" {} ", tab.label()),
            if selected {
                Style::default()
                    .fg(panel_contrast_fg(&app.palette))
                    .bg(app.palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.palette.overlay0)
            },
        ));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_room_list(app: &AppState, frame: &mut Frame, layout: &RoomLayout) {
    let column = Rect::new(
        layout.room_list.x,
        layout.room_list.y.saturating_sub(1),
        layout.room_list.width.saturating_add(1),
        layout.room_list.height.saturating_add(1),
    );
    frame.render_widget(
        Block::default().borders(Borders::RIGHT).title(ROOMS_TITLE),
        column,
    );
    if app.room_browser.rooms.is_empty() {
        frame.render_widget(
            Paragraph::new(
                " no rooms yet\n\n Right-click a\n workspace, or\n press n, to\n create one.",
            ),
            layout.room_list,
        );
        return;
    }
    let scroll = room_list_scroll(layout, app.room_browser.selected, app.room_browser.scroll);
    let lines = app
        .room_browser
        .rooms
        .iter()
        .enumerate()
        .skip(scroll)
        .take(layout.room_list.height as usize)
        .map(|(index, room)| {
            let selected = index == app.room_browser.selected;
            let style = if selected {
                Style::default()
                    .fg(panel_contrast_fg(&app.palette))
                    .bg(app.palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.palette.text)
            };
            let live = room.members.iter().filter(|m| m.bound).count();
            Line::styled(
                format!(
                    " {}  ({}/{}) ",
                    truncate_end(&room.name, 18),
                    live,
                    room.members.len()
                ),
                style,
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), layout.room_list);
}

/// One rendered row of the conversation, and the turn it belongs to.
///
/// Render and mouse both build this list, so a click toggles the turn the
/// pointer is actually on even though turns have different heights.
pub(crate) struct ConversationRow {
    pub line: Line<'static>,
    pub event: usize,
}

/// Builds the conversation rows for one room.
///
/// A collapsed turn is two rows: who sent it and who it woke, then a single
/// truncated line. An expanded turn keeps the header and wraps its whole body,
/// so a long report can actually be read in place.
pub(crate) fn conversation_rows(
    app: &AppState,
    width: usize,
    palette: &crate::app::state::Palette,
) -> Vec<ConversationRow> {
    let Some(room) = app.room_browser.selected_room() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (index, event) in room.events.iter().enumerate() {
        let expanded = app.room_browser.expanded.contains(&event.event_id);
        // Naming the woken members makes the "visibility is not delivery"
        // split visible: a note that woke nobody says so.
        let routed = if event.recipients.is_empty() {
            "room note · woke nobody".to_string()
        } else {
            format!("→ {}", event.recipients.join(", "))
        };
        let marker = if expanded { "▾ " } else { "▸ " };
        let header = format!(" {marker}{} ", event.from);
        let pad = width
            .saturating_sub(super::text::display_width(&header))
            .saturating_sub(routed.chars().count() + 1);
        rows.push(ConversationRow {
            line: Line::from(vec![
                Span::styled(
                    header,
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" ".repeat(pad)),
                Span::styled(
                    routed,
                    Style::default().fg(if event.recipients.is_empty() {
                        palette.overlay0
                    } else {
                        palette.green
                    }),
                ),
            ]),
            event: index,
        });
        let body_style = Style::default().fg(palette.subtext0);
        if expanded {
            for line in super::agent_call::wrap_prompt(&event.body, width.saturating_sub(4).max(1))
            {
                rows.push(ConversationRow {
                    line: Line::from(Span::styled(format!("   {line}"), body_style)),
                    event: index,
                });
            }
        } else {
            rows.push(ConversationRow {
                line: Line::from(Span::styled(
                    format!(
                        "   {}",
                        truncate_end(&event.body.replace('\n', " "), width.saturating_sub(4))
                    ),
                    body_style,
                )),
                event: index,
            });
        }
    }
    rows
}

/// Turn under the pointer in the conversation pane, accounting for scroll.
pub(crate) fn conversation_event_at(app: &AppState, content: Rect, row: u16) -> Option<usize> {
    if row < content.y || row >= content.bottom() {
        return None;
    }
    let rows = conversation_rows(app, content.width as usize, &app.palette);
    let visible = content.height as usize;
    let first = conversation_first_row(rows.len(), app.room_browser.content_scroll, visible);
    rows.get(first + (row - content.y) as usize)
        .map(|row| row.event)
}

/// First row to draw, scrolling back from the newest turn.
///
/// `scroll_rows` counts rows, not turns. Counting turns made an expanded turn
/// taller than the pane impossible to read: one step jumped past its whole
/// body, so its header could never be brought into view.
pub(crate) fn conversation_first_row(rows_len: usize, scroll_rows: usize, visible: usize) -> usize {
    rows_len.saturating_sub(visible).saturating_sub(scroll_rows)
}

fn render_conversation(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(room) = app.room_browser.selected_room() else {
        return;
    };
    if room.events.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " nothing posted in this room yet",
                Style::default().fg(app.palette.overlay0),
            ))),
            area,
        );
        return;
    }
    let rows = conversation_rows(app, area.width as usize, &app.palette);
    let visible = area.height as usize;
    let first = conversation_first_row(rows.len(), app.room_browser.content_scroll, visible);
    let lines: Vec<Line> = rows
        .into_iter()
        .skip(first)
        .take(visible)
        .map(|row| row.line)
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_members(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(room) = app.room_browser.selected_room() else {
        return;
    };
    if room.members.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " no seats yet — click \"add seat\" below",
                Style::default().fg(app.palette.overlay0),
            ))),
            area,
        );
        return;
    }
    let scroll = app.room_browser.content_scroll.min(
        room.members
            .len()
            .saturating_sub(area.height.max(1) as usize),
    );
    let lines = room
        .members
        .iter()
        .skip(scroll)
        .take(area.height as usize)
        .map(|member| {
            let mut spans = vec![Span::styled(
                format!(" @{:<16}", truncate_end(&member.handle, 16)),
                Style::default().fg(app.palette.text),
            )];
            // Where the agent runs, in the operator's own words: the tab or
            // pane name is what tells two agents of the same kind apart.
            let place_width = member_place_width(area) as usize;
            if place_width > 0 {
                let place = member.place.as_deref().unwrap_or("");
                spans.push(Span::styled(
                    format!(
                        "{:<width$}",
                        truncate_end(place, place_width.saturating_sub(1)),
                        width = place_width
                    ),
                    Style::default().fg(app.palette.accent),
                ));
            }
            if member.orchestrator {
                spans.push(Span::styled(
                    format!("{:<12}", "orchestrator"),
                    Style::default().fg(app.palette.mauve),
                ));
            } else if let Some(role) = member.role.as_deref() {
                spans.push(Span::styled(
                    format!("{:<12}", truncate_end(role, 12)),
                    Style::default().fg(app.palette.overlay0),
                ));
            } else {
                spans.push(Span::raw(" ".repeat(12)));
            }
            // Whether the seat can actually receive, not merely whether
            // something is bound: a bound orchestrator stuck at a prompt used
            // to look exactly like one that was ready.
            let readiness = member.readiness;
            spans.push(Span::styled(
                format!("{:<9}", readiness.label()),
                Style::default().fg(match readiness {
                    crate::app::state::SeatReadiness::Ready => app.palette.green,
                    crate::app::state::SeatReadiness::Working => app.palette.yellow,
                    crate::app::state::SeatReadiness::Blocked
                    | crate::app::state::SeatReadiness::Offline => app.palette.red,
                    crate::app::state::SeatReadiness::NoAgent => app.palette.overlay0,
                }),
            ));
            // Mail owed to a seat is worth seeing: a seat that looks idle may
            // simply never have been handed what is queued for it. One badge
            // fits, so the most urgent state wins. Unconfirmed mail outranks
            // everything because nothing in the runtime will move it on its
            // own; waiting outranks the rest because it is still actionable.
            if member.needs_attention > 0 {
                spans.push(Span::styled(
                    format!("{} stuck", member.needs_attention),
                    Style::default().fg(app.palette.red),
                ));
            } else if member.waiting > 0 {
                spans.push(Span::styled(
                    format!("{} waiting", member.waiting),
                    Style::default().fg(app.palette.yellow),
                ));
            } else if member.in_flight > 0 {
                spans.push(Span::styled(
                    format!("{} sending", member.in_flight),
                    Style::default().fg(app.palette.overlay0),
                ));
            } else if member.lost > 0 {
                spans.push(Span::styled(
                    format!("{} lost", member.lost),
                    Style::default().fg(app.palette.red),
                ));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);

    // Per-seat buttons are drawn from the same rects the mouse tests, so the
    // click target and the label can never drift apart.
    for (visible, member) in room
        .members
        .iter()
        .skip(scroll)
        .take(area.height as usize)
        .enumerate()
    {
        let confirming = app.room_browser.pending_remove.as_deref() == Some(&member.member_id);
        for (rect, action, label) in member_action_rects(area, visible, member.bound, confirming) {
            let style = match action {
                RoomMemberAction::Remove if confirming => Style::default()
                    .fg(panel_contrast_fg(&app.palette))
                    .bg(app.palette.red)
                    .add_modifier(Modifier::BOLD),
                _ => Style::default()
                    .fg(app.palette.text)
                    .bg(app.palette.surface0),
            };
            frame.render_widget(Paragraph::new(Span::styled(label, style)), rect);
        }
    }
}

/// The room creator and the seat form, which share one two-field shape.
fn render_form(app: &AppState, frame: &mut Frame, layout: &RoomLayout) {
    let (Some(form), Some(rects)) = (app.room_browser.form.as_ref(), layout.form) else {
        return;
    };
    let area = layout.content;
    let (description, primary_label, secondary_label) = match &form.kind {
        crate::app::state::RoomFormKind::CreateRoom { workspace_id } => {
            (format!(" new room in {workspace_id}"), "name", "objective")
        }
        crate::app::state::RoomFormKind::AddMember { room_name, .. } => (
            format!(" new seat in {}", truncate_end(room_name, 30)),
            "handle",
            "role",
        ),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_end(&description, area.width as usize),
            Style::default()
                .fg(app.palette.text)
                .add_modifier(Modifier::BOLD),
        ))),
        Rect::new(area.x, area.y, area.width, 1),
    );

    let focused = form.focus;
    for (rect, label, value, field, placeholder) in [
        (
            rects.primary,
            primary_label,
            form.primary.as_str(),
            crate::app::state::RoomFormField::Primary,
            if form.creating_room() {
                "what is this room called?"
            } else {
                "handle agents will @mention"
            },
        ),
        (
            rects.secondary,
            secondary_label,
            form.secondary.as_str(),
            crate::app::state::RoomFormField::Secondary,
            if form.creating_room() {
                "what is this room for? (optional)"
            } else {
                "what this seat does (optional)"
            },
        ),
    ] {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {label}"),
                Style::default().fg(app.palette.overlay0),
            ))),
            Rect::new(area.x, rect.y, FORM_LABEL_WIDTH, 1),
        );
        let selected = field == focused;
        let style = Style::default()
            .fg(if value.is_empty() {
                app.palette.overlay0
            } else {
                app.palette.text
            })
            .bg(if selected {
                app.palette.surface1
            } else {
                app.palette.surface0
            });
        let shown = if value.is_empty() { placeholder } else { value };
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(
                    " {}",
                    truncate_end(shown, rect.width.saturating_sub(1) as usize)
                ),
                style,
            ))
            .style(style),
            rect,
        );
        if selected {
            frame.set_cursor_position((
                rect.x
                    .saturating_add(1)
                    .saturating_add(display_width_u16(value))
                    .min(rect.right().saturating_sub(1)),
                rect.y,
            ));
        }
    }

    if let Some(toggle) = rects.orchestrator {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    if form.orchestrator { " [x] " } else { " [ ] " },
                    Style::default().fg(app.palette.mauve),
                ),
                Span::styled(
                    "orchestrator",
                    Style::default()
                        .fg(app.palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  — plans the work and speaks for the room",
                    Style::default().fg(app.palette.overlay0),
                ),
            ])),
            toggle,
        );
    }
}

/// Agents that could take the seat being bound.
fn render_bind_picker(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(bind) = app.room_browser.bind.as_ref() else {
        return;
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" seat ", Style::default().fg(app.palette.overlay0)),
            Span::styled(
                format!("@{}", truncate_end(&bind.handle, 20)),
                Style::default()
                    .fg(app.palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   pick the agent that takes it",
                Style::default().fg(app.palette.overlay0),
            ),
        ])),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let list = bind_list_rect(area);
    if bind.candidates.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " no running agents to bind — start one first",
                Style::default().fg(app.palette.overlay0),
            ))),
            list,
        );
        return;
    }
    let scroll = follow_selection_scroll(bind.selected, bind.scroll, list.height as usize);
    let lines = bind
        .candidates
        .iter()
        .enumerate()
        .skip(scroll)
        .take(list.height as usize)
        .map(|(index, candidate)| {
            let selected = index == bind.selected;
            let style = if selected {
                Style::default()
                    .fg(panel_contrast_fg(&app.palette))
                    .bg(app.palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.palette.text)
            };
            Line::from(vec![
                Span::styled(
                    format!(" {:<14}", truncate_end(&candidate.label, 14)),
                    style,
                ),
                Span::styled(
                    format!(
                        "{:<22}",
                        truncate_end(candidate.place.as_deref().unwrap_or(""), 21)
                    ),
                    if selected {
                        style
                    } else {
                        Style::default().fg(app.palette.accent)
                    },
                ),
                // The pane is what actually distinguishes two agents of the
                // same kind, so it stays visible while picking.
                Span::styled(
                    format!("{:<10}", truncate_end(&candidate.pane_id, 10)),
                    if selected {
                        style
                    } else {
                        Style::default().fg(app.palette.mauve)
                    },
                ),
                Span::styled(
                    format!("{:<16}", truncate_end(&candidate.workspace, 16)),
                    if selected {
                        style
                    } else {
                        Style::default().fg(app.palette.overlay0)
                    },
                ),
                Span::styled(
                    if candidate.same_workspace {
                        "this workspace"
                    } else {
                        ""
                    },
                    if selected {
                        style
                    } else {
                        Style::default().fg(app.palette.green)
                    },
                ),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), list);
}

/// Per-record action offered on a memory row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoomRecordAction {
    Accept,
    Delete,
}

fn record_action_label(action: RoomRecordAction, confirming: bool) -> &'static str {
    match action {
        RoomRecordAction::Accept => " accept ",
        RoomRecordAction::Delete => {
            if confirming {
                " delete? "
            } else {
                " delete "
            }
        }
    }
}

/// Right-aligned buttons for one record's title row, drawn and hit-tested from
/// the same rects.
pub(crate) fn record_action_rects(
    content: Rect,
    row_offset: u16,
    accepted: bool,
    confirming: bool,
) -> Vec<(Rect, RoomRecordAction, &'static str)> {
    if content.y.saturating_add(row_offset) >= content.bottom() {
        return Vec::new();
    }
    let y = content.y.saturating_add(row_offset);
    let mut rects = Vec::new();
    let mut right = content.right();
    // An accepted record needs no second approval, so only delete is offered.
    let actions: &[RoomRecordAction] = if accepted {
        &[RoomRecordAction::Delete]
    } else {
        &[RoomRecordAction::Delete, RoomRecordAction::Accept]
    };
    for action in actions {
        let label = record_action_label(*action, confirming);
        let width = display_width_u16(label);
        let Some(x) = right.checked_sub(width) else {
            return Vec::new();
        };
        if x < content.x.saturating_add(RECORD_TEXT_WIDTH) {
            return Vec::new();
        }
        rects.push((Rect::new(x, y, width, 1), *action, label));
        right = x.saturating_sub(1);
    }
    rects
}

/// Width reserved for a record's id, title, and state.
const RECORD_TEXT_WIDTH: u16 = 34;

/// One rendered row of the memory pane, and the record it belongs to.
pub(crate) struct RecordRow {
    pub line: Line<'static>,
    pub record: usize,
    /// Whether this row carries the record's action buttons.
    pub is_title: bool,
}

pub(crate) fn memory_rows(
    app: &AppState,
    width: usize,
    palette: &crate::app::state::Palette,
) -> Vec<RecordRow> {
    let Some(room) = app.room_browser.selected_room() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (index, record) in room.records.iter().enumerate() {
        let expanded = app.room_browser.expanded.contains(&record.record_id);
        let marker = if expanded { "▾ " } else { "▸ " };
        rows.push(RecordRow {
            line: Line::from(vec![
                Span::styled(format!(" {marker}"), Style::default().fg(palette.overlay0)),
                Span::styled(
                    truncate_end(&record.title, width.saturating_sub(30)),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if record.accepted {
                        "  accepted"
                    } else {
                        "  proposed"
                    },
                    Style::default().fg(if record.accepted {
                        palette.green
                    } else {
                        palette.yellow
                    }),
                ),
            ]),
            record: index,
            is_title: true,
        });
        rows.push(RecordRow {
            line: Line::from(Span::styled(
                format!(
                    "   {}  ·  {}",
                    truncate_end(&record.summary, width.saturating_sub(20)),
                    record.author
                ),
                Style::default().fg(palette.subtext0),
            )),
            record: index,
            is_title: false,
        });
        if expanded {
            for line in super::agent_call::wrap_prompt(&record.body, width.saturating_sub(5).max(1))
            {
                rows.push(RecordRow {
                    line: Line::from(Span::styled(
                        format!("    {line}"),
                        Style::default().fg(palette.subtext0),
                    )),
                    record: index,
                    is_title: false,
                });
            }
        }
    }
    rows
}

/// Record under the pointer, and whether the pointer is on its title row.
pub(crate) fn memory_row_at(app: &AppState, content: Rect, row: u16) -> Option<(usize, bool, u16)> {
    if row < content.y || row >= content.bottom() {
        return None;
    }
    let rows = memory_rows(app, content.width as usize, &app.palette);
    let scroll = memory_first_row(
        rows.len(),
        app.room_browser.content_scroll,
        content.height as usize,
    );
    let offset = (row - content.y) as usize;
    rows.get(scroll + offset)
        .map(|hit| (hit.record, hit.is_title, offset as u16))
}

/// First row to draw, where `content_scroll` counts rows from the top.
pub(crate) fn memory_first_row(rows_len: usize, scroll_rows: usize, visible: usize) -> usize {
    scroll_rows.min(rows_len.saturating_sub(visible))
}

fn render_memory(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(room) = app.room_browser.selected_room() else {
        return;
    };
    if room.records.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " no knowledge recorded yet",
                Style::default().fg(app.palette.overlay0),
            ))),
            area,
        );
        return;
    }
    let rows = memory_rows(app, area.width as usize, &app.palette);
    let scroll = memory_first_row(
        rows.len(),
        app.room_browser.content_scroll,
        area.height as usize,
    );
    let visible: Vec<&RecordRow> = rows
        .iter()
        .skip(scroll)
        .take(area.height as usize)
        .collect();
    frame.render_widget(
        Paragraph::new(
            visible
                .iter()
                .map(|row| row.line.clone())
                .collect::<Vec<_>>(),
        ),
        area,
    );

    // Buttons are drawn from the same rects the mouse tests.
    for (offset, row) in visible.iter().enumerate() {
        if !row.is_title {
            continue;
        }
        let Some(record) = room.records.get(row.record) else {
            continue;
        };
        let confirming = app.room_browser.pending_remove.as_deref() == Some(&record.record_id);
        for (rect, action, label) in
            record_action_rects(area, offset as u16, record.accepted, confirming)
        {
            let style = match action {
                RoomRecordAction::Delete if confirming => Style::default()
                    .fg(panel_contrast_fg(&app.palette))
                    .bg(app.palette.red)
                    .add_modifier(Modifier::BOLD),
                RoomRecordAction::Accept => Style::default()
                    .fg(panel_contrast_fg(&app.palette))
                    .bg(app.palette.green)
                    .add_modifier(Modifier::BOLD),
                _ => Style::default()
                    .fg(app.palette.text)
                    .bg(app.palette.surface0),
            };
            frame.render_widget(Paragraph::new(Span::styled(label, style)), rect);
        }
    }
}

fn render_composer(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(text) = app.room_browser.composer.as_deref() else {
        return;
    };
    let Some(room) = app.room_browser.selected_room() else {
        return;
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" post to ", Style::default().fg(app.palette.overlay0)),
            Span::styled(
                truncate_end(&room.name, 24),
                Style::default()
                    .fg(app.palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "   @handle addresses a member · @all the room",
                Style::default().fg(app.palette.overlay0),
            ),
        ])),
        Rect::new(area.x, area.y, area.width, 1),
    );

    let field = Rect::new(
        area.x,
        area.y.saturating_add(2),
        area.width,
        area.height.saturating_sub(2),
    );
    let style = Style::default()
        .fg(app.palette.text)
        .bg(app.palette.surface0);
    let width = field.width.saturating_sub(2).max(1) as usize;
    let wrapped = super::agent_call::wrap_prompt(text, width);
    let visible = field.height as usize;
    let first = wrapped.len().saturating_sub(visible);
    frame.render_widget(
        Paragraph::new(if text.is_empty() {
            vec![Line::from(Span::styled(
                " write a message… @handle or @all addresses members",
                Style::default()
                    .fg(app.palette.overlay0)
                    .bg(app.palette.surface0),
            ))]
        } else {
            wrapped
                .iter()
                .skip(first)
                .map(|line| Line::from(Span::styled(format!(" {line}"), style)))
                .collect()
        })
        .style(style),
        field,
    );

    let caret_row = wrapped.len().saturating_sub(first + 1) as u16;
    let caret_col = wrapped.last().map(|l| display_width_u16(l)).unwrap_or(0);
    frame.set_cursor_position((
        field
            .x
            .saturating_add(1)
            .saturating_add(caret_col)
            .min(field.right().saturating_sub(1)),
        field
            .y
            .saturating_add(caret_row)
            .min(field.bottom().saturating_sub(1)),
    ));
}

fn render_footer(app: &AppState, frame: &mut Frame, area: Rect, chrome: RoomChrome) {
    if let Some(notice) = app.room_browser.notice.as_deref() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_end(&format!(" {notice}"), area.width as usize),
                Style::default().fg(app.palette.yellow),
            ))),
            area,
        );
        return;
    }
    let hint = match chrome.view {
        RoomView::Compose => " ↵ send   alt+↵ newline   esc cancel",
        RoomView::Form => " click a field to edit it   tab next field   ↵ confirm   esc cancel",
        RoomView::Bind => " click an agent to seat it   ↑↓ pick   ↵ bind   esc cancel",
        // With no room, nothing that acts on one is offered.
        RoomView::Browse if !chrome.has_room => {
            " n new room   right-click a workspace to make one there   esc close"
        }
        RoomView::Browse if chrome.members_tab => {
            " click bind or remove on a seat   s add seat   n new room   esc close"
        }
        RoomView::Browse => " ↑↓ room   tab switch   u/d scroll   p post   n new room   esc close",
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_end(hint, area.width as usize),
            Style::default().fg(app.palette.overlay0),
        ))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{RoomEventRow, RoomMemberRow, RoomRecordRow, RoomSnapshot};
    use ratatui::{backend::TestBackend, Terminal};

    /// Chrome for a browsing operator with a room selected, which is what
    /// most geometry tests measure against.
    fn browse_chrome() -> RoomChrome {
        RoomChrome {
            view: RoomView::Browse,
            members_tab: false,
            has_room: true,
            paused: false,
            creating_room: false,
        }
    }

    fn rendered(app: &AppState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(110, 36)).expect("terminal");
        terminal
            .draw(|frame| render_room_browser(app, frame, frame.area()))
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

    fn sample_room() -> RoomSnapshot {
        RoomSnapshot {
            room_id: "room_1".into(),
            name: "Product team".into(),
            workspace_id: "w1".into(),
            objective: "Improve onboarding".into(),
            lifecycle: "active".into(),
            members: vec![
                RoomMemberRow {
                    member_id: "member_1".into(),
                    handle: "Codex".into(),
                    role: Some("developer".into()),
                    orchestrator: false,
                    bound: true,
                    readiness: crate::app::state::SeatReadiness::Ready,
                    place: Some("claude dep".into()),
                    waiting: 0,
                    in_flight: 0,
                    needs_attention: 0,
                    lost: 0,
                },
                RoomMemberRow {
                    member_id: "member_2".into(),
                    handle: "reviewer".into(),
                    role: None,
                    orchestrator: false,
                    bound: false,
                    readiness: crate::app::state::SeatReadiness::NoAgent,
                    place: None,
                    waiting: 2,
                    in_flight: 0,
                    needs_attention: 0,
                    lost: 0,
                },
                RoomMemberRow {
                    member_id: "member_3".into(),
                    handle: "orchestrator".into(),
                    role: None,
                    orchestrator: true,
                    bound: false,
                    readiness: crate::app::state::SeatReadiness::NoAgent,
                    place: None,
                    waiting: 0,
                    in_flight: 0,
                    needs_attention: 0,
                    lost: 1,
                },
            ],
            events: vec![
                RoomEventRow {
                    event_id: "evt_1".into(),
                    from: "host".into(),
                    body: "@Codex, overview the project".into(),
                    recipients: vec!["@Codex".into()],
                },
                RoomEventRow {
                    event_id: "evt_2".into(),
                    from: "host".into(),
                    body: "just a note for the record".into(),
                    recipients: Vec::new(),
                },
            ],
            records: vec![
                RoomRecordRow {
                    record_id: "rec_1".into(),
                    title: "Architecture map".into(),
                    summary: "how the runtime fits together".into(),
                    body: "the server owns the panes and renders frames".into(),
                    author: "host".into(),
                    accepted: true,
                },
                RoomRecordRow {
                    record_id: "rec_2".into(),
                    title: "Deployment claim".into(),
                    summary: "written by an agent".into(),
                    body: "deploy is deferred by design until the gates pass".into(),
                    author: "@Codex".into(),
                    accepted: false,
                },
            ],
        }
    }

    fn app_with_room() -> AppState {
        let mut app = AppState::test_new();
        app.room_browser.rooms = vec![sample_room()];
        app
    }

    #[test]
    fn the_browser_shows_the_room_header_and_tabs() {
        let text = rendered(&app_with_room());
        assert!(text.contains("company rooms"));
        assert!(text.contains("Product team"));
        assert!(text.contains("Improve onboarding"));
        assert!(text.contains("conversation"));
        assert!(text.contains("members"));
        assert!(text.contains("memory"));
    }

    #[test]
    fn the_conversation_names_who_each_turn_woke() {
        let text = rendered(&app_with_room());
        assert!(text.contains("@Codex, overview the project"));
        assert!(text.contains("→ @Codex"));
        // A post that addressed nobody must say so rather than look delivered.
        assert!(text.contains("room note · woke nobody"));
    }

    #[test]
    fn the_members_tab_distinguishes_bound_seats_and_the_orchestrator() {
        let mut app = app_with_room();
        app.room_browser.tab = RoomTab::Members;
        let text = rendered(&app);
        assert!(text.contains("@Codex"));
        // A seat reports whether it can receive, not merely that it is bound.
        assert!(text.contains("ready"));
        // Shortened to leave room for the mail counts beside it.
        assert!(text.contains("no agent"));
        assert!(text.contains("orchestrator"));
    }

    #[test]
    fn the_memory_tab_separates_accepted_from_proposed() {
        let mut app = app_with_room();
        app.room_browser.tab = RoomTab::Memory;
        let text = rendered(&app);
        assert!(text.contains("Architecture map"));
        assert!(text.contains("accepted"));
        assert!(text.contains("Deployment claim"));
        assert!(text.contains("proposed"));
    }

    #[test]
    fn an_empty_browser_explains_how_to_make_a_room() {
        let text = rendered(&AppState::test_new());
        assert!(text.contains("no rooms yet"));
        // An empty browser points at the two ways to make one, and offers the
        // button itself rather than only naming a CLI command.
        assert!(text.contains("Right-click a"));
        assert!(text.contains("new room"));
        // Nothing that acts on a room it does not have.
        assert!(!text.contains("post"));
        assert!(!text.contains("pause"));
    }

    #[test]
    fn the_composer_replaces_the_content_pane_and_swaps_the_buttons() {
        let mut app = app_with_room();
        app.room_browser.composer = Some("@Codex please look".into());
        let text = rendered(&app);
        assert!(text.contains("post to"));
        assert!(text.contains("@Codex please look"));
        assert!(text.contains("send"));
        // The browse-mode action must not linger while composing.
        assert!(!text.contains("p post"));
    }

    #[test]
    fn the_creator_names_the_workspace_it_will_bind_the_room_to() {
        let mut app = app_with_room();
        app.room_browser.form = Some(crate::app::state::RoomFormState {
            kind: crate::app::state::RoomFormKind::CreateRoom {
                workspace_id: "w7".into(),
            },
            primary: "Release team".into(),
            secondary: String::new(),
            orchestrator: false,
            focus: crate::app::state::RoomFormField::Primary,
        });
        let text = rendered(&app);
        assert!(text.contains("new room in w7"));
        assert!(text.contains("Release team"));
        assert!(text.contains("objective"));
        // An empty optional field says what it is for rather than sitting blank.
        assert!(text.contains("what is this room for?"));
        assert!(text.contains("create room"));
        // Creating a room offers no orchestrator box; seats do.
        assert!(!text.contains("orchestrator"));
    }

    #[test]
    fn the_seat_form_offers_the_orchestrator_box() {
        let mut app = app_with_room();
        app.room_browser.form = Some(crate::app::state::RoomFormState {
            kind: crate::app::state::RoomFormKind::AddMember {
                room_id: "room_1".into(),
                room_name: "Product team".into(),
            },
            primary: String::new(),
            secondary: String::new(),
            orchestrator: true,
            focus: crate::app::state::RoomFormField::Primary,
        });
        let text = rendered(&app);
        assert!(text.contains("new seat in Product team"));
        assert!(text.contains("[x] orchestrator"));
        assert!(text.contains("add seat"));
    }

    #[test]
    fn seat_buttons_are_drawn_where_the_mouse_looks_for_them() {
        let mut app = app_with_room();
        app.room_browser.tab = RoomTab::Members;
        let chrome = room_chrome(&app);
        let layout = room_layout(Rect::new(0, 0, 110, 36), chrome).expect("layout");

        // A bound seat offers unbind, an unbound one offers bind, and both
        // sit clear of the handle and role columns.
        let bound = member_action_rects(layout.content, 0, true, false);
        assert!(bound
            .iter()
            .any(|(_, action, _)| *action == RoomMemberAction::Unbind));
        let unbound = member_action_rects(layout.content, 1, false, false);
        assert!(unbound
            .iter()
            .any(|(_, action, _)| *action == RoomMemberAction::Bind));
        for (rect, _, _) in bound.iter().chain(unbound.iter()) {
            assert!(rect.x >= layout.content.x + member_text_width(layout.content));
            assert!(rect.right() <= layout.content.right());
        }

        // Every drawn button answers to a click anywhere inside it.
        for (rect, action, _) in member_action_rects(layout.content, 1, false, false) {
            for col in rect.x..rect.right() {
                assert_eq!(
                    member_action_at(&layout, 1, false, false, col, rect.y),
                    Some(action)
                );
            }
        }
        // A row beyond the seat list has no buttons at all.
        assert!(member_action_rects(layout.content, 999, false, false).is_empty());
    }

    #[test]
    fn a_turn_can_be_opened_in_full_and_closed_again() {
        let mut app = app_with_room();
        let long = "word ".repeat(80);
        if let Some(room) = app.room_browser.rooms.get_mut(0) {
            room.events[0].body = long.clone();
        }

        // Collapsed, one truncated line; the tail is not on screen.
        let text = rendered(&app);
        assert!(text.contains("▸ host"));
        assert!(!text.contains("▾ host"));

        app.room_browser.expanded.insert("evt_1".into());
        let text = rendered(&app);
        assert!(text.contains("▾ host"));
        // Expanding wraps the body over several rows rather than truncating.
        // The frame also carries the room list, so rows are matched by content
        // rather than by how they start.
        let wrapped = text.lines().filter(|line| line.contains("word ")).count();
        assert!(wrapped > 1, "expected a wrapped body, got {wrapped} rows");
    }

    #[test]
    fn a_click_toggles_the_turn_it_lands_on() {
        let mut app = app_with_room();
        // A turn only changes height when its body needs more than one row.
        if let Some(room) = app.room_browser.rooms.get_mut(0) {
            room.events[0].body = "word ".repeat(80);
        }
        let chrome = room_chrome(&app);
        let layout = room_layout(Rect::new(0, 0, 110, 36), chrome).expect("layout");
        let rows = conversation_rows(&app, layout.content.width as usize, &app.palette);
        assert_eq!(rows.len(), 4, "two turns, two rows each while collapsed");

        // Two rows per collapsed turn, in order from the top of the pane.
        let top = layout.content.y;
        assert_eq!(conversation_event_at(&app, layout.content, top), Some(0));
        assert_eq!(
            conversation_event_at(&app, layout.content, top + 1),
            Some(0)
        );
        assert_eq!(
            conversation_event_at(&app, layout.content, top + 2),
            Some(1)
        );
        assert_eq!(
            conversation_event_at(&app, layout.content, top + 3),
            Some(1)
        );

        // Expanding the first turn pushes the second down, and hit-testing
        // follows it rather than pointing at a stale row.
        app.room_browser.expanded.insert("evt_1".into());
        let rows = conversation_rows(&app, layout.content.width as usize, &app.palette);
        let second_starts = rows
            .iter()
            .position(|row| row.event == 1)
            .expect("second turn");
        assert!(second_starts > 2, "the expanded turn takes more rows");
        assert_eq!(
            conversation_event_at(&app, layout.content, top + second_starts as u16),
            Some(1)
        );
        assert_eq!(
            conversation_event_at(&app, layout.content, top + 2),
            Some(0)
        );
    }

    #[test]
    fn a_record_offers_review_actions_and_opens_in_full() {
        let mut app = app_with_room();
        app.room_browser.tab = RoomTab::Memory;
        let text = rendered(&app);
        assert!(text.contains("Architecture map"));
        assert!(text.contains("accept"), "a proposed record can be approved");
        assert!(text.contains("delete"));
        // The body stays hidden until the record is opened.
        assert!(!text.contains("deploy is deferred"));

        app.room_browser.expanded.insert("rec_2".into());
        let text = rendered(&app);
        assert!(text.contains("deploy is deferred"));

        // An accepted record needs no second approval.
        let chrome = room_chrome(&app);
        let layout = room_layout(Rect::new(0, 0, 110, 36), chrome).expect("layout");
        let accepted = record_action_rects(layout.content, 0, true, false);
        assert!(accepted
            .iter()
            .all(|(_, action, _)| *action == RoomRecordAction::Delete));
        let proposed = record_action_rects(layout.content, 0, false, false);
        assert!(proposed
            .iter()
            .any(|(_, action, _)| *action == RoomRecordAction::Accept));
        for (rect, _, _) in proposed {
            assert!(rect.x >= layout.content.x + RECORD_TEXT_WIDTH);
        }
    }

    #[test]
    fn the_bind_picker_names_the_pane_of_each_agent() {
        let mut app = app_with_room();
        app.room_browser.bind = Some(crate::app::state::RoomBindState {
            room_id: "room_1".into(),
            member_id: "member_1".into(),
            handle: "Codex".into(),
            candidates: vec![
                crate::app::state::RoomAgentCandidate {
                    pane_id: "w9:p1".into(),
                    label: "codex".into(),
                    place: None,
                    workspace: "SignalsTrader".into(),
                    same_workspace: true,
                },
                crate::app::state::RoomAgentCandidate {
                    pane_id: "wR:p1".into(),
                    label: "codex".into(),
                    place: None,
                    workspace: "SignalsTrader".into(),
                    same_workspace: false,
                },
            ],
            selected: 0,
            scroll: 0,
        });
        let text = rendered(&app);
        // Two agents of the same kind in the same-named workspace are only
        // told apart by their pane, so the pane has to be on screen.
        assert!(text.contains("w9:p1"));
        assert!(text.contains("wR:p1"));
    }

    #[test]
    fn the_room_menu_asks_twice_before_deleting() {
        let mut app = app_with_room();
        let menu = crate::app::state::RoomRowMenu {
            room_index: 0,
            x: 20,
            y: 10,
            confirming: false,
        };
        app.room_browser.row_menu = Some(menu);
        let text = rendered(&app);
        assert!(text.contains(ROOM_MENU_DELETE));
        assert!(!text.contains(ROOM_MENU_DELETE_CONFIRM));

        app.room_browser.row_menu = Some(crate::app::state::RoomRowMenu {
            confirming: true,
            ..menu
        });
        assert!(rendered(&app).contains(ROOM_MENU_DELETE_CONFIRM));

        // The item answers to a click anywhere along its width.
        let chrome = room_chrome(&app);
        let layout = room_layout(Rect::new(0, 0, 110, 36), chrome).expect("layout");
        let rect = room_menu_rect(&menu, layout.popup);
        for col in (rect.x + 1)..(rect.right() - 1) {
            assert!(room_menu_item_at(rect, col, rect.y + 1));
        }
        assert!(!room_menu_item_at(rect, rect.x, rect.y + 1));
        assert!(!room_menu_item_at(rect, rect.x + 1, rect.y));
    }

    #[test]
    fn the_browser_grows_with_the_screen() {
        // On a large terminal an expanded report needs room to be read.
        let chrome = RoomChrome {
            view: RoomView::Browse,
            members_tab: false,
            has_room: true,
            paused: false,
            creating_room: false,
        };
        let large = room_layout(Rect::new(0, 0, 240, 70), chrome).expect("layout");
        assert!(large.popup.width >= 220, "got {}", large.popup.width);
        assert!(large.popup.height >= 64, "got {}", large.popup.height);
        // A small screen still gets the minimum size, clamped to what fits.
        let small = room_layout(Rect::new(0, 0, 110, 36), chrome).expect("layout");
        assert!(small.popup.width >= 100);
        assert!(small.popup.width <= 106);
    }

    #[test]
    fn a_seat_shows_the_tab_or_pane_name_of_its_agent() {
        // Two agents of the same kind are told apart by the name the operator
        // gave their tab, so that name is on the seat row.
        let app = app_with_room();
        let mut members = app;
        members.room_browser.tab = RoomTab::Members;
        let mut terminal = Terminal::new(TestBackend::new(200, 50)).expect("terminal");
        terminal
            .draw(|frame| render_room_browser(&members, frame, frame.area()))
            .expect("render");
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("claude dep"), "the bound seat names its tab");
    }

    #[test]
    fn scrolling_the_conversation_moves_by_rows_not_whole_turns() {
        // With a turn expanded past the pane height, each step must move a
        // few rows; stepping a whole turn would jump past its header.
        let rows_len = 120;
        let visible = 20;
        assert_eq!(conversation_first_row(rows_len, 0, visible), 100);
        assert_eq!(conversation_first_row(rows_len, 3, visible), 97);
        assert_eq!(conversation_first_row(rows_len, 100, visible), 0);
        assert_eq!(conversation_first_row(rows_len, 500, visible), 0);
        // A short conversation starts at the top.
        assert_eq!(conversation_first_row(5, 0, visible), 0);
        assert_eq!(memory_first_row(rows_len, 3, visible), 3);
        assert_eq!(memory_first_row(rows_len, 500, visible), 100);
    }

    #[test]
    fn a_seat_says_why_it_is_not_receiving() {
        // Regression: a bound orchestrator stuck at a prompt, or whose agent
        // had stopped, rendered as "bound" — identical to one ready to work.
        let mut app = app_with_room();
        app.room_browser.tab = RoomTab::Members;
        let room = app.room_browser.rooms.get_mut(0).expect("room");
        room.members[0].readiness = crate::app::state::SeatReadiness::Blocked;
        room.members[0].waiting = 3;
        room.members[1].readiness = crate::app::state::SeatReadiness::Offline;
        room.members[2].readiness = crate::app::state::SeatReadiness::Working;
        let text = rendered(&app);
        assert!(text.contains("blocked"));
        assert!(text.contains("3 waiting"));
        assert!(text.contains("offline"));
        assert!(text.contains("working"));
        assert!(!text.contains("bound "), "the old catch-all label is gone");
    }

    #[test]
    fn a_seat_holding_unconfirmed_mail_says_so() {
        // Regression: the seat badge only knew Pending and Orphaned, so a
        // delivery interrupted mid-flight — the one state the runtime will
        // never resolve by itself — rendered as a seat owed nothing.
        let mut app = app_with_room();
        app.room_browser.tab = RoomTab::Members;
        let room = app.room_browser.rooms.get_mut(0).expect("room");
        room.members[0].needs_attention = 1;
        room.members[0].waiting = 4;
        room.members[1].waiting = 0;
        // Two digits is the widest badge a seat realistically shows; it must
        // still fit the column instead of truncating into nonsense.
        room.members[1].in_flight = 12;
        let text = rendered(&app);
        assert!(
            text.contains("1 stuck"),
            "mail that only a human can resolve must be the badge that shows"
        );
        assert!(
            !text.contains("4 waiting"),
            "unconfirmed outranks waiting on the same seat"
        );
        assert!(
            text.contains("12 sending"),
            "in-flight mail is still mail, and the badge must not be clipped"
        );
    }

    #[test]
    fn a_seat_shows_what_it_is_owed_and_what_it_lost() {
        let mut app = app_with_room();
        app.room_browser.tab = RoomTab::Members;
        let text = rendered(&app);
        // A seat that looks idle must not hide the mail queued behind it, nor
        // the mail it can no longer be given.
        assert!(text.contains("2 waiting"));
        assert!(text.contains("1 lost"));
        // A seat with nothing queued stays quiet.
        assert!(!text.contains("0 waiting"));
        assert!(!text.contains("0 lost"));
        // The counts must sit clear of the per-seat buttons rather than being
        // drawn over by them.
        let chrome = room_chrome(&app);
        let layout = room_layout(Rect::new(0, 0, 110, 36), chrome).expect("layout");
        let first_button = member_action_rects(layout.content, 1, false, false)
            .into_iter()
            .map(|(rect, _, _)| rect.x)
            .min()
            .expect("seat buttons");
        // indent + handle, then the place column, role, and readiness.
        let waiting_column =
            layout.content.x + 1 + 17 + member_place_width(layout.content) + 12 + 9;
        assert!(
            waiting_column + "2 waiting".len() as u16 <= first_button,
            "the waiting badge must end before the buttons begin"
        );
    }

    #[test]
    fn a_long_seat_list_scrolls_instead_of_hiding_its_tail() {
        let mut app = app_with_room();
        app.room_browser.tab = RoomTab::Members;
        let room = app.room_browser.rooms.get_mut(0).expect("room");
        room.members = (0..30)
            .map(|index| RoomMemberRow {
                member_id: format!("seat_{index}"),
                handle: format!("agent{index:02}"),
                role: None,
                orchestrator: false,
                bound: false,
                readiness: crate::app::state::SeatReadiness::NoAgent,
                place: None,
                waiting: 0,
                in_flight: 0,
                needs_attention: 0,
                lost: 0,
            })
            .collect();

        // Unscrolled, the list starts at the first seat and the last one is
        // out of view.
        let text = rendered(&app);
        assert!(text.contains("@agent00"));
        assert!(!text.contains("@agent29"));

        // Scrolled to the end, the tail is reachable.
        app.room_browser.content_scroll = 29;
        let text = rendered(&app);
        assert!(text.contains("@agent29"));
    }

    #[test]
    fn the_bind_picker_scrolls_to_keep_the_highlighted_agent_visible() {
        let mut app = app_with_room();
        app.room_browser.bind = Some(crate::app::state::RoomBindState {
            room_id: "room_1".into(),
            member_id: "member_1".into(),
            handle: "Codex".into(),
            candidates: (0..30)
                .map(|index| crate::app::state::RoomAgentCandidate {
                    pane_id: format!("w1:p{index}"),
                    label: format!("agent{index:02}"),
                    place: None,
                    workspace: "api".into(),
                    same_workspace: true,
                })
                .collect(),
            selected: 0,
            scroll: 0,
        });
        let text = rendered(&app);
        assert!(text.contains("agent00"));
        assert!(!text.contains("agent29"), "a tall list must not overflow");

        // Selecting the last candidate brings it into view.
        if let Some(bind) = app.room_browser.bind.as_mut() {
            bind.selected = 29;
        }
        let text = rendered(&app);
        assert!(text.contains("agent29"));

        // And a click on the drawn row resolves to that same candidate.
        let chrome = room_chrome(&app);
        let layout = room_layout(Rect::new(0, 0, 110, 36), chrome).expect("layout");
        let bind = app.room_browser.bind.as_ref().expect("bind");
        let scroll = bind_list_scroll(&layout, bind.selected, bind.scroll);
        let list = bind_list_rect(layout.content);
        let last_row = list.y + (29 - scroll) as u16;
        assert_eq!(
            bind_row_at(&layout, scroll, 30, list.x + 1, last_row),
            Some(29)
        );
    }

    #[test]
    fn the_seat_list_confirms_before_removing() {
        let mut app = app_with_room();
        app.room_browser.tab = RoomTab::Members;
        assert!(rendered(&app).contains(" remove "));
        app.room_browser.pending_remove = Some("member_1".into());
        let text = rendered(&app);
        assert!(text.contains(" remove? "), "the armed row asks again");
    }

    #[test]
    fn a_notice_replaces_the_key_hints() {
        let mut app = app_with_room();
        app.room_browser.notice = Some("no member named 'nobody' in this room".into());
        let text = rendered(&app);
        assert!(text.contains("no member named 'nobody'"));
        assert!(!text.contains("tab switch"));
    }

    #[test]
    fn tab_hit_testing_matches_the_rendered_strip() {
        let area = Rect::new(0, 0, 110, 36);
        let layout = room_layout(area, browse_chrome()).expect("layout");
        // Each label is clickable at its own offset, in order.
        let first = room_tab_at(&layout, layout.tabs.x + 2, layout.tabs.y);
        assert_eq!(first, Some(RoomTab::Conversation));
        assert_eq!(
            room_tab_at(&layout, layout.tabs.x + 2, layout.tabs.y + 1),
            None
        );
    }

    #[test]
    fn room_row_hit_testing_follows_the_list_scroll() {
        let area = Rect::new(0, 0, 110, 36);
        let layout = room_layout(area, browse_chrome()).expect("layout");
        assert_eq!(
            room_row_at(&layout, 3, 10, layout.room_list.x + 1, layout.room_list.y),
            Some(3)
        );
        // Above the first row is the column title, not a room.
        assert_eq!(
            room_row_at(
                &layout,
                0,
                10,
                layout.room_list.x + 1,
                layout.room_list.y - 1
            ),
            None
        );
    }

    #[test]
    fn the_room_list_scrolls_to_keep_the_selection_visible() {
        let area = Rect::new(0, 0, 110, 36);
        let layout = room_layout(area, browse_chrome()).expect("layout");
        let visible = layout.room_list.height as usize;
        assert_eq!(room_list_scroll(&layout, 0, 0), 0);
        assert_eq!(room_list_scroll(&layout, visible + 4, 0), 5);
    }
}
