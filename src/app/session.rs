use std::time::{Duration, Instant};

use super::{App, SESSION_SAVE_DEBOUNCE};

enum SessionSaveJob {
    Clear,
    Save {
        snapshot: Box<crate::persist::SessionSnapshot>,
        history: Option<crate::persist::SessionHistorySnapshot>,
    },
}

impl App {
    pub(super) fn schedule_session_save(&mut self) {
        if !self.no_session {
            self.session_save_deadline = Some(Instant::now() + SESSION_SAVE_DEBOUNCE);
        }
    }

    pub(crate) fn sync_session_save_schedule(&mut self) {
        if self.state.session_dirty {
            self.state.session_dirty = false;
            self.schedule_session_save();
        }
    }

    fn reap_finished_session_save(&mut self) {
        if self
            .session_save_thread
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            if let Some(thread) = self.session_save_thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn capture_session_save_job(&self) -> SessionSaveJob {
        if self.state.workspaces.is_empty()
            && self.state.collaboration.is_empty()
            && self.state.company.is_empty()
        {
            SessionSaveJob::Clear
        } else {
            let snapshot = crate::persist::capture(
                &self.state.workspaces,
                &self.state.terminals,
                &self.terminal_runtimes,
                self.state.active,
                self.state.selected,
                self.state.sidebar_width,
                self.state.sidebar_section_split,
                self.state.collapsed_space_keys.clone(),
                self.state.hidden_workspace_ids.clone(),
                &self.state.collaboration,
                &self.state.company,
            );
            let history = self.persist_pane_history.then(|| {
                crate::persist::capture_history(&self.state.workspaces, &self.terminal_runtimes)
            });
            SessionSaveJob::Save {
                snapshot: Box::new(snapshot),
                history,
            }
        }
    }

    pub(crate) fn start_background_session_save(&mut self) {
        if self.no_session {
            self.session_save_deadline = None;
            return;
        }

        self.reap_finished_session_save();
        if self.session_save_thread.is_some() {
            self.session_save_deadline = Some(Instant::now() + Duration::from_millis(250));
            return;
        }

        let job = self.capture_session_save_job();
        self.session_save_deadline = None;
        match std::thread::Builder::new()
            .name("herdr-session-save".into())
            .spawn(move || run_session_save_job(job))
        {
            Ok(thread) => self.session_save_thread = Some(thread),
            Err(err) => {
                tracing::warn!(err = %err, "failed to spawn session save thread; saving inline");
                run_session_save_job(self.capture_session_save_job());
            }
        }
    }

    pub(crate) fn save_session_now(&mut self) {
        if let Some(thread) = self.session_save_thread.take() {
            let _ = thread.join();
        }

        if self.no_session {
            self.session_save_deadline = None;
            return;
        }

        run_session_save_job(self.capture_session_save_job());
        self.session_save_deadline = None;
    }

    /// Synchronously persists the current session before an irreversible
    /// external side effect such as submitting bytes to a terminal.
    pub(crate) fn persist_session_barrier(&mut self) -> bool {
        if let Some(thread) = self.session_save_thread.take() {
            let _ = thread.join();
        }
        if self.no_session {
            return true;
        }
        let SessionSaveJob::Save { snapshot, history } = self.capture_session_save_job() else {
            return false;
        };
        match crate::persist::save_checked(&snapshot, history.as_ref()) {
            Ok(()) => {
                self.session_save_deadline = None;
                true
            }
            Err(err) => {
                tracing::warn!(%err, "failed to persist collaboration delivery transition");
                false
            }
        }
    }
}

fn run_session_save_job(job: SessionSaveJob) {
    match job {
        SessionSaveJob::Clear => crate::persist::clear(),
        SessionSaveJob::Save { snapshot, history } => {
            crate::persist::save(&snapshot, history.as_ref());
        }
    }
}
