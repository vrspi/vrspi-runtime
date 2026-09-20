use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::text::{display_width, display_width_u16, truncate_end};
use super::widgets::{
    action_button_row_rects, centered_popup_rect, modal_stack_areas, panel_contrast_fg,
    render_action_button, render_modal_header, render_panel_shell, ActionButtonSpec,
};
use crate::api::schema::AgentStatus;
use crate::app::state::{AgentCallState, AgentCallStep, Palette};
use crate::app::AppState;

const POPUP_WIDTH: u16 = 78;
const POPUP_HEIGHT: u16 = 22;

/// Clickable action in the call modal. Keys and mouse resolve to the same set,
/// so both paths stay in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentCallAction {
    /// Pick step: accept the highlighted agent and start writing.
    Next,
    /// Compose step: connect and hand the prompt to the runtime.
    Send,
    /// Compose step: return to the picker.
    Back,
    /// Sent step: start a second call without reopening the modal.
    Again,
    Cancel,
}

/// Geometry for the call modal, computed once and shared by render and mouse.
pub(crate) struct AgentCallLayout {
    pub popup: Rect,
    pub header: Rect,
    pub content: Rect,
    pub footer: Rect,
    pub buttons: Vec<(Rect, AgentCallAction)>,
}

/// Buttons for a step, in row order: hint, label, action, and whether the
/// button is the step's primary (accent) action.
fn button_specs(
    step: AgentCallStep,
) -> Vec<(Option<&'static str>, &'static str, AgentCallAction, bool)> {
    match step {
        AgentCallStep::Pick => vec![
            (Some("↵"), "write prompt", AgentCallAction::Next, true),
            (Some("esc"), "cancel", AgentCallAction::Cancel, false),
        ],
        AgentCallStep::Compose => vec![
            (Some("↵"), "send", AgentCallAction::Send, true),
            (Some("^b"), "back", AgentCallAction::Back, false),
            (Some("esc"), "cancel", AgentCallAction::Cancel, false),
        ],
        AgentCallStep::Sent => vec![
            (Some("↵"), "done", AgentCallAction::Cancel, true),
            (Some("^n"), "call another", AgentCallAction::Again, false),
        ],
    }
}

pub(crate) fn agent_call_layout(area: Rect, step: AgentCallStep) -> Option<AgentCallLayout> {
    let popup = centered_popup_rect(area, POPUP_WIDTH, POPUP_HEIGHT)?;
    let inner = Rect::new(
        popup.x.saturating_add(1),
        popup.y.saturating_add(1),
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );
    if inner.height < 6 {
        return None;
    }
    let areas = modal_stack_areas(inner, 2, 1, 1, 1);
    let actions = areas.actions.unwrap_or(inner);
    let specs = button_specs(step);
    let rects = action_button_row_rects(
        actions,
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
    Some(AgentCallLayout {
        popup,
        header: areas.header,
        content: areas.content,
        footer: areas.footer.unwrap_or(inner),
        buttons,
    })
}

/// Index of the candidate row under the pointer, if any.
pub(crate) fn agent_call_row_at(
    layout: &AgentCallLayout,
    scroll: usize,
    count: usize,
    col: u16,
    row: u16,
) -> Option<usize> {
    let content = layout.content;
    if col < content.x || col >= content.x.saturating_add(content.width) {
        return None;
    }
    if row < content.y || row >= content.y.saturating_add(content.height) {
        return None;
    }
    let index = scroll + (row - content.y) as usize;
    (index < count).then_some(index)
}

/// First visible candidate so the highlighted row stays in view.
pub(crate) fn agent_call_scroll(selected: usize, scroll: usize, visible_rows: usize) -> usize {
    super::widgets::follow_selection_scroll(selected, scroll, visible_rows)
}

fn status_span(status: AgentStatus, palette: &Palette) -> Span<'static> {
    let (label, color) = match status {
        AgentStatus::Idle => ("idle", palette.green),
        AgentStatus::Working => ("working", palette.yellow),
        AgentStatus::Blocked => ("blocked", palette.red),
        AgentStatus::Done => ("done", palette.blue),
        AgentStatus::Unknown => ("unknown", palette.overlay0),
    };
    Span::styled(label, Style::default().fg(color))
}

/// `area` must be the same rect the mouse layer measures against
/// (`sidebar_rect ∪ terminal_area`), so a click lands on the button it looks
/// like it landed on.
pub(super) fn render_agent_call(app: &AppState, frame: &mut Frame, area: Rect) {
    let Some(call) = app.agent_call.as_ref() else {
        return;
    };
    let full = frame.area();
    super::dim_background(frame, full);
    let Some(layout) = agent_call_layout(area, call.step) else {
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

    render_modal_header(frame, layout.header, "call another agent", &app.palette);
    let subtitle = Rect::new(
        layout.header.x,
        layout.header.y.saturating_add(1),
        layout.header.width,
        1,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("calling as ", Style::default().fg(app.palette.overlay0)),
            Span::styled(
                truncate_end(&call.caller_label, 28),
                Style::default().fg(app.palette.text),
            ),
            Span::styled(
                format!("  ({})", call.caller_pane_id),
                Style::default().fg(app.palette.overlay0),
            ),
        ])),
        subtitle,
    );

    match call.step {
        AgentCallStep::Pick => render_pick(app, call, frame, &layout),
        AgentCallStep::Compose => render_compose(app, call, frame, &layout),
        AgentCallStep::Sent => render_sent(app, call, frame, &layout),
    }

    render_footer(app, call, frame, layout.footer);
    for ((rect, _), (hint, label, _, primary)) in layout.buttons.iter().zip(button_specs(call.step))
    {
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

fn render_footer(app: &AppState, call: &AgentCallState, frame: &mut Frame, area: Rect) {
    if let Some(error) = call.error.as_deref() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_end(&format!(" {error}"), area.width as usize),
                Style::default().fg(app.palette.red),
            ))),
            area,
        );
        return;
    }
    let hint = match call.step {
        AgentCallStep::Pick => " ↑↓ select   ↵ write prompt   esc cancel",
        AgentCallStep::Compose => " ↵ send   alt+↵ newline   ^b back   esc cancel",
        AgentCallStep::Sent => " ↵ done   ^n call another",
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            truncate_end(hint, area.width as usize),
            Style::default().fg(app.palette.overlay0),
        ))),
        area,
    );
}

fn render_pick(app: &AppState, call: &AgentCallState, frame: &mut Frame, layout: &AgentCallLayout) {
    let content = layout.content;
    if call.candidates.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    " no other agent is running",
                    Style::default().fg(app.palette.text),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " Start a second agent in any workspace, then call it from here.",
                    Style::default().fg(app.palette.overlay0),
                )),
            ]),
            content,
        );
        return;
    }
    let visible = content.height as usize;
    let scroll = agent_call_scroll(call.selected, call.scroll, visible);
    let lines = call
        .candidates
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .map(|(index, candidate)| {
            let selected = index == call.selected;
            let base = if selected {
                Style::default()
                    .fg(panel_contrast_fg(&app.palette))
                    .bg(app.palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.palette.text)
            };
            let muted = if selected {
                base
            } else {
                Style::default().fg(app.palette.overlay0)
            };
            let mut spans = vec![
                Span::styled(format!(" {:<18}", truncate_end(&candidate.label, 18)), base),
                Span::styled(format!("{:<12}", truncate_end(&candidate.agent, 12)), muted),
                Span::styled(
                    format!("{:<16}", truncate_end(&candidate.workspace, 16)),
                    muted,
                ),
            ];
            if selected {
                spans.push(Span::styled(status_label(candidate.status), base));
            } else {
                spans.push(status_span(candidate.status, &app.palette));
            }
            if candidate.connected {
                spans.push(Span::styled("  linked", muted));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), content);
}

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "idle",
        AgentStatus::Working => "working",
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Unknown => "unknown",
    }
}

fn render_compose(
    app: &AppState,
    call: &AgentCallState,
    frame: &mut Frame,
    layout: &AgentCallLayout,
) {
    let content = layout.content;
    if content.height < 3 {
        return;
    }
    let target = call.selected_candidate();
    let (label, status) = match target {
        Some(candidate) => (candidate.label.clone(), candidate.status),
        None => (String::from("agent"), AgentStatus::Unknown),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" to ", Style::default().fg(app.palette.overlay0)),
            Span::styled(
                truncate_end(&label, 28),
                Style::default()
                    .fg(app.palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            status_span(status, &app.palette),
            Span::styled(
                match status {
                    AgentStatus::Idle => "  — delivered as soon as you send",
                    _ => "  — delivered the moment it goes idle",
                },
                Style::default().fg(app.palette.overlay0),
            ),
        ])),
        Rect::new(content.x, content.y, content.width, 1),
    );

    if let Some(forward) = call.forward.as_ref() {
        let mut spans = vec![Span::styled(
            format!(
                " forwarding {} turn(s) from \"{}\"",
                forward.entries.len(),
                truncate_end(&forward.source_lobby, 24)
            ),
            Style::default().fg(app.palette.blue),
        )];
        if forward.dropped > 0 {
            spans.push(Span::styled(
                format!("  ({} older turn(s) trimmed to fit)", forward.dropped),
                Style::default().fg(app.palette.yellow),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(content.x, content.y.saturating_add(1), content.width, 1),
        );
    }

    let field = Rect::new(
        content.x,
        content.y.saturating_add(2),
        content.width,
        content.height.saturating_sub(2),
    );
    let text_width = field.width.saturating_sub(2).max(1) as usize;
    let wrapped = wrap_prompt(&call.prompt, text_width);
    let visible_rows = field.height as usize;
    let first = wrapped.len().saturating_sub(visible_rows);
    let style = Style::default()
        .fg(app.palette.text)
        .bg(app.palette.surface0);
    let lines = wrapped
        .iter()
        .skip(first)
        .map(|line| Line::from(Span::styled(format!(" {line}"), style)))
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if call.prompt.is_empty() {
            vec![Line::from(Span::styled(
                if call.forward.is_some() {
                    " add a note to send with the forward (optional)…"
                } else {
                    " ask the other agent for something…"
                },
                Style::default()
                    .fg(app.palette.overlay0)
                    .bg(app.palette.surface0),
            ))]
        } else {
            lines
        })
        .style(style),
        field,
    );

    let caret_row = wrapped.len().saturating_sub(first + 1) as u16;
    let caret_col = wrapped
        .last()
        .map(|line| display_width_u16(line))
        .unwrap_or(0);
    let caret_x = field
        .x
        .saturating_add(1)
        .saturating_add(caret_col)
        .min(field.right().saturating_sub(1));
    let caret_y = field
        .y
        .saturating_add(caret_row)
        .min(field.bottom().saturating_sub(1));
    frame.set_cursor_position((caret_x, caret_y));
}

/// Wraps the prompt for the composer field. Always returns at least one line so
/// the caret has a row to sit on.
pub(super) fn wrap_prompt(prompt: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in prompt.split('\n') {
        let mut current = String::new();
        let mut current_width = 0;
        for ch in paragraph.chars() {
            let ch_width = display_width(&ch.to_string());
            if current_width + ch_width > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn render_sent(app: &AppState, call: &AgentCallState, frame: &mut Frame, layout: &AgentCallLayout) {
    let receipt = call
        .receipt
        .as_deref()
        .unwrap_or("prompt handed to runtime");
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(app.palette.green)),
            Span::styled(
                truncate_end(receipt, layout.content.width.saturating_sub(4) as usize),
                Style::default()
                    .fg(app.palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];
    for text in [
        "The runtime submits it into that agent's own prompt for you, labeled",
        "untrusted peer context so it is read as input, not instruction.",
        "",
        "Open agent lobbies from the global menu to see who is connected.",
    ] {
        lines.push(Line::from(Span::styled(
            format!(" {text}"),
            Style::default().fg(app.palette.overlay0),
        )));
    }
    frame.render_widget(Paragraph::new(lines), layout.content);
}

/// Border chip that opens this modal from the focused agent pane.
pub(crate) fn pane_call_button_label() -> &'static str {
    " ⇄ call agent "
}

/// Right-aligned button rect on a focused agent pane's top border, when the
/// pane is wide enough to carry it without eating the whole title.
pub(crate) fn pane_call_button_rect(pane_rect: Rect) -> Option<Rect> {
    let width = display_width_u16(pane_call_button_label());
    // Leave room for the pane's own corner plus a readable slice of its title.
    if pane_rect.width < width.saturating_add(12) {
        return None;
    }
    let x = pane_rect
        .x
        .saturating_add(pane_rect.width)
        .saturating_sub(1)
        .saturating_sub(width);
    Some(Rect::new(x, pane_rect.y, width, 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{AgentCallCandidate, AgentCallState, AgentCallStep};
    use ratatui::{backend::TestBackend, Terminal};

    fn candidate(label: &str, status: AgentStatus, connected: bool) -> AgentCallCandidate {
        AgentCallCandidate {
            pane_id: format!("w1:p{label}"),
            label: label.into(),
            agent: "claude".into(),
            workspace: "herdr".into(),
            status,
            connected,
        }
    }

    fn rendered(app: &AppState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 32)).expect("test terminal");
        terminal
            .draw(|frame| render_agent_call(app, frame, frame.area()))
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

    fn app_with(call: AgentCallState) -> AppState {
        let mut app = AppState::test_new();
        app.agent_call = Some(call);
        app
    }

    #[test]
    fn pick_step_lists_targets_with_status_and_connection() {
        let app = app_with(AgentCallState {
            caller_label: "author".into(),
            caller_pane_id: "w1:p1".into(),
            candidates: vec![
                candidate("reviewer", AgentStatus::Idle, true),
                candidate("builder", AgentStatus::Working, false),
            ],
            ..Default::default()
        });
        let text = rendered(&app);
        assert!(text.contains("call another agent"));
        assert!(text.contains("calling as author"));
        assert!(text.contains("reviewer"));
        assert!(text.contains("idle"));
        assert!(text.contains("linked"));
        assert!(text.contains("builder"));
        assert!(text.contains("working"));
        assert!(text.contains("write prompt"));
    }

    #[test]
    fn pick_step_explains_an_empty_roster() {
        let app = app_with(AgentCallState {
            caller_label: "author".into(),
            ..Default::default()
        });
        let text = rendered(&app);
        assert!(text.contains("no other agent is running"));
    }

    #[test]
    fn compose_step_shows_target_and_prompt() {
        let app = app_with(AgentCallState {
            caller_label: "author".into(),
            candidates: vec![candidate("reviewer", AgentStatus::Working, false)],
            prompt: "review the parser boundary".into(),
            step: AgentCallStep::Compose,
            ..Default::default()
        });
        let text = rendered(&app);
        assert!(text.contains("to reviewer"));
        assert!(text.contains("review the parser boundary"));
        assert!(text.contains("goes idle"));
        assert!(text.contains("send"));
    }

    #[test]
    fn compose_step_reports_the_error_instead_of_the_hint() {
        let app = app_with(AgentCallState {
            candidates: vec![candidate("reviewer", AgentStatus::Idle, false)],
            step: AgentCallStep::Compose,
            error: Some("message body exceeds 16384 bytes".into()),
            ..Default::default()
        });
        let text = rendered(&app);
        assert!(text.contains("message body exceeds 16384 bytes"));
        assert!(!text.contains("alt+↵ newline"));
    }

    #[test]
    fn sent_step_reports_what_the_runtime_will_do() {
        let app = app_with(AgentCallState {
            candidates: vec![candidate("reviewer", AgentStatus::Idle, true)],
            step: AgentCallStep::Sent,
            receipt: Some("queued for reviewer; delivers as soon as it goes idle".into()),
            ..Default::default()
        });
        let text = rendered(&app);
        assert!(text.contains("queued for reviewer"));
        assert!(text.contains("call another"));
    }

    #[test]
    fn wrap_prompt_breaks_long_input_and_keeps_explicit_newlines() {
        assert_eq!(wrap_prompt("abcdef", 3), vec!["abc", "def"]);
        assert_eq!(wrap_prompt("a\nb", 8), vec!["a", "b"]);
        assert_eq!(wrap_prompt("", 8), vec![""]);
    }

    #[test]
    fn scroll_follows_the_selection_in_both_directions() {
        assert_eq!(agent_call_scroll(0, 0, 3), 0);
        assert_eq!(agent_call_scroll(5, 0, 3), 3);
        assert_eq!(agent_call_scroll(1, 3, 3), 1);
    }

    #[test]
    fn row_hit_test_maps_pointer_rows_to_scrolled_candidates() {
        let area = Rect::new(0, 0, 100, 32);
        let layout = agent_call_layout(area, AgentCallStep::Pick).expect("layout");
        let content = layout.content;
        assert_eq!(
            agent_call_row_at(&layout, 2, 10, content.x + 1, content.y),
            Some(2)
        );
        assert_eq!(
            agent_call_row_at(&layout, 0, 1, content.x + 1, content.y + 4),
            None
        );
        assert_eq!(agent_call_row_at(&layout, 0, 10, 0, 0), None);
    }

    /// The mouse layer measures the modal against `sidebar_rect ∪
    /// terminal_area`; the full-frame render must place it there too, or clicks
    /// land a row away from the buttons they aim at.
    #[test]
    fn full_render_places_the_modal_where_the_mouse_layer_measures_it() {
        let mut app = AppState::test_new();
        // An odd frame height is where a mismatched origin shows up.
        app.view.sidebar_rect = Rect::new(0, 1, 26, 30);
        app.view.terminal_area = Rect::new(26, 1, 74, 30);
        app.view.tab_bar_rect = Rect::new(0, 0, 100, 1);
        app.agent_call = Some(AgentCallState {
            caller_label: "author".into(),
            candidates: vec![candidate("reviewer", AgentStatus::Idle, false)],
            ..Default::default()
        });
        app.mode = crate::app::Mode::AgentCall;

        let layout = agent_call_layout(
            app.view.sidebar_rect.union(app.view.terminal_area),
            AgentCallStep::Pick,
        )
        .expect("layout");
        let mut terminal = Terminal::new(TestBackend::new(100, 31)).expect("test terminal");
        terminal
            .draw(|frame| crate::ui::render(&app, frame))
            .expect("render");
        let buffer = terminal.backend().buffer();
        let header_row = (0..buffer.area.width)
            .map(|x| buffer[(x, layout.header.y)].symbol())
            .collect::<String>();
        assert!(header_row.contains("call another agent"));
    }

    #[test]
    fn pane_call_button_sits_inside_a_wide_pane_and_is_dropped_when_narrow() {
        let rect = pane_call_button_rect(Rect::new(4, 2, 60, 20)).expect("button");
        assert_eq!(rect.y, 2);
        assert!(rect.x > 4);
        assert!(rect.x + rect.width < 4 + 60);
        assert_eq!(pane_call_button_rect(Rect::new(0, 0, 12, 8)), None);
    }
}
