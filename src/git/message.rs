/// One vault write that contributed to a debounced batch.
#[derive(Debug, Clone)]
pub struct WriteRecord {
    /// Short human label of the operation, e.g. "create", "update", "delete".
    pub op: String,
    /// A display name for the primary target, e.g. "Projects/New".
    pub target: String,
    /// Absolute paths the operation created, modified, or removed.
    pub affected_paths: Vec<std::path::PathBuf>,
    /// Optional agent-supplied summary line.
    pub summary: Option<String>,
}

/// The per-Vault batch of writes waiting for their Git turn.
///
/// A write lands on disk long before the Vault's Git turn commits it, and one
/// turn coalesces every write since the last one. This is where the record of
/// each write waits in between: the Vault mutation core appends one entry per
/// successful write ([`WriteLedger::record`]), and the commit that records
/// those writes takes the whole batch ([`WriteLedger::take`]) to build its
/// message. A turn that ends up committing nothing puts the batch back
/// ([`WriteLedger::restore`]) so the next turn still has it.
///
/// A record waits for a real commit rather than for a turn, so a write that
/// produces no Git drift — a path the checkout's own `.gitignore` covers —
/// keeps its line until the next commit that does happen, and lands there.
/// The alternative is dropping the line on a turn that committed nothing,
/// which loses the summaries of every write made between two commits.
///
/// Bounded at [`WriteLedger::CAPACITY`] entries, oldest dropped first. A Vault
/// with no Git turn at all (a `Local` source) never has its ledger taken, so
/// without the bound its entries would accumulate for the life of the process.
/// Over the bound the commit message loses its oldest lines, which is the
/// cheapest thing to lose.
#[derive(Debug, Default)]
pub struct WriteLedger {
    records: std::sync::Mutex<std::collections::VecDeque<WriteRecord>>,
}

impl WriteLedger {
    /// How many pending records one Vault keeps. Far above any real debounce
    /// window: a turn that has not run in this many writes has a problem the
    /// commit message is not going to fix.
    pub const CAPACITY: usize = 200;

    pub fn new() -> Self {
        Self::default()
    }

    /// Append one write to the batch, dropping the oldest entry when the
    /// ledger is already full.
    pub fn record(&self, record: WriteRecord) {
        let mut records = self.lock();
        if records.len() >= Self::CAPACITY {
            records.pop_front();
        }
        records.push_back(record);
    }

    /// Name a commit from the pending batch and make it.
    ///
    /// The one place the batch's lifecycle lives, so both commit paths obey
    /// the same rule: `commit` receives the message built from the batch and
    /// reports whether it committed, and the batch is consumed only if it
    /// did. A turn that finds nothing to commit, or fails, leaves the writes
    /// for the turn that does record them.
    pub fn commit_batch<E>(&self, commit: impl FnOnce(&str) -> Result<bool, E>) -> Result<bool, E> {
        let batch = self.take();
        let result = commit(&build_commit_message(&batch));
        if !matches!(result, Ok(true)) {
            self.restore(batch);
        }
        result
    }

    /// Take the whole pending batch, leaving the ledger empty.
    pub fn take(&self) -> Vec<WriteRecord> {
        self.lock().drain(..).collect()
    }

    /// Put a taken batch back at the front, ahead of anything recorded since.
    /// Used by a turn that took the batch and then did not commit, so those
    /// writes still reach the message of whichever turn does.
    pub fn restore(&self, batch: Vec<WriteRecord>) {
        if batch.is_empty() {
            return;
        }
        let mut records = self.lock();
        for record in batch.into_iter().rev() {
            records.push_front(record);
        }
        while records.len() > Self::CAPACITY {
            records.pop_front();
        }
    }

    /// Recover from a poisoned lock rather than propagating a panic into a
    /// Vault write or a Git turn: this guards a commit message, and a message
    /// is never worth failing a write for.
    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::VecDeque<WriteRecord>> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Build a commit message from a batch of write records.
/// Title summarizes ops + file count; body lists agent-supplied summaries.
pub fn build_commit_message(records: &[WriteRecord]) -> String {
    if records.is_empty() {
        return "hatchdoor: vault update".to_string();
    }

    let file_count: usize = {
        let mut paths: Vec<&std::path::Path> = records
            .iter()
            .flat_map(|r| r.affected_paths.iter().map(|p| p.as_path()))
            .collect();
        paths.sort();
        paths.dedup();
        paths.len()
    };

    let mut highlights: Vec<String> = records
        .iter()
        .take(3)
        .map(|r| format!("{} \"{}\"", r.op, r.target))
        .collect();
    if records.len() > 3 {
        highlights.push(format!("+{} more", records.len() - 3));
    }

    let file_word = if file_count == 1 { "file" } else { "files" };
    let title = format!(
        "hatchdoor: {} ({file_count} {file_word})",
        highlights.join(", ")
    );

    let body: Vec<String> = records
        .iter()
        .filter_map(|r| r.summary.as_ref())
        .map(|s| format!("- {s}"))
        .collect();

    if body.is_empty() {
        title
    } else {
        format!("{title}\n\n{}", body.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn record(op: &str, target: &str, paths: &[&str], summary: Option<&str>) -> WriteRecord {
        WriteRecord {
            op: op.to_string(),
            target: target.to_string(),
            affected_paths: paths.iter().map(PathBuf::from).collect(),
            summary: summary.map(str::to_string),
        }
    }

    #[test]
    fn ledger_hands_a_turn_every_write_recorded_since_the_last_one() {
        let ledger = WriteLedger::new();
        ledger.record(record(
            "create",
            "Meeting",
            &["/v/Meeting.md"],
            Some("first"),
        ));
        ledger.record(record(
            "update",
            "Meeting",
            &["/v/Meeting.md"],
            Some("second"),
        ));

        let batch = ledger.take();

        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].summary.as_deref(), Some("first"));
        assert_eq!(batch[1].summary.as_deref(), Some("second"));
        assert!(
            ledger.take().is_empty(),
            "a taken batch is not handed out twice"
        );
    }

    #[test]
    fn a_restored_batch_stays_ahead_of_writes_recorded_during_the_turn() {
        let ledger = WriteLedger::new();
        ledger.record(record("create", "A", &["/v/A.md"], Some("before")));
        let batch = ledger.take();
        ledger.record(record("create", "B", &["/v/B.md"], Some("during")));

        ledger.restore(batch);

        let summaries: Vec<_> = ledger
            .take()
            .into_iter()
            .filter_map(|entry| entry.summary)
            .collect();
        assert_eq!(summaries, vec!["before".to_string(), "during".to_string()]);
    }

    #[test]
    fn a_vault_that_never_commits_keeps_a_bounded_ledger() {
        let ledger = WriteLedger::new();
        for index in 0..WriteLedger::CAPACITY + 5 {
            ledger.record(record(
                "update",
                "Home",
                &["/v/Home.md"],
                Some(&index.to_string()),
            ));
        }

        let batch = ledger.take();

        assert_eq!(batch.len(), WriteLedger::CAPACITY);
        assert_eq!(
            batch[0].summary.as_deref(),
            Some("5"),
            "oldest entries drop first"
        );
    }

    #[test]
    fn title_summarizes_ops_and_unique_file_count() {
        let records = vec![
            record(
                "update",
                "Project X",
                &["/v/Project X.md", "/v/Other.md"],
                None,
            ),
            record("create", "Meeting", &["/v/Meeting.md"], None),
        ];
        let msg = build_commit_message(&records);
        assert_eq!(
            msg,
            "hatchdoor: update \"Project X\", create \"Meeting\" (3 files)"
        );
    }

    #[test]
    fn body_lists_agent_summaries() {
        let records = vec![record(
            "update",
            "Project X",
            &["/v/Project X.md"],
            Some("tighten intro"),
        )];
        let msg = build_commit_message(&records);
        assert!(msg.starts_with("hatchdoor: update \"Project X\" (1 file)"));
        assert!(msg.ends_with("- tighten intro"));
    }

    #[test]
    fn empty_batch_has_fallback_message() {
        assert_eq!(build_commit_message(&[]), "hatchdoor: vault update");
    }
}
