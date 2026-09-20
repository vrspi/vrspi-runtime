//! Durable journal for company task state.
//!
//! The session snapshot is a cache of the projection; this file is the record
//! of how it got there. Commands are committed here before any caller is told
//! what happened, so a crash between "the worker was told" and "the snapshot
//! was saved" loses nothing: recovery replays whatever the journal holds
//! beyond the restored snapshot.
//!
//! One entry per line, so a torn final write costs the last record rather
//! than the file.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use super::tasks::{JournalEntry, JournalSink, StoreError, TaskLedger};

/// Name of the journal beside the session snapshot.
pub(crate) const JOURNAL_FILE_NAME: &str = "company-tasks.journal";

/// An append-only, fsynced journal file.
#[derive(Debug, Clone)]
pub(crate) struct FileJournalSink {
    path: PathBuf,
}

impl FileJournalSink {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The journal beside the current session snapshot.
    pub(crate) fn for_session() -> Self {
        Self::new(crate::session::data_dir().join(JOURNAL_FILE_NAME))
    }

    /// Where the journal lives. Used by recovery tooling and tests; the
    /// running server only ever writes through `persist`.
    #[allow(dead_code)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Every durable entry, oldest first.
    ///
    /// A trailing partial line is dropped: it was never acknowledged, because
    /// the fsync that would have acknowledged it did not complete.
    pub(crate) fn read_all(&self) -> Result<Vec<JournalEntry>, StoreError> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(StoreError::Persistence {
                    detail: format!("reading {}: {err}", self.path.display()),
                })
            }
        };
        let mut entries = Vec::new();
        for line in BufReader::new(file).lines() {
            let Ok(line) = line else {
                // An unreadable tail is a torn write, not a corrupt history.
                break;
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<JournalEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(_) => break,
            }
        }
        Ok(entries)
    }
}

impl JournalSink for FileJournalSink {
    fn persist(&mut self, delta: &[JournalEntry]) -> Result<(), StoreError> {
        let fail = |detail: String| StoreError::Persistence { detail };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| fail(format!("creating {}: {err}", parent.display())))?;
        }
        let mut buffer = String::new();
        for entry in delta {
            let line = serde_json::to_string(entry)
                .map_err(|err| fail(format!("encoding a journal entry: {err}")))?;
            buffer.push_str(&line);
            buffer.push('\n');
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|err| fail(format!("opening {}: {err}", self.path.display())))?;
        file.write_all(buffer.as_bytes())
            .map_err(|err| fail(format!("writing {}: {err}", self.path.display())))?;
        // The whole point of this type: the caller is told only after the
        // bytes are on the device, not merely in the page cache.
        file.sync_all()
            .map_err(|err| fail(format!("syncing {}: {err}", self.path.display())))?;
        Ok(())
    }
}

/// Catches a restored ledger up to the journal.
///
/// Returns how many entries were replayed. Entries the snapshot already
/// contains are skipped by sequence, so recovery is idempotent and a healthy
/// start replays nothing.
pub(crate) fn recover(
    ledger: &mut TaskLedger,
    sink: &FileJournalSink,
) -> Result<usize, StoreError> {
    let durable = sink.read_all()?;
    let restored_through = ledger.last_sequence();
    let mut replayed = 0;
    for entry in durable {
        if entry.sequence() <= restored_through {
            continue;
        }
        ledger
            .absorb(&entry)
            .map_err(|err| StoreError::Persistence {
                detail: format!("replaying journal entry: {}", err.message()),
            })?;
        replayed += 1;
    }
    Ok(replayed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::company::tasks::{
        commit_mutation, CommandContext, CommittedOutcome, TaskActor, TaskMutation, TaskState,
    };

    fn temp_journal(name: &str) -> FileJournalSink {
        let unique = format!(
            "vrspi-journal-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        );
        FileJournalSink::new(std::env::temp_dir().join(unique).join(JOURNAL_FILE_NAME))
    }

    fn ctx(key: &str, expected: Option<u64>, now: u64) -> CommandContext {
        CommandContext {
            actor: TaskActor::Host,
            idempotency_key: key.to_string(),
            expected_revision: expected,
            caused_by: None,
            now_unix_ms: now,
        }
    }

    fn create() -> TaskMutation {
        TaskMutation::Create {
            room_id: "room_3".into(),
            root_id: "evt_41".into(),
            title: "Durable store".into(),
            owner_member_id: "seat_11".into(),
            depends_on: Vec::new(),
            artifact_id: None,
        }
    }

    #[test]
    fn a_committed_command_survives_a_snapshot_that_was_never_saved() {
        // The crash this exists for: the delta is durable and the worker has
        // been told, but the process dies before the snapshot is written.
        let sink = temp_journal("recovery");
        let mut live = TaskLedger::default();
        let mut writer = sink.clone();
        let outcome =
            commit_mutation(&mut live, &mut writer, ctx("c1", None, 10), create()).expect("commit");
        let task_id = match outcome {
            CommittedOutcome::Applied(result) => result.events[0].task_id.clone(),
            CommittedOutcome::Rejected(_) => panic!("expected acceptance"),
        };
        commit_mutation(
            &mut live,
            &mut writer,
            ctx("c2", Some(1), 20),
            TaskMutation::Claim {
                task_id: task_id.clone(),
                instance_id: "agent_1".into(),
                lease_ms: 1_000,
            },
        )
        .expect("commit claim");

        // A snapshot that only captured the first command.
        let mut restored = TaskLedger::default();
        restored
            .absorb(&sink.read_all().expect("read")[0])
            .expect("absorb the saved part");
        assert_eq!(restored.last_sequence(), 1);

        let replayed = recover(&mut restored, &sink).expect("recover");
        assert_eq!(replayed, 1, "only what the snapshot missed is replayed");
        assert_eq!(restored, live, "recovery rebuilds exactly what was told");
        assert_eq!(
            restored.task(&task_id).expect("task").state,
            TaskState::Leased
        );

        // Recovery is idempotent: a healthy start replays nothing.
        assert_eq!(recover(&mut restored, &sink).expect("recover"), 0);
        assert_eq!(restored, live);

        let _ = std::fs::remove_dir_all(sink.path().parent().expect("dir"));
    }

    #[test]
    fn a_torn_final_line_costs_only_that_record() {
        let sink = temp_journal("torn");
        let mut live = TaskLedger::default();
        let mut writer = sink.clone();
        commit_mutation(&mut live, &mut writer, ctx("c1", None, 10), create()).expect("commit");
        let intact = std::fs::read_to_string(sink.path()).expect("read");

        // Simulate a write interrupted mid-line.
        std::fs::write(sink.path(), format!("{intact}{{\"entry\":\"eve")).expect("write torn tail");
        let entries = sink.read_all().expect("read");
        assert_eq!(entries.len(), 1, "the torn tail is dropped, not the file");

        let mut restored = TaskLedger::default();
        assert_eq!(recover(&mut restored, &sink).expect("recover"), 1);
        assert_eq!(restored, live);

        let _ = std::fs::remove_dir_all(sink.path().parent().expect("dir"));
    }

    #[test]
    fn an_unwritable_journal_refuses_the_command_and_publishes_nothing() {
        // A directory where the file should be: opening it for append fails.
        let sink = temp_journal("unwritable");
        std::fs::create_dir_all(sink.path()).expect("create the blocking dir");
        let mut live = TaskLedger::default();
        let baseline = live.clone();
        let mut writer = sink.clone();

        let error = commit_mutation(&mut live, &mut writer, ctx("c1", None, 10), create())
            .expect_err("storage refused");
        assert_eq!(error.code(), "store_persistence_failed");
        assert_eq!(live, baseline, "nothing was published");

        let _ = std::fs::remove_dir_all(sink.path());
    }
}
