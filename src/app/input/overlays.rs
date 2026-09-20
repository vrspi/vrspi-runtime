use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders},
};

use crate::app::{
    state::{AppState, DragState, DragTarget, Mode, NavigatorTarget},
    App,
};

use super::{
    modal::{
        close_agent_call, keybind_help_back, leave_modal, modal_action_from_buttons, ModalAction,
    },
    ScrollbarClickTarget,
};

fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

impl App {
    pub(super) fn handle_overlay_mouse(&mut self, mouse: MouseEvent) -> bool {
        if self.state.mode == Mode::AgentLobbies {
            self.handle_lobby_browser_mouse(mouse);
            return true;
        }

        if self.state.mode == Mode::CompanyRooms {
            self.handle_room_browser_mouse(mouse);
            return true;
        }

        if self.state.mode == Mode::AgentCall {
            self.handle_agent_call_mouse(mouse);
            return true;
        }

        if self.state.mode == Mode::ReleaseNotes {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left)
                    if self
                        .state
                        .release_notes_close_button_at(mouse.column, mouse.row) =>
                {
                    self.dismiss_release_notes();
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(target) = self
                        .state
                        .release_notes_scrollbar_target_at(mouse.column, mouse.row)
                    {
                        match target {
                            ScrollbarClickTarget::Thumb { grab_row_offset } => {
                                self.state.drag = Some(DragState {
                                    target: DragTarget::ReleaseNotesScrollbar { grab_row_offset },
                                });
                            }
                            ScrollbarClickTarget::Track { offset_from_bottom } => {
                                self.state
                                    .set_release_notes_offset_from_bottom(offset_from_bottom);
                            }
                        }
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if let Some(DragState {
                        target: DragTarget::ReleaseNotesScrollbar { grab_row_offset },
                    }) = &self.state.drag
                    {
                        if let Some(offset_from_bottom) = self
                            .state
                            .release_notes_offset_for_drag_row(mouse.row, *grab_row_offset)
                        {
                            self.state
                                .set_release_notes_offset_from_bottom(offset_from_bottom);
                        }
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.state.drag = None;
                }
                MouseEventKind::ScrollUp => self.scroll_release_notes(-3),
                MouseEventKind::ScrollDown => self.scroll_release_notes(3),
                _ => {}
            }
            return true;
        }

        if self.state.mode == Mode::ProductAnnouncement {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left)
                    if self
                        .state
                        .product_announcement_close_button_at(mouse.column, mouse.row) =>
                {
                    self.dismiss_product_announcement();
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(target) = self
                        .state
                        .product_announcement_scrollbar_target_at(mouse.column, mouse.row)
                    {
                        match target {
                            ScrollbarClickTarget::Thumb { grab_row_offset } => {
                                self.state.drag = Some(DragState {
                                    target: DragTarget::ProductAnnouncementScrollbar {
                                        grab_row_offset,
                                    },
                                });
                            }
                            ScrollbarClickTarget::Track { offset_from_bottom } => self
                                .state
                                .set_product_announcement_offset_from_bottom(offset_from_bottom),
                        }
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if let Some(DragState {
                        target: DragTarget::ProductAnnouncementScrollbar { grab_row_offset },
                    }) = &self.state.drag
                    {
                        if let Some(offset_from_bottom) = self
                            .state
                            .product_announcement_offset_for_drag_row(mouse.row, *grab_row_offset)
                        {
                            self.state
                                .set_product_announcement_offset_from_bottom(offset_from_bottom);
                        }
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.state.drag = None;
                }
                MouseEventKind::ScrollUp => self.scroll_product_announcement(-3),
                MouseEventKind::ScrollDown => self.scroll_product_announcement(3),
                _ => {}
            }
            return true;
        }

        if self.state.mode == Mode::Navigator {
            match mouse.kind {
                MouseEventKind::Moved => {
                    if let Some(idx) = self.state.navigator_row_index_at_from(
                        &self.terminal_runtimes,
                        mouse.column,
                        mouse.row,
                    ) {
                        self.state.navigator.selected = idx;
                        self.state
                            .ensure_navigator_selection_visible_from(&self.terminal_runtimes);
                    }
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if self
                        .state
                        .navigator_search_contains(mouse.column, mouse.row)
                    {
                        self.state.navigator.search_focused = true;
                    } else if let Some(idx) = self.state.navigator_row_index_at_from(
                        &self.terminal_runtimes,
                        mouse.column,
                        mouse.row,
                    ) {
                        self.state.navigator.selected = idx;
                        let target = self
                            .state
                            .navigator_rows_from(&self.terminal_runtimes)
                            .get(idx)
                            .map(|row| (row.target.clone(), row.is_workspace));
                        if let Some((NavigatorTarget::Workspace { .. }, true)) = target {
                            if self.state.navigator_row_caret_at(mouse.column) {
                                self.state.toggle_selected_navigator_workspace_from(
                                    &self.terminal_runtimes,
                                );
                            } else {
                                self.state
                                    .accept_navigator_selection_from(&self.terminal_runtimes);
                            }
                        } else {
                            self.state
                                .accept_navigator_selection_from(&self.terminal_runtimes);
                        }
                    } else if !self.state.navigator_popup_contains(mouse.column, mouse.row) {
                        leave_modal(&mut self.state);
                    }
                }
                MouseEventKind::ScrollUp => {
                    self.state.navigator.scroll = self.state.navigator.scroll.saturating_sub(3);
                    self.state
                        .align_navigator_selection_to_scroll_from(&self.terminal_runtimes);
                }
                MouseEventKind::ScrollDown => {
                    let viewport = self.state.navigator_body_rect().height as usize;
                    let max = self
                        .state
                        .navigator_max_scroll_from(&self.terminal_runtimes, viewport);
                    self.state.navigator.scroll =
                        self.state.navigator.scroll.saturating_add(3).min(max);
                    self.state
                        .align_navigator_selection_to_scroll_from(&self.terminal_runtimes);
                }
                _ => {}
            }
            return true;
        }

        if self.state.mode == Mode::KeybindHelp {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left)
                    if self
                        .state
                        .keybind_help_close_button_at(mouse.column, mouse.row) =>
                {
                    keybind_help_back(&mut self.state);
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(target) = self
                        .state
                        .keybind_help_scrollbar_target_at(mouse.column, mouse.row)
                    {
                        match target {
                            ScrollbarClickTarget::Thumb { grab_row_offset } => {
                                self.state.drag = Some(DragState {
                                    target: DragTarget::KeybindHelpScrollbar { grab_row_offset },
                                });
                            }
                            ScrollbarClickTarget::Track { offset_from_bottom } => {
                                self.state
                                    .set_keybind_help_offset_from_bottom(offset_from_bottom);
                            }
                        }
                    } else {
                        let rect = self.state.keybind_help_popup_rect();
                        let inside = mouse.column >= rect.x
                            && mouse.column < rect.x + rect.width
                            && mouse.row >= rect.y
                            && mouse.row < rect.y + rect.height;
                        if !inside {
                            leave_modal(&mut self.state);
                        }
                    }
                }
                MouseEventKind::Drag(MouseButton::Left) => {
                    if let Some(DragState {
                        target: DragTarget::KeybindHelpScrollbar { grab_row_offset },
                    }) = &self.state.drag
                    {
                        if let Some(offset_from_bottom) = self
                            .state
                            .keybind_help_offset_for_drag_row(mouse.row, *grab_row_offset)
                        {
                            self.state
                                .set_keybind_help_offset_from_bottom(offset_from_bottom);
                        }
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.state.drag = None;
                }
                MouseEventKind::ScrollUp => self.state.scroll_keybind_help(-3),
                MouseEventKind::ScrollDown => self.state.scroll_keybind_help(3),
                _ => {}
            }
            return true;
        }

        false
    }
}

impl AppState {
    pub(crate) fn onboarding_full_area(&self) -> Rect {
        self.view.sidebar_rect.union(self.view.terminal_area)
    }

    pub(crate) fn navigator_popup_rect(&self) -> Rect {
        let area = self.onboarding_full_area();
        let margin_x = (area.width / 16).max(2);
        let margin_y = (area.height / 10).max(1);
        let width = area.width.saturating_sub(margin_x.saturating_mul(2));
        let height = area.height.saturating_sub(margin_y.saturating_mul(2));
        Rect::new(
            area.x + margin_x,
            area.y + margin_y,
            width.max(4),
            height.max(4),
        )
    }

    pub(crate) fn navigator_inner_rect(&self) -> Rect {
        Block::default()
            .borders(Borders::ALL)
            .inner(self.navigator_popup_rect())
    }

    pub(crate) fn navigator_search_rect(&self) -> Rect {
        let inner = self.navigator_inner_rect();
        Rect::new(inner.x, inner.y, inner.width, inner.height.min(1))
    }

    pub(crate) fn navigator_body_rect(&self) -> Rect {
        let inner = self.navigator_inner_rect();
        if inner.height <= 4 {
            return Rect::default();
        }
        Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(4),
        )
    }

    pub(crate) fn navigator_detail_rect(&self) -> Rect {
        let inner = self.navigator_inner_rect();
        Rect::new(
            inner.x,
            inner.y + inner.height.saturating_sub(2),
            inner.width,
            inner.height.min(1),
        )
    }

    pub(crate) fn navigator_footer_rect(&self) -> Rect {
        let inner = self.navigator_inner_rect();
        Rect::new(
            inner.x,
            inner.y + inner.height.saturating_sub(1),
            inner.width,
            inner.height.min(1),
        )
    }

    pub(crate) fn navigator_popup_contains(&self, col: u16, row: u16) -> bool {
        rect_contains(self.navigator_popup_rect(), col, row)
    }

    pub(crate) fn navigator_search_contains(&self, col: u16, row: u16) -> bool {
        rect_contains(self.navigator_search_rect(), col, row)
    }

    pub(crate) fn navigator_row_index_at_from(
        &self,
        terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
        col: u16,
        row: u16,
    ) -> Option<usize> {
        let body = self.navigator_body_rect();
        if !rect_contains(body, col, row) {
            return None;
        }
        let line_idx = self
            .navigator
            .scroll
            .saturating_add(row.saturating_sub(body.y) as usize);
        let lines = crate::app::state::navigator_display_lines(
            &self.navigator_rows_from(terminal_runtimes),
        );
        match lines.get(line_idx) {
            Some(crate::app::state::NavigatorDisplayLine::Row(idx)) => Some(*idx),
            _ => None,
        }
    }

    pub(crate) fn navigator_row_caret_at(&self, col: u16) -> bool {
        let body = self.navigator_body_rect();
        col <= body.x.saturating_add(3)
    }

    pub(super) fn onboarding_modal_inner(&self, popup_w: u16, popup_h: u16) -> Option<Rect> {
        // Welcome and release overlays render against the complete frame,
        // including the masthead and footer, unlike the collaboration browser.
        let area = self.screen_rect();
        let popup_w = popup_w.min(area.width.saturating_sub(4));
        let popup_h = popup_h.min(area.height.saturating_sub(2));
        if popup_w < 4 || popup_h < 4 {
            return None;
        }
        let popup_x = area.x + (area.width.saturating_sub(popup_w)) / 2;
        let popup_y = area.y + (area.height.saturating_sub(popup_h)) / 2;
        let popup = Rect::new(popup_x, popup_y, popup_w, popup_h);
        Some(Block::default().borders(Borders::ALL).inner(popup))
    }

    fn release_notes_modal_inner(&self) -> Option<Rect> {
        self.onboarding_modal_inner(
            crate::ui::RELEASE_NOTES_MODAL_SIZE.0,
            crate::ui::RELEASE_NOTES_MODAL_SIZE.1,
        )
    }

    fn product_announcement_modal_inner(&self) -> Option<Rect> {
        self.onboarding_modal_inner(
            crate::ui::PRODUCT_ANNOUNCEMENT_MODAL_SIZE.0,
            crate::ui::PRODUCT_ANNOUNCEMENT_MODAL_SIZE.1,
        )
    }

    fn release_notes_close_button_at(&self, col: u16, row: u16) -> bool {
        let Some(inner) = self.release_notes_modal_inner() else {
            return false;
        };
        if inner.height < 4 || inner.width < 12 {
            return false;
        }
        let button =
            crate::ui::release_notes_close_button_rect(Rect::new(inner.x, inner.y, inner.width, 1));
        col >= button.x
            && col < button.x + button.width
            && row >= button.y
            && row < button.y + button.height
    }

    pub(super) fn rename_modal_inner(&self) -> Option<Rect> {
        self.onboarding_modal_inner(56, 7)
    }

    fn release_notes_body_rect(&self) -> Option<Rect> {
        let inner = self.release_notes_modal_inner()?;
        if inner.height < 8 || inner.width < 4 {
            return None;
        }
        Some(crate::ui::modal_stack_areas(inner, 2, 1, 0, 1).content)
    }

    fn release_notes_scroll_metrics(&self) -> Option<crate::pane::ScrollMetrics> {
        let notes = self.release_notes.as_ref()?;
        let body = self.release_notes_body_rect()?;
        let viewport_rows = body.height.max(1) as usize;
        let lines = crate::ui::release_notes_display_lines(
            notes,
            &self.update_install_command,
            &self.palette,
        );

        let rows_for_width = |wrap_width: u16| {
            crate::ui::release_notes_wrapped_line_count(&lines, wrap_width.max(1))
        };

        let full_width = body.width.max(1);
        let mut total_rows = rows_for_width(full_width);
        let wrap_width = if total_rows > viewport_rows && full_width > 1 {
            body.width.saturating_sub(1).max(1)
        } else {
            full_width
        };
        total_rows = rows_for_width(wrap_width);

        let max_offset_from_bottom = total_rows.saturating_sub(viewport_rows);
        Some(crate::pane::ScrollMetrics {
            offset_from_bottom: max_offset_from_bottom.saturating_sub(notes.scroll as usize),
            max_offset_from_bottom,
            viewport_rows,
        })
    }

    pub(crate) fn release_notes_max_scroll(&self) -> u16 {
        self.release_notes_scroll_metrics()
            .map(|metrics| metrics.max_offset_from_bottom as u16)
            .unwrap_or(0)
    }

    fn release_notes_scrollbar_target_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<ScrollbarClickTarget> {
        let body = self.release_notes_body_rect()?;
        let metrics = self.release_notes_scroll_metrics()?;
        let track = crate::ui::release_notes_scrollbar_rect(body, metrics)?;
        if !(col >= track.x
            && col < track.x + track.width
            && row >= track.y
            && row < track.y + track.height)
        {
            return None;
        }
        if let Some(grab_row_offset) = crate::ui::scrollbar_thumb_grab_offset(metrics, track, row) {
            Some(ScrollbarClickTarget::Thumb { grab_row_offset })
        } else {
            Some(ScrollbarClickTarget::Track {
                offset_from_bottom: crate::ui::scrollbar_offset_from_row(metrics, track, row),
            })
        }
    }

    fn release_notes_offset_for_drag_row(&self, row: u16, grab_row_offset: u16) -> Option<usize> {
        let body = self.release_notes_body_rect()?;
        let metrics = self.release_notes_scroll_metrics()?;
        let track = crate::ui::release_notes_scrollbar_rect(body, metrics)?;
        Some(crate::ui::scrollbar_offset_from_drag_row(
            metrics,
            track,
            row,
            grab_row_offset,
        ))
    }

    fn set_release_notes_offset_from_bottom(&mut self, offset_from_bottom: usize) {
        let max_scroll = self.release_notes_max_scroll() as usize;
        if let Some(notes) = &mut self.release_notes {
            notes.scroll = max_scroll.saturating_sub(offset_from_bottom) as u16;
        }
    }

    fn product_announcement_close_button_at(&self, col: u16, row: u16) -> bool {
        let Some(inner) = self.product_announcement_modal_inner() else {
            return false;
        };
        if inner.height < 4 || inner.width < 12 {
            return false;
        }
        let button =
            crate::ui::release_notes_close_button_rect(Rect::new(inner.x, inner.y, inner.width, 1));
        col >= button.x
            && col < button.x + button.width
            && row >= button.y
            && row < button.y + button.height
    }

    fn product_announcement_body_rect(&self) -> Option<Rect> {
        let inner = self.product_announcement_modal_inner()?;
        if inner.height < 8 || inner.width < 4 {
            return None;
        }
        Some(crate::ui::modal_stack_areas(inner, 2, 1, 0, 1).content)
    }

    fn product_announcement_scroll_metrics(&self) -> Option<crate::pane::ScrollMetrics> {
        let announcement = self.product_announcement.as_ref()?;
        let body = self.product_announcement_body_rect()?;
        let viewport_rows = body.height.max(1) as usize;
        let lines = crate::ui::product_announcement_display_lines(announcement, &self.palette);

        let rows_for_width = |wrap_width: u16| {
            crate::ui::release_notes_wrapped_line_count(&lines, wrap_width.max(1))
        };

        let full_width = body.width.max(1);
        let mut total_rows = rows_for_width(full_width);
        let wrap_width = if total_rows > viewport_rows && full_width > 1 {
            body.width.saturating_sub(1).max(1)
        } else {
            full_width
        };
        total_rows = rows_for_width(wrap_width);

        let max_offset_from_bottom = total_rows.saturating_sub(viewport_rows);
        Some(crate::pane::ScrollMetrics {
            offset_from_bottom: max_offset_from_bottom.saturating_sub(announcement.scroll as usize),
            max_offset_from_bottom,
            viewport_rows,
        })
    }

    pub(crate) fn product_announcement_max_scroll(&self) -> u16 {
        self.product_announcement_scroll_metrics()
            .map(|metrics| metrics.max_offset_from_bottom as u16)
            .unwrap_or(0)
    }

    fn product_announcement_scrollbar_target_at(
        &self,
        col: u16,
        row: u16,
    ) -> Option<ScrollbarClickTarget> {
        let body = self.product_announcement_body_rect()?;
        let metrics = self.product_announcement_scroll_metrics()?;
        let track = crate::ui::release_notes_scrollbar_rect(body, metrics)?;
        if !(col >= track.x
            && col < track.x + track.width
            && row >= track.y
            && row < track.y + track.height)
        {
            return None;
        }
        if let Some(grab_row_offset) = crate::ui::scrollbar_thumb_grab_offset(metrics, track, row) {
            Some(ScrollbarClickTarget::Thumb { grab_row_offset })
        } else {
            Some(ScrollbarClickTarget::Track {
                offset_from_bottom: crate::ui::scrollbar_offset_from_row(metrics, track, row),
            })
        }
    }

    fn product_announcement_offset_for_drag_row(
        &self,
        row: u16,
        grab_row_offset: u16,
    ) -> Option<usize> {
        let body = self.product_announcement_body_rect()?;
        let metrics = self.product_announcement_scroll_metrics()?;
        let track = crate::ui::release_notes_scrollbar_rect(body, metrics)?;
        Some(crate::ui::scrollbar_offset_from_drag_row(
            metrics,
            track,
            row,
            grab_row_offset,
        ))
    }

    fn set_product_announcement_offset_from_bottom(&mut self, offset_from_bottom: usize) {
        let max_scroll = self.product_announcement_max_scroll() as usize;
        if let Some(announcement) = &mut self.product_announcement {
            announcement.scroll = max_scroll.saturating_sub(offset_from_bottom) as u16;
        }
    }

    pub(super) fn handle_onboarding_mouse(&mut self, mouse: MouseEvent) {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return;
        }

        let Some(inner) = self.onboarding_modal_inner(64, 16) else {
            return;
        };
        let actions = crate::ui::modal_stack_areas(inner, 2, 0, 1, 1)
            .actions
            .unwrap_or_default();
        let button = crate::ui::onboarding_welcome_continue_rect(actions);
        if modal_action_from_buttons(mouse.column, mouse.row, &[(button, ModalAction::Continue)])
            == Some(ModalAction::Continue)
        {
            self.request_complete_onboarding = true;
        }
    }

    pub(super) fn keybind_help_popup_rect(&self) -> Rect {
        crate::ui::centered_popup_rect(self.screen_rect(), 76, 22).unwrap_or_default()
    }

    fn keybind_help_modal_inner(&self) -> Option<Rect> {
        self.onboarding_modal_inner(76, 22)
    }

    fn keybind_help_close_button_at(&self, col: u16, row: u16) -> bool {
        let Some(inner) = self.keybind_help_modal_inner() else {
            return false;
        };
        if inner.height < 4 || inner.width < 12 {
            return false;
        }
        let button =
            crate::ui::release_notes_close_button_rect(Rect::new(inner.x, inner.y, inner.width, 1));
        col >= button.x
            && col < button.x + button.width
            && row >= button.y
            && row < button.y + button.height
    }

    fn keybind_help_body_rect(&self) -> Option<Rect> {
        let inner = self.keybind_help_modal_inner()?;
        if inner.height < 6 || inner.width < 4 {
            return None;
        }
        Some(crate::ui::modal_stack_areas(inner, 2, 1, 0, 1).content)
    }

    fn keybind_help_scroll_metrics(&self) -> Option<crate::pane::ScrollMetrics> {
        let body = self.keybind_help_body_rect()?;
        let viewport_rows = body.height.max(1) as usize;
        let wrap_width = body.width.max(1) as usize;
        let total_rows = crate::ui::keybind_help_lines(self)
            .into_iter()
            .map(|(width, _)| width.max(1).div_ceil(wrap_width))
            .sum::<usize>();
        let max_offset_from_bottom = total_rows.saturating_sub(viewport_rows);
        Some(crate::pane::ScrollMetrics {
            offset_from_bottom: max_offset_from_bottom
                .saturating_sub(self.keybind_help.scroll as usize),
            max_offset_from_bottom,
            viewport_rows,
        })
    }

    fn keybind_help_scrollbar_target_at(&self, col: u16, row: u16) -> Option<ScrollbarClickTarget> {
        let body = self.keybind_help_body_rect()?;
        let metrics = self.keybind_help_scroll_metrics()?;
        let track = crate::ui::release_notes_scrollbar_rect(body, metrics)?;
        if !(col >= track.x
            && col < track.x + track.width
            && row >= track.y
            && row < track.y + track.height)
        {
            return None;
        }
        if let Some(grab_row_offset) = crate::ui::scrollbar_thumb_grab_offset(metrics, track, row) {
            Some(ScrollbarClickTarget::Thumb { grab_row_offset })
        } else {
            Some(ScrollbarClickTarget::Track {
                offset_from_bottom: crate::ui::scrollbar_offset_from_row(metrics, track, row),
            })
        }
    }

    fn keybind_help_offset_for_drag_row(&self, row: u16, grab_row_offset: u16) -> Option<usize> {
        let body = self.keybind_help_body_rect()?;
        let metrics = self.keybind_help_scroll_metrics()?;
        let track = crate::ui::release_notes_scrollbar_rect(body, metrics)?;
        Some(crate::ui::scrollbar_offset_from_drag_row(
            metrics,
            track,
            row,
            grab_row_offset,
        ))
    }

    pub(crate) fn keybind_help_max_scroll(&self) -> u16 {
        self.keybind_help_scroll_metrics()
            .map(|metrics| metrics.max_offset_from_bottom as u16)
            .unwrap_or(0)
    }

    fn set_keybind_help_offset_from_bottom(&mut self, offset_from_bottom: usize) {
        let max_scroll = self.keybind_help_max_scroll() as usize;
        self.keybind_help.scroll = max_scroll.saturating_sub(offset_from_bottom) as u16;
    }

    pub(super) fn scroll_keybind_help(&mut self, delta: i16) {
        let max_scroll = self.keybind_help_max_scroll();
        let current = self.keybind_help.scroll as i16;
        self.keybind_help.scroll = current.saturating_add(delta).clamp(0, max_scroll as i16) as u16;
    }
}

impl App {
    fn handle_room_browser_mouse(&mut self, mouse: MouseEvent) {
        let chrome = crate::ui::room_chrome(&self.state);
        let browsing = chrome.view == crate::ui::RoomView::Browse;
        let area = self.state.onboarding_full_area();
        let Some(layout) = crate::ui::room_layout(area, chrome) else {
            leave_modal(&mut self.state);
            return;
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Right) if browsing => {
                // Right-clicking a room offers what the button row cannot: an
                // action destructive enough to want its own confirmation.
                let scroll = crate::ui::room_list_scroll(
                    &layout,
                    self.state.room_browser.selected,
                    self.state.room_browser.scroll,
                );
                if let Some(index) = crate::ui::room_row_at(
                    &layout,
                    scroll,
                    self.state.room_browser.rooms.len(),
                    mouse.column,
                    mouse.row,
                ) {
                    self.state.room_browser.selected = index;
                    self.state.room_browser.scroll = scroll;
                    self.state.room_browser.notice = None;
                    self.state.room_browser.row_menu = Some(crate::app::state::RoomRowMenu {
                        room_index: index,
                        x: mouse.column,
                        y: mouse.row,
                        confirming: false,
                    });
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if !rect_contains(layout.popup, mouse.column, mouse.row) {
                    leave_modal(&mut self.state);
                    return;
                }
                // An open room menu takes the click before anything under it.
                if let Some(menu) = self.state.room_browser.row_menu {
                    let rect = crate::ui::room_menu_rect(&menu, layout.popup);
                    if crate::ui::room_menu_item_at(rect, mouse.column, mouse.row) {
                        self.delete_selected_room();
                    } else {
                        self.state.room_browser.row_menu = None;
                        self.state.room_browser.notice = None;
                    }
                    return;
                }
                if let Some(action) =
                    modal_action_from_buttons(mouse.column, mouse.row, &layout.buttons)
                {
                    self.apply_room_action(action);
                    return;
                }
                match chrome.view {
                    // While composing, clicks in the body must not switch
                    // rooms out from under the draft.
                    crate::ui::RoomView::Compose => {}
                    crate::ui::RoomView::Form => {
                        self.handle_room_form_click(&layout, mouse.column, mouse.row)
                    }
                    crate::ui::RoomView::Bind => {
                        self.handle_room_bind_click(&layout, mouse.column, mouse.row)
                    }
                    crate::ui::RoomView::Browse => {
                        self.handle_room_browse_click(&layout, chrome, mouse.column, mouse.row)
                    }
                }
            }
            MouseEventKind::ScrollUp if browsing => {
                if rect_contains(layout.content, mouse.column, mouse.row) {
                    self.scroll_room_content(1);
                } else {
                    self.move_room_selection(-1);
                }
            }
            MouseEventKind::ScrollDown if browsing => {
                if rect_contains(layout.content, mouse.column, mouse.row) {
                    self.scroll_room_content(-1);
                } else {
                    self.move_room_selection(1);
                }
            }
            MouseEventKind::ScrollUp if chrome.view == crate::ui::RoomView::Bind => {
                self.move_room_bind_selection(-1);
            }
            MouseEventKind::ScrollDown if chrome.view == crate::ui::RoomView::Bind => {
                self.move_room_bind_selection(1);
            }
            _ => {}
        }
    }

    /// One place where a room button becomes a runtime action, shared by the
    /// mouse and the keys that advertise the same buttons.
    pub(crate) fn apply_room_action(&mut self, action: crate::ui::RoomAction) {
        match action {
            crate::ui::RoomAction::Post => self.open_room_composer(),
            crate::ui::RoomAction::Send => self.submit_room_post(),
            crate::ui::RoomAction::Cancel => self.cancel_room_overlay(),
            crate::ui::RoomAction::Close => leave_modal(&mut self.state),
            crate::ui::RoomAction::NewRoom => self.open_room_creator_for_selection(),
            crate::ui::RoomAction::AddSeat => self.open_room_seat_form(),
            crate::ui::RoomAction::Submit => self.submit_room_form(),
            crate::ui::RoomAction::ToggleLifecycle => self.toggle_room_lifecycle(),
        }
    }

    /// Closes whichever overlay is open, leaving the browser itself up.
    pub(crate) fn cancel_room_overlay(&mut self) {
        self.state.room_browser.clear_overlays();
        self.state.room_browser.notice = None;
    }

    fn handle_room_form_click(&mut self, layout: &crate::ui::RoomLayout, col: u16, row: u16) {
        let Some(rects) = layout.form else {
            return;
        };
        if rect_contains(rects.primary, col, row) {
            self.focus_room_form_field(crate::app::state::RoomFormField::Primary);
        } else if rect_contains(rects.secondary, col, row) {
            self.focus_room_form_field(crate::app::state::RoomFormField::Secondary);
        } else if rects
            .orchestrator
            .is_some_and(|toggle| rect_contains(toggle, col, row))
        {
            self.toggle_room_form_orchestrator();
        }
    }

    fn handle_room_bind_click(&mut self, layout: &crate::ui::RoomLayout, col: u16, row: u16) {
        let Some((count, scroll)) = self.state.room_browser.bind.as_ref().map(|bind| {
            (
                bind.candidates.len(),
                crate::ui::bind_list_scroll(layout, bind.selected, bind.scroll),
            )
        }) else {
            return;
        };
        let Some(index) = crate::ui::bind_row_at(layout, scroll, count, col, row) else {
            return;
        };
        if let Some(bind) = self.state.room_browser.bind.as_mut() {
            bind.selected = index;
            bind.scroll = scroll;
        }
        self.confirm_room_bind();
    }

    fn handle_room_browse_click(
        &mut self,
        layout: &crate::ui::RoomLayout,
        chrome: crate::ui::RoomChrome,
        col: u16,
        row: u16,
    ) {
        if let Some(tab) = crate::ui::room_tab_at(layout, col, row) {
            self.set_room_tab(tab);
            return;
        }
        if chrome.members_tab && self.handle_room_member_click(layout, col, row) {
            return;
        }
        if rect_contains(layout.content, col, row) {
            match self.state.room_browser.tab {
                crate::app::state::RoomTab::Conversation => {
                    // Clicking a turn opens it in full; the pane shows one
                    // line per turn otherwise, which is too little to read.
                    if let Some(index) =
                        crate::ui::conversation_event_at(&self.state, layout.content, row)
                    {
                        if let Some(event_id) = self
                            .state
                            .room_browser
                            .selected_room()
                            .and_then(|room| room.events.get(index))
                            .map(|event| event.event_id.clone())
                        {
                            self.toggle_room_expanded(&event_id);
                        }
                        return;
                    }
                }
                crate::app::state::RoomTab::Memory => {
                    if self.handle_room_record_click(layout, col, row) {
                        return;
                    }
                }
                crate::app::state::RoomTab::Members => {}
            }
        }
        let scroll = crate::ui::room_list_scroll(
            layout,
            self.state.room_browser.selected,
            self.state.room_browser.scroll,
        );
        if let Some(index) = crate::ui::room_row_at(
            layout,
            scroll,
            self.state.room_browser.rooms.len(),
            col,
            row,
        ) {
            self.state.room_browser.selected = index;
            self.state.room_browser.scroll = scroll;
            self.state.room_browser.content_scroll = 0;
            self.state.room_browser.pending_remove = None;
            self.state.room_browser.notice = None;
        }
    }

    /// Returns whether the click landed on the memory list.
    fn handle_room_record_click(
        &mut self,
        layout: &crate::ui::RoomLayout,
        col: u16,
        row: u16,
    ) -> bool {
        let Some((index, is_title, offset)) =
            crate::ui::memory_row_at(&self.state, layout.content, row)
        else {
            return false;
        };
        let Some((record_id, accepted)) = self
            .state
            .room_browser
            .selected_room()
            .and_then(|room| room.records.get(index))
            .map(|record| (record.record_id.clone(), record.accepted))
        else {
            return false;
        };
        if is_title {
            let confirming = self.state.room_browser.pending_remove.as_deref() == Some(&record_id);
            for (rect, action, _) in
                crate::ui::record_action_rects(layout.content, offset, accepted, confirming)
            {
                if col >= rect.x && col < rect.right() && row == rect.y {
                    match action {
                        crate::ui::RoomRecordAction::Accept => self.accept_room_record(index),
                        crate::ui::RoomRecordAction::Delete => self.delete_room_record(index),
                    }
                    return true;
                }
            }
        }
        // Anywhere else on the record opens it in full, so the body can be
        // read before approving it.
        self.state.room_browser.pending_remove = None;
        self.toggle_room_expanded(&record_id);
        true
    }

    /// Returns whether the click landed on the seat list.
    fn handle_room_member_click(
        &mut self,
        layout: &crate::ui::RoomLayout,
        col: u16,
        row: u16,
    ) -> bool {
        let Some(seats) = self.state.room_browser.selected_room().map(|room| {
            room.members
                .iter()
                .map(|member| {
                    (
                        member.bound,
                        self.state.room_browser.pending_remove.as_deref()
                            == Some(member.member_id.as_str()),
                    )
                })
                .collect::<Vec<_>>()
        }) else {
            return false;
        };
        let scroll = self.state.room_browser.content_scroll.min(
            seats
                .len()
                .saturating_sub(layout.content.height.max(1) as usize),
        );
        for (visible, (bound, confirming)) in seats.iter().copied().skip(scroll).enumerate() {
            let Some(action) =
                crate::ui::member_action_at(layout, visible, bound, confirming, col, row)
            else {
                continue;
            };
            let index = scroll + visible;
            match action {
                crate::ui::RoomMemberAction::Bind => self.open_room_bind_picker(index),
                crate::ui::RoomMemberAction::Unbind => self.unbind_room_member(index),
                crate::ui::RoomMemberAction::Remove => self.remove_room_member(index),
            }
            return true;
        }
        // A click elsewhere on the seat list takes back a pending removal.
        if crate::ui::member_row_at(layout, scroll, seats.len(), col, row).is_some() {
            self.state.room_browser.pending_remove = None;
            return true;
        }
        false
    }

    fn handle_lobby_browser_mouse(&mut self, mouse: MouseEvent) {
        let area = self.state.onboarding_full_area();
        let Some(layout) = crate::ui::lobby_browser_layout(area) else {
            leave_modal(&mut self.state);
            return;
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !rect_contains(layout.popup, mouse.column, mouse.row)
                    || rect_contains(layout.close, mouse.column, mouse.row)
                {
                    leave_modal(&mut self.state);
                    return;
                }
                let scroll = crate::ui::lobby_list_scroll(
                    &layout,
                    self.state.lobby_browser.selected,
                    self.state.lobby_browser.scroll,
                );
                if let Some(index) = crate::ui::lobby_row_at(
                    &layout,
                    scroll,
                    self.state.lobby_browser.lobbies.len(),
                    mouse.column,
                    mouse.row,
                ) {
                    self.state.lobby_browser.selected = index;
                    self.state.lobby_browser.scroll = scroll;
                    self.state.lobby_browser.thread_scroll = 0;
                    self.state.refresh_lobby_browser();
                }
            }
            // Over the conversation the wheel scrolls history; anywhere else it
            // moves between lobbies.
            MouseEventKind::ScrollUp if rect_contains(layout.thread, mouse.column, mouse.row) => {
                self.state.lobby_browser.thread_scroll = self
                    .state
                    .lobby_browser
                    .thread_scroll
                    .saturating_add(1)
                    .min(self.state.lobby_browser.thread.len().saturating_sub(1));
            }
            MouseEventKind::ScrollDown if rect_contains(layout.thread, mouse.column, mouse.row) => {
                self.state.lobby_browser.thread_scroll =
                    self.state.lobby_browser.thread_scroll.saturating_sub(1);
            }
            MouseEventKind::ScrollUp => self.move_lobby_selection(&layout, -1),
            MouseEventKind::ScrollDown => self.move_lobby_selection(&layout, 1),
            _ => {}
        }
    }

    fn move_lobby_selection(&mut self, layout: &crate::ui::LobbyBrowserLayout, delta: isize) {
        if self.state.lobby_browser.lobbies.is_empty() {
            return;
        }
        let last = self.state.lobby_browser.lobbies.len() - 1;
        self.state.lobby_browser.selected = self
            .state
            .lobby_browser
            .selected
            .saturating_add_signed(delta)
            .min(last);
        self.state.lobby_browser.scroll = crate::ui::lobby_list_scroll(
            layout,
            self.state.lobby_browser.selected,
            self.state.lobby_browser.scroll,
        );
        self.state.lobby_browser.thread_scroll = 0;
        self.state.refresh_lobby_browser();
    }

    fn handle_agent_call_mouse(&mut self, mouse: MouseEvent) {
        let Some(step) = self.state.agent_call.as_ref().map(|call| call.step) else {
            leave_modal(&mut self.state);
            return;
        };
        let area = self.state.onboarding_full_area();
        let Some(layout) = crate::ui::agent_call_layout(area, step) else {
            close_agent_call(&mut self.state);
            return;
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !rect_contains(layout.popup, mouse.column, mouse.row) {
                    close_agent_call(&mut self.state);
                    return;
                }
                if let Some(action) =
                    modal_action_from_buttons(mouse.column, mouse.row, &layout.buttons)
                {
                    self.apply_agent_call_action(action);
                    return;
                }
                if step != crate::app::state::AgentCallStep::Pick {
                    return;
                }
                let Some(call) = self.state.agent_call.as_ref() else {
                    return;
                };
                let scroll = crate::ui::agent_call_scroll(
                    call.selected,
                    call.scroll,
                    layout.content.height as usize,
                );
                if let Some(index) = crate::ui::agent_call_row_at(
                    &layout,
                    scroll,
                    call.candidates.len(),
                    mouse.column,
                    mouse.row,
                ) {
                    if let Some(call) = self.state.agent_call.as_mut() {
                        // A second click on the highlighted agent is the fast
                        // path straight into the prompt.
                        let advance = call.selected == index;
                        call.selected = index;
                        call.scroll = scroll;
                        if advance {
                            call.step = crate::app::state::AgentCallStep::Compose;
                            call.error = None;
                        }
                    }
                }
            }
            MouseEventKind::ScrollUp if step == crate::app::state::AgentCallStep::Pick => {
                self.move_agent_call_selection(-1);
            }
            MouseEventKind::ScrollDown if step == crate::app::state::AgentCallStep::Pick => {
                self.move_agent_call_selection(1);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{MouseButton, MouseEventKind};
    use ratatui::layout::Rect;

    use super::super::{app_for_mouse_test, mouse};
    use super::*;

    #[test]
    fn clicking_keybind_help_close_button_closes_overlay() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::KeybindHelp;

        let rect = app.state.keybind_help_popup_rect();
        let inner = Rect::new(
            rect.x + 1,
            rect.y + 1,
            rect.width.saturating_sub(2),
            rect.height.saturating_sub(2),
        );
        let close =
            crate::ui::release_notes_close_button_rect(Rect::new(inner.x, inner.y, inner.width, 1));
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            close.x,
            close.y,
        ));

        assert_eq!(app.state.mode, Mode::Navigate);
    }

    #[test]
    fn clicking_keybind_help_back_button_leaves_help_open() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::KeybindHelp;
        app.state.keybind_help.search_focused = true;
        app.state.keybind_help.query = "work".into();

        let rect = app.state.keybind_help_popup_rect();
        let inner = Rect::new(
            rect.x + 1,
            rect.y + 1,
            rect.width.saturating_sub(2),
            rect.height.saturating_sub(2),
        );
        let back =
            crate::ui::release_notes_close_button_rect(Rect::new(inner.x, inner.y, inner.width, 1));
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            back.x,
            back.y,
        ));

        assert_eq!(app.state.mode, Mode::KeybindHelp);
        assert!(!app.state.keybind_help.search_focused);
        assert!(app.state.keybind_help.query.is_empty());
    }

    #[test]
    fn onboarding_hover_does_not_change_selection() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::Onboarding;

        let inner = app.state.onboarding_modal_inner(64, 16).unwrap();
        let content = crate::ui::modal_stack_areas(inner, 2, 0, 1, 1).content;
        app.handle_mouse(mouse(MouseEventKind::Moved, content.x + 2, content.y));

        assert!(!app.state.request_complete_onboarding);
    }

    #[test]
    fn onboarding_click_continue_requests_completion() {
        let mut app = app_for_mouse_test();
        app.state.mode = Mode::Onboarding;

        let inner = app.state.onboarding_modal_inner(64, 16).unwrap();
        let actions = crate::ui::modal_stack_areas(inner, 2, 0, 1, 1)
            .actions
            .unwrap();
        let continue_rect = crate::ui::onboarding_welcome_continue_rect(actions);
        app.handle_mouse(mouse(
            MouseEventKind::Down(MouseButton::Left),
            continue_rect.x,
            continue_rect.y,
        ));

        assert!(app.state.request_complete_onboarding);
    }

    #[test]
    fn release_notes_preview_scrollbar_uses_full_content_body() {
        let mut app = app_for_mouse_test();
        app.state.view.sidebar_rect = Rect::new(0, 0, 24, 16);
        app.state.view.terminal_area = Rect::new(24, 0, 96, 16);
        app.state.release_notes = Some(crate::app::state::ReleaseNotesState {
            version: "9.9.9".into(),
            body: "### Added\n- Custom command keybindings now accept an optional description field.\n\n### Fixed\n- Sidebar Git status refresh now deduplicates workspaces.\n- Large restored sessions no longer leave panes without shells after startup.\n- Pane shutdown no longer warns after the direct child has already exited.\n- Closing the last pane or tab in a parent worktree workspace now shows the existing confirmation before closing the whole worktree group.\n- Update prompts, toasts, and docs now distinguish installing a new binary from stopping or reattaching a running Herdr session to use it."
                .into(),
            scroll: 0,
            preview: true,
        });
        app.state.update_install_command = "brew update && brew upgrade herdr".into();

        let inner = app.state.release_notes_modal_inner().unwrap();
        let expected_body = crate::ui::modal_stack_areas(inner, 2, 1, 0, 1).content;
        let body = app.state.release_notes_body_rect().unwrap();

        assert_eq!(body, expected_body);

        let metrics = app.state.release_notes_scroll_metrics().unwrap();
        assert_eq!(metrics.viewport_rows, body.height as usize);
        assert!(metrics.max_offset_from_bottom > 0);

        let track = crate::ui::release_notes_scrollbar_rect(body, metrics).unwrap();
        assert_eq!(track.y, body.y);
        assert!(matches!(
            app.state
                .release_notes_scrollbar_target_at(track.x, track.y),
            Some(ScrollbarClickTarget::Thumb { .. } | ScrollbarClickTarget::Track { .. })
        ));
    }

    #[test]
    fn agent_call_modal_buttons_are_clickable_where_they_render() {
        use crate::app::state::{AgentCallCandidate, AgentCallState, AgentCallStep};

        let mut app = super::super::app_for_mouse_test();
        app.state.agent_call = Some(AgentCallState {
            caller_pane_id: "w1:p1".into(),
            caller_label: "author".into(),
            candidates: vec![AgentCallCandidate {
                pane_id: "w1:p2".into(),
                label: "reviewer".into(),
                agent: "claude".into(),
                workspace: "herdr".into(),
                status: crate::api::schema::AgentStatus::Idle,
                connected: false,
            }],
            ..Default::default()
        });
        app.state.mode = Mode::AgentCall;

        let area = app.state.onboarding_full_area();
        let layout = crate::ui::agent_call_layout(area, AgentCallStep::Pick).expect("layout");
        let (next_rect, _) = layout
            .buttons
            .iter()
            .find(|(_, action)| *action == crate::ui::AgentCallAction::Next)
            .copied()
            .expect("next button");
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            next_rect.x + 1,
            next_rect.y,
        ));
        assert_eq!(
            app.state.agent_call.as_ref().map(|call| call.step),
            Some(AgentCallStep::Compose)
        );

        let layout = crate::ui::agent_call_layout(area, AgentCallStep::Compose).expect("layout");
        let (cancel_rect, _) = layout
            .buttons
            .iter()
            .find(|(_, action)| *action == crate::ui::AgentCallAction::Cancel)
            .copied()
            .expect("cancel button");
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            cancel_rect.x + 1,
            cancel_rect.y,
        ));
        assert!(app.state.agent_call.is_none());
        assert_ne!(app.state.mode, Mode::AgentCall);
    }

    #[tokio::test]
    async fn the_global_menu_row_for_company_rooms_is_clickable() {
        let mut app = super::super::app_for_mouse_test();
        let launcher = app.state.global_launcher_rect();
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            launcher.x,
            launcher.y,
        ));
        let row = app
            .state
            .global_menu_labels()
            .iter()
            .position(|label| *label == "company rooms")
            .expect("company rooms entry") as u16;
        let menu = app.state.global_menu_rect();
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            menu.x + 2,
            menu.y + 1 + row,
        ));
        // The click requests the browser; the App opens it on its next pass.
        assert!(app.state.request_open_room_browser);
    }

    /// The whole path the operator was promised: right-click a workspace,
    /// name the room, seat an agent, all without leaving the mouse except to
    /// type the names themselves.
    #[tokio::test]
    async fn a_room_is_created_and_seated_with_the_mouse() {
        use crate::app::state::{RoomFormField, RoomFormKind, RoomTab};

        let mut app = super::super::app_for_mouse_test();
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("api")];
        app.state.active = Some(0);
        let workspace_id = app.state.workspaces[0].id.clone();

        // The workspace context menu asks for a room; the App opens the
        // creator on its next pass.
        app.state.mode = Mode::ContextMenu;
        let menu = crate::app::state::ContextMenuState {
            kind: crate::app::state::ContextMenuKind::Workspace { ws_idx: 0 },
            x: 0,
            y: 0,
            list: crate::app::state::MenuListState::new(0),
        };
        let entry = menu
            .items()
            .iter()
            .position(|item| *item == crate::app::state::NEW_COMPANY_ROOM_ITEM)
            .expect("new company room entry");
        app.apply_context_menu_action_via_api(menu, entry);
        assert_eq!(app.state.request_new_company_room, Some(0));

        app.open_room_creator(0);
        assert_eq!(app.state.mode, Mode::CompanyRooms);
        assert_eq!(
            app.state.room_browser.form.as_ref().map(|form| &form.kind),
            Some(&RoomFormKind::CreateRoom {
                workspace_id: workspace_id.clone(),
            }),
            "the creator is pre-bound to the workspace that was right-clicked"
        );

        let area = app.state.onboarding_full_area();
        let layout =
            crate::ui::room_layout(area, crate::ui::room_chrome(&app.state)).expect("layout");
        let rects = layout.form.expect("form rects");

        // Clicking the objective field moves the caret there.
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            rects.secondary.x + 2,
            rects.secondary.y,
        ));
        assert_eq!(
            app.state.room_browser.form.as_ref().map(|form| form.focus),
            Some(RoomFormField::Secondary)
        );
        app.edit_room_form_field(|text| text.push_str("Ship onboarding"));
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            rects.primary.x + 2,
            rects.primary.y,
        ));
        app.edit_room_form_field(|text| text.push_str("Product team"));

        let (submit, _) = layout
            .buttons
            .iter()
            .find(|(_, action)| *action == crate::ui::RoomAction::Submit)
            .copied()
            .expect("submit button");
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            submit.x + 1,
            submit.y,
        ));
        assert!(app.state.room_browser.form.is_none());
        let room = app
            .state
            .room_browser
            .selected_room()
            .expect("the new room is selected");
        assert_eq!(room.name, "Product team");
        assert_eq!(room.objective, "Ship onboarding");
        assert_eq!(room.workspace_id, workspace_id);

        // The members tab offers a seat button that the conversation tab does
        // not, so the row is re-measured after switching.
        app.set_room_tab(RoomTab::Members);
        let layout =
            crate::ui::room_layout(area, crate::ui::room_chrome(&app.state)).expect("layout");
        let (seat, _) = layout
            .buttons
            .iter()
            .find(|(_, action)| *action == crate::ui::RoomAction::AddSeat)
            .copied()
            .expect("add seat button");
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            seat.x + 1,
            seat.y,
        ));
        let form_layout =
            crate::ui::room_layout(area, crate::ui::room_chrome(&app.state)).expect("layout");
        let toggle = form_layout
            .form
            .expect("form rects")
            .orchestrator
            .expect("the seat form offers an orchestrator box");
        app.edit_room_form_field(|text| text.push_str("Codex"));
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            toggle.x + 2,
            toggle.y,
        ));
        assert_eq!(
            app.state
                .room_browser
                .form
                .as_ref()
                .map(|form| form.orchestrator),
            Some(true)
        );
        // The lead seat is named for whoever fills it, so ticking the box on
        // a real handle succeeds.
        app.submit_room_form();
        assert!(app.state.room_browser.form.is_none());
        let seated = app
            .state
            .room_browser
            .selected_room()
            .expect("room")
            .members
            .clone();
        assert_eq!(seated.len(), 1);
        assert_eq!(seated[0].handle, "Codex");
        assert!(seated[0].orchestrator);

        // A second lead is refused, and the draft survives so the box can be
        // unticked rather than the handle retyped.
        app.open_room_seat_form();
        app.edit_room_form_field(|text| text.push_str("reviewer"));
        app.toggle_room_form_orchestrator();
        app.submit_room_form();
        assert!(app.state.room_browser.form.is_some());
        assert!(app.state.room_browser.notice.is_some());
        app.toggle_room_form_orchestrator();
        app.submit_room_form();
        assert!(app.state.room_browser.form.is_none());
        assert_eq!(
            app.state
                .room_browser
                .selected_room()
                .map(|room| room.members.len()),
            Some(2)
        );
    }

    /// Seat buttons act on the seat they are drawn on, and removal asks twice.
    #[tokio::test]
    async fn seat_rows_bind_and_remove_with_the_mouse() {
        use crate::app::state::RoomTab;

        let mut app = super::super::app_for_mouse_test();
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("api")];
        app.state.active = Some(0);
        let room = app
            .state
            .company
            .create_room(
                &crate::company::Actor::Host,
                app.state.workspaces[0].id.clone(),
                "Product team".into(),
                String::new(),
                1,
            )
            .expect("room");
        for handle in ["Codex", "reviewer"] {
            app.state
                .company
                .add_member(
                    &crate::company::Actor::Host,
                    &room.room_id,
                    handle.into(),
                    None,
                    false,
                )
                .expect("seat");
        }
        app.open_room_browser();
        app.set_room_tab(RoomTab::Members);

        let area = app.state.onboarding_full_area();
        let layout =
            crate::ui::room_layout(area, crate::ui::room_chrome(&app.state)).expect("layout");

        // The bind button on the second row opens the picker for that seat.
        let (bind, _, _) = crate::ui::member_action_rects(layout.content, 1, false, false)
            .into_iter()
            .find(|(_, action, _)| *action == crate::ui::RoomMemberAction::Bind)
            .expect("bind button");
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            bind.x + 1,
            bind.y,
        ));
        assert_eq!(
            app.state
                .room_browser
                .bind
                .as_ref()
                .map(|bind| bind.handle.clone()),
            Some("reviewer".into()),
            "the picker opens for the seat whose button was clicked"
        );
        app.cancel_room_bind();

        // Removal takes two clicks, and a click elsewhere takes back the first.
        let remove_rect = |index: usize, confirming: bool| {
            crate::ui::member_action_rects(layout.content, index, false, confirming)
                .into_iter()
                .find(|(_, action, _)| *action == crate::ui::RoomMemberAction::Remove)
                .map(|(rect, _, _)| rect)
                .expect("remove button")
        };
        let first = remove_rect(0, false);
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            first.x + 1,
            first.y,
        ));
        assert_eq!(
            app.state
                .room_browser
                .selected_room()
                .map(|room| room.members.len()),
            Some(2),
            "one click only arms the removal"
        );
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            layout.content.x + 1,
            layout.content.y + 1,
        ));
        assert!(app.state.room_browser.pending_remove.is_none());

        let armed = remove_rect(0, false);
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            armed.x + 1,
            armed.y,
        ));
        let confirm = remove_rect(0, true);
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            confirm.x + 1,
            confirm.y,
        ));
        assert_eq!(
            app.state
                .room_browser
                .selected_room()
                .map(|room| room.members.len()),
            Some(1),
            "the second click removes the seat"
        );
    }

    #[tokio::test]
    async fn the_room_browser_is_fully_operable_with_the_mouse() {
        use crate::app::state::{RoomSnapshot, RoomTab};

        let mut app = super::super::app_for_mouse_test();
        app.state
            .company
            .create_room(
                &crate::company::Actor::Host,
                "w1".into(),
                "Product team".into(),
                String::new(),
                1,
            )
            .expect("room");
        app.state
            .company
            .create_room(
                &crate::company::Actor::Host,
                "w1".into(),
                "Release team".into(),
                String::new(),
                1,
            )
            .expect("room");
        app.open_room_browser();
        assert_eq!(app.state.room_browser.rooms.len(), 2);

        let area = app.state.onboarding_full_area();
        let layout =
            crate::ui::room_layout(area, crate::ui::room_chrome(&app.state)).expect("layout");

        // Clicking the second room row selects it.
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            layout.room_list.x + 1,
            layout.room_list.y + 1,
        ));
        assert_eq!(app.state.room_browser.selected, 1);

        // Clicking a tab label switches panes.
        let members_x = layout.tabs.x + 1 + "conversation".len() as u16 + 3;
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            members_x + 1,
            layout.tabs.y,
        ));
        assert_eq!(app.state.room_browser.tab, RoomTab::Members);

        // The wheel over the room list moves the selection.
        app.handle_mouse(super::super::mouse(
            MouseEventKind::ScrollUp,
            layout.room_list.x + 1,
            layout.room_list.y,
        ));
        assert_eq!(app.state.room_browser.selected, 0);

        // The post button opens the composer, and the buttons swap. The
        // members tab offers an extra seat button, so the row is re-measured
        // rather than reused from before the tab switch.
        let browsing =
            crate::ui::room_layout(area, crate::ui::room_chrome(&app.state)).expect("layout");
        let (post_rect, _) = browsing
            .buttons
            .iter()
            .find(|(_, action)| *action == crate::ui::RoomAction::Post)
            .copied()
            .expect("post button");
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            post_rect.x + 1,
            post_rect.y,
        ));
        assert!(app.state.room_browser.composer.is_some());

        // While composing, a click in the body must not switch rooms away
        // from the draft.
        let composing =
            crate::ui::room_layout(area, crate::ui::room_chrome(&app.state)).expect("layout");
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            composing.room_list.x + 1,
            composing.room_list.y + 1,
        ));
        assert_eq!(app.state.room_browser.selected, 0);
        assert!(app.state.room_browser.composer.is_some());

        // Cancel returns to browsing.
        let (cancel_rect, _) = composing
            .buttons
            .iter()
            .find(|(_, action)| *action == crate::ui::RoomAction::Cancel)
            .copied()
            .expect("cancel button");
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            cancel_rect.x + 1,
            cancel_rect.y,
        ));
        assert!(app.state.room_browser.composer.is_none());

        // A click outside the popup closes the browser.
        app.handle_mouse(super::super::mouse(
            MouseEventKind::Down(MouseButton::Left),
            layout.popup.x.saturating_sub(2),
            layout.popup.y,
        ));
        assert_ne!(app.state.mode, Mode::CompanyRooms);
        let _ = RoomSnapshot::default();
    }
}
