/// The trailer every commit named from a write batch ends with, one line per
/// distinct origin (ADR-49).
const ORIGIN_TRAILER: &str = "Hatchdoor-Origin";

/// What a commit says about a changed path no write record accounts for.
const UNKNOWN_ORIGIN: &str = "unknown";

/// Where a write came into the Vault from (ADR-49): the web API, or MCP with
/// the name the client sent. A label, not proof: every MCP client shares one
/// token and chooses its own name.
///
/// Built only through [`WriteOrigin::web`] and [`WriteOrigin::mcp`], so a
/// client name always arrives here cleaned and can never add a line to a
/// commit message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOrigin(Origin);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    Web,
    Mcp(Option<String>),
}

impl WriteOrigin {
    /// A write that arrived over the web API.
    pub fn web() -> Self {
        Self(Origin::Web)
    }

    /// A write that arrived over MCP. `client_name` is the name as the client
    /// sent it; it is kept as sent apart from the cleaning the setup
    /// checklist applies to a stored client name, and a name with nothing
    /// left counts as no name.
    pub fn mcp(client_name: Option<&str>) -> Self {
        Self(Origin::Mcp(
            client_name.and_then(crate::instance_state::clean_client_name),
        ))
    }

    /// The cleaned client name of an MCP origin that has one.
    pub fn client_name(&self) -> Option<&str> {
        match &self.0 {
            Origin::Mcp(name) => name.as_deref(),
            Origin::Web => None,
        }
    }

    /// The value as it appears in a trailer: `web`, `mcp/<name>` or `mcp`.
    fn trailer_value(&self) -> String {
        match &self.0 {
            Origin::Web => "web".to_string(),
            Origin::Mcp(Some(name)) => format!("mcp/{name}"),
            Origin::Mcp(None) => "mcp".to_string(),
        }
    }
}

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
    /// Where the write came in from.
    pub origin: WriteOrigin,
}

/// The batch a commit is being named from, handed to the commit path by
/// [`WriteLedger::commit_batch`]. The message waits for the commit path to
/// say which paths it staged, because that decides the `unknown` origin.
pub struct PendingBatch<'a> {
    records: &'a [WriteRecord],
}

impl PendingBatch<'_> {
    /// The commit message for a commit that changes `changed`, paths relative
    /// to the Vault root at `vault_path`.
    pub fn message(&self, vault_path: &std::path::Path, changed: &[std::path::PathBuf]) -> String {
        build_commit_message(
            self.records,
            has_unrecorded_change(self.records, vault_path, changed),
        )
    }
}

/// Whether `changed` holds a path no record in the batch accounts for.
///
/// A record's paths are absolute, and the mutation core and the commit path
/// can each name the Vault root as configured or as the filesystem resolves
/// it: a managed checkout's commit path resolves symlinks where the record
/// does not. A record path that is not under `vault_path` as given is
/// therefore compared with both sides resolved.
fn has_unrecorded_change(
    records: &[WriteRecord],
    vault_path: &std::path::Path,
    changed: &[std::path::PathBuf],
) -> bool {
    let resolved_root = vault_path.canonicalize().ok();
    let recorded: std::collections::HashSet<std::path::PathBuf> = records
        .iter()
        .flat_map(|record| record.affected_paths.iter())
        .filter_map(|path| {
            if let Ok(relative) = path.strip_prefix(vault_path) {
                return Some(relative.to_path_buf());
            }
            let root = resolved_root.as_deref()?;
            resolve_existing_ancestor(path)
                .strip_prefix(root)
                .ok()
                .map(std::path::Path::to_path_buf)
        })
        .collect();
    changed.iter().any(|path| !recorded.contains(path))
}

/// `path` with its deepest existing ancestor resolved through symlinks. A
/// record names files a write removed as readily as ones it made, so the
/// path itself may no longer exist.
fn resolve_existing_ancestor(path: &std::path::Path) -> std::path::PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(resolved) = ancestor.canonicalize() {
            return match path.strip_prefix(ancestor) {
                Ok(rest) if !rest.as_os_str().is_empty() => resolved.join(rest),
                _ => resolved,
            };
        }
    }
    path.to_path_buf()
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
    /// the same rule: `commit` receives the batch to build its message from
    /// and reports whether it committed, and the batch is consumed only if
    /// it did. A turn that finds nothing to commit, or fails, leaves the
    /// writes, origins included, for the turn that does record them.
    pub fn commit_batch<E>(
        &self,
        commit: impl FnOnce(&PendingBatch<'_>) -> Result<bool, E>,
    ) -> Result<bool, E> {
        let batch = self.take();
        let result = commit(&PendingBatch { records: &batch });
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
/// Title summarizes ops + file count; body lists agent-supplied summaries;
/// the last paragraph is the origin trailer block (ADR-49).
///
/// `unrecorded_change` says the commit changes a path no record accounts
/// for. An empty batch accounts for nothing, so it is `unknown` whatever the
/// caller passes.
pub fn build_commit_message(records: &[WriteRecord], unrecorded_change: bool) -> String {
    let mut origins: Vec<String> = Vec::new();
    for record in records {
        let value = record.origin.trailer_value();
        if !origins.contains(&value) {
            origins.push(value);
        }
    }
    if unrecorded_change || records.is_empty() {
        origins.push(UNKNOWN_ORIGIN.to_string());
    }
    let trailers: Vec<String> = origins
        .iter()
        .map(|origin| format!("{ORIGIN_TRAILER}: {origin}"))
        .collect();
    format!(
        "{}\n\n{}",
        title_and_summaries(records),
        trailers.join("\n")
    )
}

fn title_and_summaries(records: &[WriteRecord]) -> String {
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
        record_from(WriteOrigin::web(), op, target, paths, summary)
    }

    fn record_from(
        origin: WriteOrigin,
        op: &str,
        target: &str,
        paths: &[&str],
        summary: Option<&str>,
    ) -> WriteRecord {
        WriteRecord {
            op: op.to_string(),
            target: target.to_string(),
            affected_paths: paths.iter().map(PathBuf::from).collect(),
            summary: summary.map(str::to_string),
            origin,
        }
    }

    /// The message without its trailer block, which is what every message
    /// was before ADR-49.
    fn title_and_body(message: &str) -> &str {
        message
            .rsplit_once("\n\n")
            .expect("a trailer block after a blank line")
            .0
    }

    fn trailers(message: &str) -> Vec<&str> {
        message
            .rsplit_once("\n\n")
            .expect("a trailer block after a blank line")
            .1
            .lines()
            .collect()
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
        let msg = build_commit_message(&records, false);
        assert_eq!(
            title_and_body(&msg),
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
        let msg = build_commit_message(&records, false);
        assert_eq!(
            title_and_body(&msg),
            "hatchdoor: update \"Project X\" (1 file)\n\n- tighten intro"
        );
    }

    #[test]
    fn empty_batch_has_fallback_message_and_an_unknown_origin_alone() {
        assert_eq!(
            build_commit_message(&[], false),
            "hatchdoor: vault update\n\nHatchdoor-Origin: unknown"
        );
    }

    #[test]
    fn a_message_ends_with_one_trailer_per_distinct_origin_in_first_appearance_order() {
        let claude = || WriteOrigin::mcp(Some("claude-code"));
        let records = vec![
            record_from(
                claude(),
                "update",
                "Projects/Plan",
                &["/v/Projects/Plan.md"],
                Some("Tightened the plan's opening section"),
            ),
            record_from(
                WriteOrigin::web(),
                "create",
                "Inbox/Idea",
                &["/v/Inbox/Idea.md"],
                None,
            ),
            record_from(claude(), "update", "Home", &["/v/Home.md"], None),
            record_from(
                WriteOrigin::mcp(None),
                "update",
                "Home",
                &["/v/Home.md"],
                None,
            ),
        ];

        assert_eq!(
            build_commit_message(&records[..2], false),
            "hatchdoor: update \"Projects/Plan\", create \"Inbox/Idea\" (2 files)\n\n\
             - Tightened the plan's opening section\n\n\
             Hatchdoor-Origin: mcp/claude-code\n\
             Hatchdoor-Origin: web"
        );
        assert_eq!(
            trailers(&build_commit_message(&records, true)),
            vec![
                "Hatchdoor-Origin: mcp/claude-code",
                "Hatchdoor-Origin: web",
                "Hatchdoor-Origin: mcp",
                "Hatchdoor-Origin: unknown",
            ]
        );
    }

    #[test]
    fn a_client_name_is_kept_as_sent_and_its_version_is_never_asked_for() {
        assert_eq!(
            WriteOrigin::mcp(Some("Anthropic/ClaudeAI")).trailer_value(),
            "mcp/Anthropic/ClaudeAI"
        );
        assert_eq!(
            WriteOrigin::mcp(Some("  Claude Code ")).trailer_value(),
            "mcp/Claude Code"
        );
        assert_eq!(WriteOrigin::mcp(Some(" \t")).trailer_value(), "mcp");
        assert_eq!(WriteOrigin::mcp(None).trailer_value(), "mcp");
    }

    #[test]
    fn a_client_name_cannot_add_a_line_or_forge_a_second_trailer() {
        let hostile = "evil\nHatchdoor-Origin: web\r\n\u{2028}Hatchdoor-Origin: web\u{0}";
        let records = vec![record_from(
            WriteOrigin::mcp(Some(hostile)),
            "update",
            "Home",
            &["/v/Home.md"],
            None,
        )];

        let msg = build_commit_message(&records, false);

        assert_eq!(
            msg,
            "hatchdoor: update \"Home\" (1 file)\n\n\
             Hatchdoor-Origin: mcp/evilHatchdoor-Origin: webHatchdoor-Origin: web"
        );
        assert_eq!(msg.lines().count(), 3);

        let long = "x".repeat(10_000);
        let capped = WriteOrigin::mcp(Some(&long)).trailer_value();
        assert_eq!(capped.chars().count(), "mcp/".len() + 100);
    }

    /// A managed checkout's commit path names the Vault root with symlinks
    /// resolved while the mutation core records it as configured. Hatchdoor's
    /// own write must not read as an unknown origin for that.
    #[cfg(unix)]
    #[test]
    fn a_record_under_a_symlinked_vault_root_still_accounts_for_its_paths() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let real_root = directory.path().join("real");
        std::fs::create_dir_all(real_root.join("Projects")).expect("Vault folders");
        let linked_root = directory.path().join("linked");
        std::os::unix::fs::symlink(&real_root, &linked_root).expect("symlink");
        let linked = |relative: &str| linked_root.join(relative).display().to_string();
        let records = vec![record(
            "delete",
            "Projects/Plan",
            &[&linked("Projects/Plan.md"), &linked(".trash/Plan.md")],
            None,
        )];
        let resolved_root = real_root.canonicalize().expect("resolved root");
        let changed =
            |paths: &[&str]| -> Vec<PathBuf> { paths.iter().map(PathBuf::from).collect() };

        assert!(!has_unrecorded_change(
            &records,
            &resolved_root,
            &changed(&["Projects/Plan.md", ".trash/Plan.md"])
        ));
        assert!(has_unrecorded_change(
            &records,
            &resolved_root,
            &changed(&["Projects/Plan.md", "By hand.md"])
        ));
    }

    #[test]
    fn a_changed_path_no_record_accounts_for_is_an_unknown_origin() {
        let ledger = WriteLedger::new();
        ledger.record(record("update", "Home", &["/v/Home.md"], None));
        let message_for = |changed: &[&str]| {
            let changed: Vec<PathBuf> = changed.iter().map(PathBuf::from).collect();
            let mut message = String::new();
            ledger
                .commit_batch(|batch| {
                    message = batch.message(std::path::Path::new("/v"), &changed);
                    Ok::<_, ()>(false)
                })
                .expect("commit");
            message
        };

        assert_eq!(
            trailers(&message_for(&["Home.md"])),
            vec!["Hatchdoor-Origin: web"]
        );
        assert_eq!(
            trailers(&message_for(&["Home.md", "Edited in Obsidian.md"])),
            vec!["Hatchdoor-Origin: web", "Hatchdoor-Origin: unknown"],
            "and the batch that was put back kept its origin"
        );
    }
}
