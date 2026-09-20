//! Vrspi's presentation-only desktop chrome. No runtime state or PTY access.
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::text::{display_width_u16, truncate_end};
use crate::app::AppState;

pub(super) fn desktop_content_area(area: Rect, branded: bool) -> (Rect, Rect, Rect) {
    if !branded || area.height < 12 || area.width < 40 {
        return (area, Rect::default(), Rect::default());
    }
    (
        Rect::new(area.x, area.y + 2, area.width, area.height - 3),
        Rect::new(area.x, area.y, area.width, 2),
        Rect::new(area.x, area.bottom() - 1, area.width, 1),
    )
}

/// One masthead per frame. All values are client state; no terminal locks or scans.
pub(super) fn render_runtime_header(app: &AppState, frame: &mut Frame) {
    let area = app.view.runtime_header_rect;
    if area.is_empty() {
        return;
    }
    let p = &app.palette;
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(p.sidebar_bg).fg(p.text));
    let brand_width = 15.min(area.width);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ◈ ", Style::default().fg(p.accent)),
            Span::styled("V R S P I", Style::default().add_modifier(Modifier::BOLD)),
        ])),
        Rect::new(area.x, area.y, brand_width, 1),
    );
    let mode = match app.mode {
        crate::app::Mode::Terminal => " TERMINAL ",
        crate::app::Mode::Navigate => " NAVIGATE ",
        _ => " COMMAND ",
    };
    let mode_width = display_width_u16(mode);
    frame.render_widget(
        Paragraph::new(mode).style(Style::default().fg(p.accent).bg(p.active_row_bg)),
        Rect::new(area.right() - mode_width - 1, area.y, mode_width, 1),
    );
    if area.width >= 70 {
        frame.render_widget(
            Paragraph::new("R U N T I M E").style(Style::default().fg(p.overlay0)),
            Rect::new(
                area.x + brand_width + 2,
                area.y,
                area.width - brand_width - mode_width - 4,
                1,
            ),
        );
    }
    let buf = frame.buffer_mut();
    for x in area.x..area.right() {
        buf[(x, area.y + 1)].set_symbol("─").set_fg(p.surface_dim);
    }
    for x in area.x + 1..area.x + brand_width - 1 {
        buf[(x, area.y + 1)].set_fg(p.accent);
    }
}

pub(super) fn render_runtime_footer(app: &AppState, frame: &mut Frame) {
    let area = app.view.runtime_footer_rect;
    if area.is_empty() {
        return;
    }
    let p = &app.palette;
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(p.surface0).fg(p.subtext0));
    let workspace = app.active.and_then(|idx| app.workspaces.get(idx));
    let count = workspace
        .and_then(|ws| ws.active_tab())
        .map_or(0, |tab| tab.layout.pane_count());
    // Do not infer a local/remote connection or lobby membership from pane count.
    let right = format!(
        "{count} {}  /  RUNTIME ",
        if count == 1 { "pane" } else { "panes" }
    );
    let right_width = display_width_u16(&right).min(area.width);
    let label = workspace
        .map(|ws| {
            format!(
                "{} / {}",
                ws.display_name_from_terminals(&app.terminals),
                ws.active_tab_display_name().unwrap_or_default()
            )
        })
        .unwrap_or_else(|| "Make room for your next idea".into());
    let left_width = area.width.saturating_sub(right_width + 2);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ◇ ", Style::default().fg(p.accent)),
            Span::raw(truncate_end(&label, left_width.saturating_sub(3) as usize)),
        ])),
        Rect::new(area.x, area.y, left_width, 1),
    );
    frame.render_widget(
        Paragraph::new(right).style(Style::default().fg(p.overlay1)),
        Rect::new(area.right() - right_width, area.y, right_width, 1),
    );
}

pub(super) fn render_studio_empty(app: &AppState, frame: &mut Frame, area: Rect) {
    let p = &app.palette;
    frame
        .buffer_mut()
        .set_style(area, Style::default().bg(p.panel_bg).fg(p.text));
    let mut lines = Vec::new();
    if area.width >= 48 && area.height >= 18 {
        for line in [
            "         ╱╲         ",
            "    ╱╲  ╱  ╲  ╱╲    ",
            "   ╱  ╲╱    ╲╱  ╲   ",
            "   ╲  ╱╲    ╱╲  ╱   ",
            "    ╲╱  ╲  ╱  ╲╱    ",
            "         ╲╱         ",
        ] {
            lines.push(Line::from(Span::styled(
                line,
                Style::default().fg(p.accent),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "V R S P I",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "A space for ambitious work.",
        Style::default().fg(p.text),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Start a workspace. Bring your agents.",
        Style::default().fg(p.subtext0),
    )));
    lines.push(Line::from(Span::styled(
        "Build something worth making.",
        Style::default().fg(p.overlay0),
    )));
    lines.push(Line::from(""));
    let key = app
        .keybinds
        .new_workspace
        .label()
        .unwrap_or_else(|| "unset".into());
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(p.panel_bg)
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  new workspace", Style::default().fg(p.text)),
    ]));
    let height = (lines.len() as u16).min(area.height);
    let content = Rect::new(
        area.x,
        area.y + area.height.saturating_sub(height) / 2,
        area.width,
        height,
    );
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), content);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branded_app() -> AppState {
        let mut app = AppState::test_new();
        app.theme_name = "vrspi".into();
        app.palette = crate::app::state::Palette::vrspi();
        app.mode = crate::app::Mode::Terminal;
        app.hide_tab_bar_when_single_tab = true;
        app.show_agent_labels_on_pane_borders = true;
        app
    }

    #[test]
    fn branded_footer_stays_outside_panes_and_mobile_has_no_footer() {
        let mut app = branded_app();
        app.workspaces = vec![crate::workspace::Workspace::test_new("project")];
        app.active = Some(0);
        app.mobile_width_threshold = 60;
        for collapsed in [false, true] {
            app.sidebar_collapsed = collapsed;
            crate::ui::compute_view(&mut app, Rect::new(5, 3, 120, 30));
            let footer = app.view.runtime_footer_rect;
            let header = app.view.runtime_header_rect;
            assert_eq!(header, Rect::new(5, 3, 120, 2));
            assert_eq!(footer, Rect::new(5, 32, 120, 1));
            assert!(!app.view.sidebar_rect.intersects(header));
            assert!(!app.view.terminal_area.intersects(header));
            assert!(!app.view.sidebar_rect.intersects(footer));
            assert!(!app.view.terminal_area.intersects(footer));
            for pane in &app.view.pane_infos {
                assert!(!pane.inner_rect.intersects(header));
                assert!(!pane.inner_rect.intersects(footer));
            }
        }
        crate::ui::compute_view(&mut app, Rect::new(0, 0, 40, 30));
        assert!(app.view.runtime_footer_rect.is_empty());
        assert!(app.view.runtime_header_rect.is_empty());
    }

    #[test]
    fn studio_geometry_handles_hidden_sidebar_and_theme_transitions() {
        let mut app = branded_app();
        app.workspaces = vec![crate::workspace::Workspace::test_new("project")];
        app.active = Some(0);
        app.sidebar_collapsed = true;
        app.sidebar_collapsed_mode = crate::config::SidebarCollapsedModeConfig::Hidden;
        let area = Rect::new(7, 4, 120, 30);
        crate::ui::compute_view(&mut app, area);
        assert_eq!(app.view.terminal_area.x, area.x);
        assert_eq!(app.view.terminal_area.y, area.y + 2);
        app.theme_name = "terminal".into();
        crate::ui::compute_view(&mut app, area);
        assert!(app.view.runtime_header_rect.is_empty());
        assert!(app.view.runtime_footer_rect.is_empty());
        assert_eq!(app.view.terminal_area, area);
    }

    #[test]
    fn studio_empty_and_compact_frames_render_without_clipping_chrome() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = branded_app();
        for (width, height) in [(120, 34), (80, 24), (40, 12), (19, 8), (1, 1)] {
            let area = Rect::new(0, 0, width, height);
            crate::ui::compute_view(&mut app, area);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| crate::ui::render(&app, frame))
                .unwrap();
            if width == 120 {
                let buffer = terminal.backend().buffer();
                let rendered = buffer
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(rendered.contains("A space for ambitious work."));
                assert!(rendered.contains("new workspace"));
                if let Ok(path) = std::env::var("VRSPI_UI_EMPTY_PREVIEW") {
                    std::fs::write(path, buffer_svg(buffer)).unwrap();
                }
            }
        }
    }

    #[tokio::test]
    async fn branded_three_pane_preview_uses_real_renderer_and_status() {
        use crate::{
            detect::{Agent, AgentState},
            terminal::TerminalRuntime,
            workspace::Workspace,
        };
        use ratatui::{backend::TestBackend, layout::Direction, Terminal};
        let mut app = branded_app();
        let mut ws = Workspace::test_new("my-product");
        ws.identity_cwd = std::path::PathBuf::from("my-product");
        ws.cached_git_branch = Some("feature/auth".into());
        ws.cached_git_ahead_behind = None;
        ws.cached_git_space = None;
        ws.tabs[0].custom_name = Some("development".into());
        let builder = ws.tabs[0].root_pane;
        let reviewer = ws.test_split(Direction::Horizontal);
        let researcher = ws.test_split(Direction::Vertical);
        ws.tabs[0].layout.focus_pane(builder);
        app.workspaces = vec![ws];
        app.active = Some(0);
        app.ensure_test_terminals();
        let rows = [
            (builder, Agent::Claude, "builder", AgentState::Working,
                "~/projects/my-product · feature/auth\r\n\r\n› Implement session authentication\r\n\r\nI'll add the session handler and ask\r\nthe reviewer to check the boundary.\r\n\r\nReading src/auth/session.ts\r\nUpdating src/api/middleware.ts\r\n\r\n+ export async function validateSession(\r\n+   request: Request\r\n+ ) {\r\n+   const token = getSessionToken(request);\r\n+   return verify(token);\r\n+ }\r\n\r\nSession handler ready for review."),
            (reviewer, Agent::Codex, "reviewer", AgentState::Idle,
                "~/projects/my-product\r\n\r\nReady to review your changes.\r\n\r\nNo pending messages.\r\nWaiting for the next task."),
            (researcher, Agent::Gemini, "researcher", AgentState::Working,
                "› Map the authentication edge cases\r\n\r\nInspecting existing tests…\r\nChecking token refresh behavior…\r\n\r\nGathering context for the team."),
        ];
        crate::ui::compute_view(&mut app, Rect::new(0, 0, 160, 34));
        for (pane, agent, label, status, text) in rows {
            let terminal_id = app.workspaces[0].terminal_id(pane).unwrap().clone();
            let state = app.terminals.get_mut(&terminal_id).unwrap();
            state.set_agent_name(label.into());
            state.detected_agent = Some(agent);
            state.state = status;
            let info = app
                .view
                .pane_infos
                .iter()
                .find(|info| info.id == pane)
                .unwrap();
            app.workspaces[0].tabs[0].runtimes.insert(
                pane,
                TerminalRuntime::test_with_scrollback_bytes(
                    info.inner_rect.width,
                    info.inner_rect.height,
                    4096,
                    text.as_bytes(),
                ),
            );
        }
        let mut terminal = Terminal::new(TestBackend::new(160, 34)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(&app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        for expected in [
            "WORKSPACE",
            "02 / AGENTS",
            "V R S P I",
            "R U N T I M E",
            "working",
            "idle",
            "3 panes",
            "Claude Code",
            "Gemini CLI",
        ] {
            assert!(rendered.contains(expected), "missing {expected}");
        }
        // Rooms replaced lobbies, and the masthead no longer calls this a studio.
        assert!(!rendered.contains("Agent lobby"));
        assert!(!rendered.contains("S T U D I O"));
        // Optional visual QA artifact from the actual cell buffer, not a second UI implementation.
        if let Ok(path) = std::env::var("VRSPI_UI_PREVIEW") {
            std::fs::write(path, buffer_svg(buffer)).unwrap();
        }
    }

    fn buffer_svg(buffer: &ratatui::buffer::Buffer) -> String {
        use ratatui::style::Color;
        fn color(value: Color, fallback: &str) -> String {
            match value {
                Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
                _ => fallback.into(),
            }
        }
        let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"#0b0e11\"/>", buffer.area.width * 10, buffer.area.height * 20, buffer.area.width * 10, buffer.area.height * 20);
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                let bg = color(cell.bg, "#0b0e11");
                let fg = color(cell.fg, "#efe6d9");
                let weight = if cell.modifier.contains(Modifier::BOLD) {
                    "700"
                } else {
                    "400"
                };
                let symbol = cell
                    .symbol()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                svg.push_str(&format!("<rect x=\"{}\" y=\"{}\" width=\"10\" height=\"20\" fill=\"{bg}\"/><text x=\"{}\" y=\"{}\" font-family=\"Menlo,monospace\" font-size=\"13\" font-weight=\"{weight}\" fill=\"{fg}\">{symbol}</text>", x*10,y*20,x*10,y*20+15));
            }
        }
        svg.push_str("</svg>");
        svg
    }
    #[test]
    fn studio_chrome_partitions_the_viewport_without_overlap() {
        let area = Rect::new(7, 4, 160, 40);
        let (content, header, footer) = desktop_content_area(area, true);
        assert_eq!(content, Rect::new(7, 6, 160, 37));
        assert_eq!(header, Rect::new(7, 4, 160, 2));
        assert_eq!(footer, Rect::new(7, 43, 160, 1));
        assert_eq!(content.union(footer).union(header), area);
        assert!(!content.intersects(footer));
        assert!(!content.intersects(header));
        assert!(!header.intersects(footer));
    }
    #[test]
    fn tiny_and_legacy_views_do_not_lose_terminal_rows() {
        for area in [
            Rect::default(),
            Rect::new(2, 3, 80, 11),
            Rect::new(0, 0, 39, 40),
        ] {
            assert_eq!(
                desktop_content_area(area, true),
                (area, Rect::default(), Rect::default())
            );
        }
        let area = Rect::new(0, 0, 100, 40);
        assert_eq!(
            desktop_content_area(area, false),
            (area, Rect::default(), Rect::default())
        );
    }
}
