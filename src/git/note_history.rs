//! When each note first entered a Git-backed Vault's history (#300, ADR-29).
//!
//! The Stats page charts notes by created date, and for a Git-backed Vault
//! the repository already knows it: the commit that first added the note,
//! carried forward through every rename since. Modification time cannot stand
//! in for it, because git stores none and a checkout stamps every file with
//! the moment it ran.
//!
//! The walk covers the whole repository rather than one Vault's subfolder, so
//! a note moved into the Vault from elsewhere keeps its date. It runs once per
//! Vault on a background thread, is held in memory with the commit it
//! describes, and when the branch moves forward only the new commits are
//! walked. Nothing here runs per note or per request.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use git2::{Delta, DiffFindOptions, DiffOptions, Oid, Repository, Sort, TreeWalkMode};

/// Every note path in a repository's history at one commit, mapped to when it
/// first appeared.
#[derive(Clone, Debug, Default)]
pub struct FirstAdds {
    /// `None` for an unborn branch, where nothing has been committed yet.
    head: Option<Oid>,
    /// Repository-relative, `/`-separated, and kept for notes deleted since,
    /// so a merge that keeps a note one branch deleted still knows its date.
    dates: HashMap<String, NoteEntry>,
    /// Whether the history was shallow when walked. Such a walk is never
    /// extended: once the missing history arrives, only a fresh walk can
    /// date the notes it had to leave undated.
    grafted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NoteEntry {
    /// `None` marks a note a shallow clone's graft point introduced: it
    /// predates the history on hand, so its real first commit is unknown.
    date: Option<i64>,
    /// Whether the note exists at the walked commit.
    live: bool,
}

/// What history says about one note path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstAdd {
    /// Nanoseconds since the Unix epoch: the author date of the commit that
    /// first added the note, or the note it was renamed from.
    Known(i64),
    /// The note arrived with a shallow clone's graft point, so the history
    /// that would date it is not in the repository.
    Unknown,
    /// No commit has the note yet, so it has no history to consult.
    Uncommitted,
}

impl FirstAdds {
    /// A walk's result stated directly, for a reader's tests.
    #[cfg(test)]
    pub(crate) fn from_dates<'a>(dates: impl IntoIterator<Item = (&'a str, Option<i64>)>) -> Self {
        Self {
            head: None,
            dates: dates
                .into_iter()
                .map(|(path, date)| (path.to_string(), NoteEntry { date, live: true }))
                .collect(),
            grafted: false,
        }
    }

    pub fn lookup(&self, repository_path: &str) -> FirstAdd {
        match self.dates.get(repository_path) {
            Some(NoteEntry {
                date: Some(nanos),
                live: true,
            }) => FirstAdd::Known(*nanos),
            Some(NoteEntry {
                date: None,
                live: true,
            }) => FirstAdd::Unknown,
            _ => FirstAdd::Uncommitted,
        }
    }
}

/// The answer to one read of a Vault's history.
#[derive(Clone, Debug)]
pub enum HistoryRead {
    Ready(Arc<FirstAdds>),
    /// The walk for the current commit is still running.
    Reading,
    /// The repository could not be opened or walked.
    Unavailable,
}

/// One Vault's cached history walk. Lives on the Vault's control block, so a
/// Vault whose definition changes starts again from nothing.
#[derive(Default)]
pub struct NoteHistory {
    state: Mutex<HistoryState>,
    settled: Condvar,
}

#[derive(Default)]
struct HistoryState {
    latest: Option<Arc<FirstAdds>>,
    /// The commit a background walk is currently dating.
    walking: Option<Oid>,
    /// The commit the last walk failed on, so a broken repository is not
    /// walked again on every read until its branch moves.
    failed: Option<Oid>,
}

impl NoteHistory {
    /// Created dates for the repository at `repository_root` as of its
    /// current `HEAD`. Starts a background walk when the cached one describes
    /// another commit, and waits up to `wait` for it before answering
    /// [`HistoryRead::Reading`]. A later read picks up the finished walk.
    pub fn read(self: &Arc<Self>, repository_root: &Path, wait: Duration) -> HistoryRead {
        let head = match current_head(repository_root) {
            Ok(Some(head)) => head,
            Ok(None) => return HistoryRead::Ready(Arc::new(FirstAdds::default())),
            Err(_) => return HistoryRead::Unavailable,
        };
        let deadline = Instant::now() + wait;
        let mut state = self.state.lock().expect("note history poisoned");
        loop {
            if let Some(latest) = state
                .latest
                .as_ref()
                .filter(|latest| latest.head == Some(head))
            {
                return HistoryRead::Ready(Arc::clone(latest));
            }
            if state.failed == Some(head) {
                return HistoryRead::Unavailable;
            }
            if state.walking.is_none() {
                state.walking = Some(head);
                self.spawn_walk(repository_root.to_path_buf(), head, state.latest.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return HistoryRead::Reading;
            }
            state = self
                .settled
                .wait_timeout(state, deadline - now)
                .expect("note history poisoned")
                .0;
        }
    }

    fn spawn_walk(self: &Arc<Self>, root: PathBuf, head: Oid, base: Option<Arc<FirstAdds>>) {
        let history = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("note-history".to_string())
            .spawn(move || {
                let walked = Repository::open(&root)
                    .and_then(|repository| walk(&repository, head, base.as_deref()));
                let mut state = history.state.lock().expect("note history poisoned");
                state.walking = None;
                match walked {
                    Ok(first_adds) => {
                        state.latest = Some(Arc::new(first_adds));
                        state.failed = None;
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "reading a Vault's git history for created dates failed"
                        );
                        state.failed = Some(head);
                    }
                }
                history.settled.notify_all();
            });
        if spawned.is_err() {
            let mut state = self.state.lock().expect("note history poisoned");
            state.walking = None;
            state.failed = Some(head);
        }
    }
}

fn current_head(repository_root: &Path) -> Result<Option<Oid>, git2::Error> {
    let repository = Repository::open(repository_root)?;
    match repository.head() {
        Ok(head) => Ok(head.target()),
        Err(error)
            if matches!(
                error.code(),
                git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Date every note in `head`'s history. With a `base` that `head` descends
/// from, only the commits since it are walked and applied on top of it;
/// otherwise, after a reset, a rewritten branch, or a shallow base, the whole
/// history is.
fn walk(
    repository: &Repository,
    head: Oid,
    base: Option<&FirstAdds>,
) -> Result<FirstAdds, git2::Error> {
    let base = match base.and_then(|base| base.head.map(|oid| (base, oid))) {
        Some((base, base_head))
            if !base.grafted
                && (base_head == head || repository.graph_descendant_of(head, base_head)?) =>
        {
            Some((base, base_head))
        }
        _ => None,
    };
    let mut dates = base.map(|(base, _)| base.dates.clone()).unwrap_or_default();

    let grafts = shallow_grafts(repository);
    let mut revwalk = repository.revwalk()?;
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::REVERSE)?;
    revwalk.push(head)?;
    if let Some((_, base_head)) = base {
        revwalk.hide(base_head)?;
    }

    for oid in revwalk {
        let commit = repository.find_commit(oid?)?;
        let tree = commit.tree()?;
        if grafts.contains(&commit.id()) {
            // A shallow clone's oldest commit lists every note as new, but
            // they are only new to this clone. Record them as undatable so a
            // later edit does not pass for their creation either.
            for path in note_paths(&tree)? {
                dates.entry(path).or_insert(NoteEntry {
                    date: None,
                    live: true,
                });
            }
            continue;
        }

        let when = author_nanos(&commit);
        let merge = commit.parent_count() > 1;
        // A merge is read against its first parent. The commits it brings in
        // were walked on their own branch first, so a note arriving through
        // the merge already has the date its own branch gave it.
        let parent_tree = match commit.parent_count() {
            0 => None,
            _ => Some(commit.parent(0)?.tree()?),
        };
        let mut options = DiffOptions::new();
        options.pathspec("*.md");
        let mut diff =
            repository.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut options))?;
        let mut find = DiffFindOptions::new();
        // A bulk rename can touch more notes in one commit than libgit2's
        // default limit of 1000, beyond which it stops pairing renames.
        find.renames(true).rename_limit(100_000);
        diff.find_similar(Some(&mut find))?;

        for delta in diff.deltas() {
            let old = delta.old_file().path().and_then(path_key);
            let new = delta.new_file().path().and_then(path_key);
            match delta.status() {
                Delta::Added => {
                    if let Some(new) = new.filter(|path| is_note(path)) {
                        arrive(&mut dates, new, None, when, merge);
                    }
                }
                Delta::Deleted => {
                    if let Some(entry) = old.and_then(|old| dates.get_mut(&old)) {
                        entry.live = false;
                    }
                }
                Delta::Renamed => {
                    let carried = old.and_then(|old| dates.get_mut(&old)).map(|entry| {
                        entry.live = false;
                        entry.date
                    });
                    if let Some(new) = new.filter(|path| is_note(path)) {
                        arrive(&mut dates, new, carried, when, merge);
                    }
                }
                _ => {}
            }
        }
    }

    // What the diffs say about a note can disagree with the tree when a
    // merge resolves a delete on one branch against a keep on the other: the
    // merge's diff against its first parent shows nothing, but the note was
    // marked deleted on the other branch. The tree at `head` settles which
    // notes exist.
    let present = note_paths(&repository.find_commit(head)?.tree()?)?;
    for (path, entry) in dates.iter_mut() {
        entry.live = present.contains(path);
    }
    for path in present {
        // A note the diffs never saw arrive is undatable rather than new.
        dates.entry(path).or_insert(NoteEntry {
            date: None,
            live: true,
        });
    }

    Ok(FirstAdds {
        head: Some(head),
        dates,
        grafted: !grafts.is_empty(),
    })
}

/// Record a note arriving at `path`, from a rename when `carried` holds the
/// date it had before. A note that is already there keeps its date. One that
/// was deleted and comes back in an ordinary commit was written again, so it
/// is new. One that comes back through a merge never left the branch the merge
/// brought in, so it keeps the date it had.
fn arrive(
    dates: &mut HashMap<String, NoteEntry>,
    path: String,
    carried: Option<Option<i64>>,
    when: i64,
    merge: bool,
) {
    let arriving = NoteEntry {
        date: carried.unwrap_or(Some(when)),
        live: true,
    };
    let entry = dates.entry(path).or_insert(arriving);
    if !entry.live {
        *entry = if merge {
            NoteEntry {
                live: true,
                ..*entry
            }
        } else {
            arriving
        };
    }
}

/// Every note path in `tree`, repository-relative.
fn note_paths(tree: &git2::Tree<'_>) -> Result<HashSet<String>, git2::Error> {
    let mut paths = HashSet::new();
    tree.walk(TreeWalkMode::PreOrder, |directory, entry| {
        if let Ok(name) = entry.name()
            && is_note(name)
            && entry.kind() == Some(git2::ObjectType::Blob)
        {
            paths.insert(format!("{directory}{name}"));
        }
        git2::TreeWalkResult::Ok
    })?;
    Ok(paths)
}

/// The commits a shallow clone was cut at, whose parents are missing. Empty
/// for a complete repository.
fn shallow_grafts(repository: &Repository) -> HashSet<Oid> {
    if !repository.is_shallow() {
        return HashSet::new();
    }
    std::fs::read_to_string(repository.path().join("shallow"))
        .map(|text| {
            text.lines()
                .filter_map(|line| Oid::from_str(line.trim()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn author_nanos(commit: &git2::Commit<'_>) -> i64 {
    commit
        .author()
        .when()
        .seconds()
        .saturating_mul(1_000_000_000)
}

fn path_key(path: &Path) -> Option<String> {
    path.to_str().map(|path| path.replace('\\', "/"))
}

fn is_note(path: &str) -> bool {
    Path::new(path).extension().and_then(|ext| ext.to_str()) == Some("md")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    /// 2026-02-01T12:00:00Z.
    const FEB: i64 = 1_769_947_200;
    /// 2026-05-01T12:00:00Z.
    const MAY: i64 = 1_777_636_800;
    /// 2026-08-01T12:00:00Z.
    const AUG: i64 = 1_785_585_600;

    fn nanos(seconds: i64) -> FirstAdd {
        FirstAdd::Known(seconds * 1_000_000_000)
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        repository: Repository,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = dir.path().to_path_buf();
            let repository = Repository::init(&root).expect("init");
            Self {
                _dir: dir,
                root,
                repository,
            }
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.root.join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, text).expect("write");
        }

        fn remove(&self, path: &str) {
            std::fs::remove_file(self.root.join(path)).expect("remove");
        }

        fn rename(&self, from: &str, to: &str) {
            let to_path = self.root.join(to);
            std::fs::create_dir_all(to_path.parent().expect("parent")).expect("mkdir");
            std::fs::rename(self.root.join(from), to_path).expect("rename");
        }

        /// Commit the whole working tree, authored at `seconds` but committed
        /// a day later, so a test can tell which of the two dates was read.
        fn commit(&self, seconds: i64) -> Oid {
            let mut index = self.repository.index().expect("index");
            index
                .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .expect("add");
            index.update_all(["*"], None).expect("stage removals");
            index.write().expect("write index");
            let tree = self
                .repository
                .find_tree(index.write_tree().expect("tree"))
                .expect("find tree");
            let author = git2::Signature::new("A", "a@example.com", &git2::Time::new(seconds, 0))
                .expect("author");
            let committer =
                git2::Signature::new("C", "c@example.com", &git2::Time::new(seconds + DAY, 0))
                    .expect("committer");
            let parents: Vec<git2::Commit<'_>> = self
                .repository
                .head()
                .ok()
                .and_then(|head| head.peel_to_commit().ok())
                .into_iter()
                .collect();
            let parents: Vec<&git2::Commit<'_>> = parents.iter().collect();
            self.repository
                .commit(Some("HEAD"), &author, &committer, "change", &tree, &parents)
                .expect("commit")
        }

        fn first_adds(&self) -> FirstAdds {
            let head = self.repository.head().expect("head").target().expect("oid");
            walk(&self.repository, head, None).expect("walk")
        }
    }

    /// A rename carries the note's first date forward, and the date read is
    /// the author's, not the committer's.
    #[test]
    fn a_renamed_note_keeps_the_date_it_was_first_added() {
        let repo = Fixture::new();
        repo.write("Vault — Rules.md", "# Rules\n\nKeep it plain.\n");
        repo.commit(FEB);
        repo.rename("Vault — Rules.md", "Vault - Rules.md");
        repo.commit(AUG);

        let dates = repo.first_adds();

        assert_eq!(dates.lookup("Vault - Rules.md"), nanos(FEB));
        assert_eq!(dates.lookup("Vault — Rules.md"), FirstAdd::Uncommitted);
    }

    /// Light edits in the same commit as the rename still pair the two paths.
    #[test]
    fn a_rename_with_a_light_edit_keeps_its_date() {
        let repo = Fixture::new();
        let body = "line\n".repeat(40);
        repo.write("draft.md", &format!("# Draft\n{body}"));
        repo.commit(FEB);
        repo.remove("draft.md");
        repo.write("notes/final.md", &format!("# Final\n{body}"));
        repo.commit(AUG);

        assert_eq!(repo.first_adds().lookup("notes/final.md"), nanos(FEB));
    }

    #[test]
    fn a_note_deleted_and_recreated_is_new_from_the_recreation() {
        let repo = Fixture::new();
        repo.write("Groceries.md", "milk\n");
        repo.commit(FEB);
        repo.remove("Groceries.md");
        repo.write("keep.md", "x\n");
        repo.commit(MAY);
        repo.write("Groceries.md", "bread and eggs and a whole new list\n");
        repo.commit(AUG);

        assert_eq!(repo.first_adds().lookup("Groceries.md"), nanos(AUG));
    }

    #[test]
    fn a_copied_note_is_new_from_the_copy() {
        let repo = Fixture::new();
        let template = "# Template\n\n- [ ] one\n- [ ] two\n";
        repo.write("Template.md", template);
        repo.commit(FEB);
        repo.write("Trip.md", template);
        repo.commit(AUG);

        let dates = repo.first_adds();

        assert_eq!(dates.lookup("Template.md"), nanos(FEB));
        assert_eq!(dates.lookup("Trip.md"), nanos(AUG));
    }

    /// The walk spans the whole repository, so a note moved into a Vault that
    /// is one subfolder of it keeps the date it had outside.
    #[test]
    fn a_note_moved_into_a_vault_subdirectory_keeps_its_date() {
        let repo = Fixture::new();
        repo.write("inbox/idea.md", "# Idea\n\nSomething worth keeping.\n");
        repo.commit(FEB);
        repo.rename("inbox/idea.md", "vault/idea.md");
        repo.commit(AUG);

        assert_eq!(repo.first_adds().lookup("vault/idea.md"), nanos(FEB));
    }

    /// A bulk rename like the em dash cleanup in #299, sized past libgit2's
    /// default limit of 1000 renames per diff, beyond which it silently
    /// reports deletions and additions instead.
    #[test]
    fn a_bulk_rename_keeps_every_date() {
        let repo = Fixture::new();
        for index in 0..1200 {
            repo.write(
                &format!("Note — {index}.md"),
                &format!(
                    "# Note {index}\n\nBody of note number {index}, long enough to compare.\n"
                ),
            );
        }
        repo.commit(FEB);
        for index in 0..1200 {
            repo.rename(&format!("Note — {index}.md"), &format!("Note - {index}.md"));
        }
        repo.commit(AUG);

        let dates = repo.first_adds();

        for index in 0..1200 {
            assert_eq!(dates.lookup(&format!("Note - {index}.md")), nanos(FEB));
        }
    }

    #[test]
    fn only_markdown_notes_are_dated() {
        let repo = Fixture::new();
        repo.write("a/b/deep.md", "deep\n");
        repo.write("image.png", "not a note\n");
        repo.commit(FEB);

        let dates = repo.first_adds();

        assert_eq!(dates.lookup("a/b/deep.md"), nanos(FEB));
        assert_eq!(dates.lookup("image.png"), FirstAdd::Uncommitted);
    }

    /// A note arriving through a merge keeps the date its own branch gave it.
    #[test]
    fn a_note_merged_from_a_branch_keeps_its_branch_date() {
        let repo = Fixture::new();
        repo.write("main.md", "main\n");
        let root = repo.commit(FEB);
        repo.write("side.md", "side\n");
        let side = repo.commit(MAY);

        // Rewind the branch to the root and commit alongside the side branch.
        let root_commit = repo.repository.find_commit(root).expect("root");
        repo.repository
            .reset(root_commit.as_object(), git2::ResetType::Hard, None)
            .expect("reset");
        repo.write("other.md", "other\n");
        let main = repo.commit(MAY + DAY);

        let side_commit = repo.repository.find_commit(side).expect("side");
        let main_commit = repo.repository.find_commit(main).expect("main");
        let mut merged = repo
            .repository
            .merge_commits(&main_commit, &side_commit, None)
            .expect("merge");
        let tree = repo
            .repository
            .find_tree(merged.write_tree_to(&repo.repository).expect("tree"))
            .expect("find tree");
        let signature =
            git2::Signature::new("A", "a@example.com", &git2::Time::new(AUG, 0)).expect("sig");
        repo.repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "merge",
                &tree,
                &[&main_commit, &side_commit],
            )
            .expect("merge commit");

        let dates = repo.first_adds();

        assert_eq!(dates.lookup("side.md"), nanos(MAY));
        assert_eq!(dates.lookup("other.md"), nanos(MAY + DAY));
        assert_eq!(dates.lookup("main.md"), nanos(FEB));
    }

    /// Merge `side` into the current branch, taking the working tree as the
    /// merge's result.
    fn merge_keeping_working_tree(repo: &Fixture, side: Oid, seconds: i64) {
        let mut index = repo.repository.index().expect("index");
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .expect("add");
        index.update_all(["*"], None).expect("stage removals");
        index.write().expect("write index");
        let tree = repo
            .repository
            .find_tree(index.write_tree().expect("tree"))
            .expect("find tree");
        let head = repo
            .repository
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("head commit");
        let side = repo.repository.find_commit(side).expect("side");
        let signature =
            git2::Signature::new("A", "a@example.com", &git2::Time::new(seconds, 0)).expect("sig");
        repo.repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "merge",
                &tree,
                &[&head, &side],
            )
            .expect("merge commit");
    }

    fn rewind(repo: &Fixture, to: Oid) {
        let commit = repo.repository.find_commit(to).expect("commit");
        repo.repository
            .reset(commit.as_object(), git2::ResetType::Hard, None)
            .expect("reset");
    }

    /// A side branch deletes a note the main branch keeps, and the merge
    /// keeps it. The deletion was walked, but the note never stopped existing
    /// on the branch that won, so it keeps its first date.
    #[test]
    fn a_note_kept_by_a_merge_over_a_side_branch_delete_keeps_its_date() {
        let repo = Fixture::new();
        repo.write("Keep.md", "keep\n");
        let root = repo.commit(FEB);
        repo.remove("Keep.md");
        repo.write("side.md", "side\n");
        let side = repo.commit(MAY);

        rewind(&repo, root);
        repo.write("main.md", "main\n");
        repo.commit(MAY + DAY);
        repo.write("side.md", "side\n");
        merge_keeping_working_tree(&repo, side, AUG);

        let dates = repo.first_adds();

        assert_eq!(dates.lookup("Keep.md"), nanos(FEB));
        assert_eq!(dates.lookup("side.md"), nanos(MAY));
    }

    /// The main branch deletes a note, the side branch keeps it, and the merge
    /// brings it back. It never left the side branch, so it is not new.
    #[test]
    fn a_note_restored_by_a_merge_keeps_its_date() {
        let repo = Fixture::new();
        repo.write("Keep.md", "keep\n");
        let root = repo.commit(FEB);
        repo.write("side.md", "side\n");
        let side = repo.commit(MAY);

        rewind(&repo, root);
        repo.remove("Keep.md");
        repo.write("main.md", "main\n");
        repo.commit(MAY + DAY);
        repo.write("Keep.md", "keep\n");
        repo.write("side.md", "side\n");
        merge_keeping_working_tree(&repo, side, AUG);

        assert_eq!(repo.first_adds().lookup("Keep.md"), nanos(FEB));
    }

    /// Extending a cached walk by the new commits gives what a full walk
    /// gives, and a rewritten branch falls back to a full walk.
    #[test]
    fn an_incremental_walk_matches_a_full_walk() {
        let repo = Fixture::new();
        repo.write("one.md", "one\n");
        let first = repo.commit(FEB);
        let cached = walk(&repo.repository, first, None).expect("walk");

        repo.rename("one.md", "renamed.md");
        repo.write("two.md", "two\n");
        let second = repo.commit(AUG);

        let extended = walk(&repo.repository, second, Some(&cached)).expect("extend");
        let full = walk(&repo.repository, second, None).expect("full");
        assert_eq!(extended.dates, full.dates);
        assert_eq!(extended.lookup("renamed.md"), nanos(FEB));

        // After a reset, the new head no longer descends from the cached
        // one, so the cache is dropped and the history walked afresh.
        let reset = repo.repository.find_commit(first).expect("first");
        repo.repository
            .reset(reset.as_object(), git2::ResetType::Hard, None)
            .expect("reset");
        repo.write("three.md", "three\n");
        let rewritten = repo.commit(MAY);
        let rewalked = walk(&repo.repository, rewritten, Some(&extended)).expect("rewalk");
        assert_eq!(rewalked.lookup("one.md"), nanos(FEB));
        assert_eq!(rewalked.lookup("three.md"), nanos(MAY));
        assert_eq!(rewalked.lookup("two.md"), FirstAdd::Uncommitted);
    }

    /// A shallow clone knows nothing about its graft point's notes, so they
    /// are undatable rather than all created on the graft commit's day.
    #[test]
    fn a_shallow_clone_does_not_date_notes_to_its_graft_point() {
        let repo = Fixture::new();
        repo.write("old.md", "old\n");
        repo.commit(FEB);
        repo.write("middle.md", "middle\n");
        repo.commit(MAY);
        repo.write("new.md", "new\n");
        repo.commit(AUG);

        let clone_dir = tempfile::tempdir().expect("clone dir");
        let status = std::process::Command::new("git")
            .args(["clone", "--quiet", "--depth", "2"])
            .arg(format!("file://{}", repo.root.display()))
            .arg(clone_dir.path())
            .status()
            .expect("git is installed");
        assert!(status.success(), "shallow clone");

        let clone = Repository::open(clone_dir.path()).expect("open clone");
        assert!(clone.is_shallow());
        let head = clone.head().expect("head").target().expect("oid");
        let dates = walk(&clone, head, None).expect("walk");

        assert_eq!(dates.lookup("old.md"), FirstAdd::Unknown);
        assert_eq!(dates.lookup("middle.md"), FirstAdd::Unknown);
        assert_eq!(dates.lookup("new.md"), nanos(AUG));

        // Fetching the rest of the history lets a later walk date them, even
        // though it starts from the cached, shallow one.
        let status = std::process::Command::new("git")
            .args(["fetch", "--quiet", "--unshallow"])
            .current_dir(clone_dir.path())
            .status()
            .expect("git is installed");
        assert!(status.success(), "unshallow");
        let clone = Repository::open(clone_dir.path()).expect("reopen clone");
        assert!(!clone.is_shallow());
        let rewalked = walk(&clone, head, Some(&dates)).expect("rewalk");

        assert_eq!(rewalked.lookup("old.md"), nanos(FEB));
        assert_eq!(rewalked.lookup("middle.md"), nanos(MAY));
    }

    #[test]
    fn reading_history_answers_ready_and_is_reused_for_the_same_commit() {
        let repo = Fixture::new();
        repo.write("note.md", "note\n");
        repo.commit(FEB);
        let history = Arc::new(NoteHistory::default());

        let HistoryRead::Ready(first) = history.read(&repo.root, Duration::from_secs(30)) else {
            panic!("the walk should finish inside the wait");
        };
        let HistoryRead::Ready(second) = history.read(&repo.root, Duration::ZERO) else {
            panic!("the cached walk should answer at once");
        };

        assert!(Arc::ptr_eq(&first, &second), "one walk per commit");
        assert_eq!(first.lookup("note.md"), nanos(FEB));
        assert_eq!(first.lookup("draft.md"), FirstAdd::Uncommitted);
    }

    #[test]
    fn reading_history_without_waiting_answers_reading_then_ready() {
        let repo = Fixture::new();
        repo.write("note.md", "note\n");
        repo.commit(FEB);
        let history = Arc::new(NoteHistory::default());

        // Nothing is cached yet, and a zero wait never blocks on the walk.
        match history.read(&repo.root, Duration::ZERO) {
            HistoryRead::Reading | HistoryRead::Ready(_) => {}
            HistoryRead::Unavailable => panic!("a healthy repository is readable"),
        }
        assert!(matches!(
            history.read(&repo.root, Duration::from_secs(30)),
            HistoryRead::Ready(_)
        ));
    }

    #[test]
    fn an_unborn_branch_has_no_dates_and_a_missing_repository_is_unavailable() {
        let repo = Fixture::new();
        let history = Arc::new(NoteHistory::default());
        let HistoryRead::Ready(empty) = history.read(&repo.root, Duration::ZERO) else {
            panic!("an unborn branch is simply empty");
        };
        assert_eq!(empty.lookup("note.md"), FirstAdd::Uncommitted);

        let missing = repo.root.join("nowhere");
        assert!(matches!(
            history.read(&missing, Duration::ZERO),
            HistoryRead::Unavailable
        ));
    }
}
