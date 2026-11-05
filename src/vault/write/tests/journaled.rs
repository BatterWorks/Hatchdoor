//! The intent record of a multi-note write, and the repair that finishes one
//! a dead process left behind (#631, ADR-51).
//!
//! A crash is simulated by making the journal panic at a step boundary: the
//! panic unwinds past every rollback, so the Vault and the record are left
//! exactly as a killed process leaves them.

use std::collections::BTreeMap;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::super::fs_ops::{MutationJournal, crash_after_steps};
use super::super::intent::{IntentLog, WriteRepair, repair_interrupted_writes};
use super::super::types::{TextRewrite, WriteError};
use super::build;
use crate::cache::parse::content_hash;
use crate::vault::types::{VaultIndex, VaultScanConfig};

/// Every file and folder under `root`, by Vault-relative path, with a
/// file's bytes. A folder maps to `None`, so a leftover move gate shows.
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut found = BTreeMap::new();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).expect("read folder").flatten() {
            let file_type = entry.file_type().expect("file type");
            if file_type.is_dir() {
                found.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .expect("inside")
                        .to_path_buf(),
                    None,
                );
                folders.push(entry.path());
            } else {
                let path = entry.path();
                found.insert(
                    path.strip_prefix(root).expect("inside").to_path_buf(),
                    Some(fs::read(&path).expect("read file")),
                );
            }
        }
    }
    found
}

/// The files alone.
fn files(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    tree(root)
        .into_iter()
        .filter(|(_, content)| content.is_some())
        .collect()
}

struct Workspace {
    _dir: TempDir,
    vault: PathBuf,
    intent_log: IntentLog,
}

fn workspace(setup: &dyn Fn(&Path)) -> Workspace {
    let dir = TempDir::new().expect("temp dir");
    let vault = dir.path().join("vault");
    let state = dir.path().join("state");
    fs::create_dir_all(&vault).expect("vault");
    fs::create_dir_all(&state).expect("state");
    setup(&vault);
    Workspace {
        intent_log: IntentLog::for_vault(&state, "vault-1"),
        vault,
        _dir: dir,
    }
}

/// Stop `operation` at each of its step boundaries in turn, repair the Vault
/// as its next activation would, and require the result to be the Vault as
/// it was (stopped before any step landed) or as an uninterrupted run leaves
/// it (stopped after one did).
fn interrupted_at_every_step(setup: &dyn Fn(&Path), operation: &dyn Fn(&Path, &IntentLog)) {
    let untouched = {
        let workspace = workspace(setup);
        tree(&workspace.vault)
    };
    let finished = {
        let workspace = workspace(setup);
        operation(&workspace.vault, &workspace.intent_log);
        assert!(
            !workspace.intent_log.path().exists(),
            "a finished write leaves no record"
        );
        tree(&workspace.vault)
    };
    assert_ne!(untouched, finished, "the operation changes the Vault");

    let mut boundary = 0;
    loop {
        let workspace = workspace(setup);
        crash_after_steps(Some(boundary));
        let crashed = catch_unwind(AssertUnwindSafe(|| {
            operation(&workspace.vault, &workspace.intent_log)
        }))
        .is_err();
        crash_after_steps(None);
        if !crashed {
            assert!(boundary > 2, "the operation has several steps");
            return;
        }
        assert!(
            workspace.intent_log.path().exists(),
            "a crash after {boundary} steps leaves the record"
        );

        let repair = repair_interrupted_writes(&workspace.vault, &workspace.intent_log);

        assert!(
            !workspace.intent_log.path().exists(),
            "the repair removes the record after {boundary} steps"
        );
        assert!(repair.needs_attention.is_empty(), "{repair:?}");
        assert_eq!(
            repair.rolled_forward,
            boundary > 0,
            "after {boundary} steps"
        );
        let repaired = tree(&workspace.vault);
        if boundary == 0 {
            // Nothing landed, so every file is as it was. The folders the
            // operation made for its destinations before recording its plan
            // may stay, as they do after an in-process rollback; any other
            // folder, a move's reservation above all, may not.
            let only_files = |tree: &BTreeMap<PathBuf, Option<Vec<u8>>>| {
                tree.iter()
                    .filter(|(_, content)| content.is_some())
                    .map(|(path, content)| (path.clone(), content.clone()))
                    .collect::<BTreeMap<_, _>>()
            };
            assert_eq!(only_files(&repaired), only_files(&untouched));
            for (path, content) in &repaired {
                assert!(
                    content.is_some()
                        || untouched.contains_key(path)
                        || finished.get(path) == Some(&None),
                    "a stray folder '{}' after a crash before any step",
                    path.display()
                );
            }
        } else {
            assert_eq!(
                repaired, finished,
                "the Vault after a crash after {boundary} steps and a repair"
            );
        }
        boundary += 1;
    }
}

fn hub_with_backlinks(root: &Path) {
    fs::write(root.join("Hub.md"), "# Hub\n").expect("hub");
    for number in 0..4 {
        fs::write(
            root.join(format!("n{number}.md")),
            format!("# Note {number}\nSee [[Hub]] here.\n"),
        )
        .expect("backlink");
    }
}

#[test]
fn a_rename_stopped_at_any_step_is_finished_by_the_repair() {
    interrupted_at_every_step(&hub_with_backlinks, &|vault, intent_log| {
        let index = build(vault);
        let hub = index.find_by_slug("hub").expect("hub");
        super::super::move_or_rename_note(
            vault,
            &index,
            hub,
            "Hub Renamed.md",
            &content_hash("# Hub\n"),
            Some(intent_log),
        )
        .expect("rename");
    });
}

#[test]
fn a_delete_stopped_at_any_step_is_finished_by_the_repair() {
    interrupted_at_every_step(&hub_with_backlinks, &|vault, intent_log| {
        let index = build(vault);
        let hub = index.find_by_slug("hub").expect("hub");
        super::super::delete_note(
            vault,
            &index,
            hub,
            &content_hash("# Hub\n"),
            Some(intent_log),
        )
        .expect("delete");
    });
}

#[test]
fn an_attachment_move_stopped_at_any_step_is_finished_by_the_repair() {
    let setup = |root: &Path| {
        fs::write(root.join("image.png"), b"\x89PNG not text").expect("image");
        for number in 0..3 {
            fs::write(
                root.join(format!("n{number}.md")),
                format!("# Note {number}\n![[image.png]]\n"),
            )
            .expect("note");
        }
    };
    interrupted_at_every_step(&setup, &|vault, intent_log| {
        let index = build(vault);
        super::super::move_attachment(
            vault,
            &index,
            "image.png",
            "media/image.png",
            Some(intent_log),
        )
        .expect("move attachment");
    });
}

#[test]
fn a_tag_rename_stopped_at_any_step_is_finished_by_the_repair() {
    let setup = |root: &Path| {
        for number in 0..4 {
            fs::write(
                root.join(format!("n{number}.md")),
                format!("---\ntags: [old]\n---\n# Note {number}\nBody #old\n"),
            )
            .expect("note");
        }
    };
    interrupted_at_every_step(&setup, &|vault, intent_log| {
        let catalog = VaultIndex::build_catalog_with_config(vault, &VaultScanConfig::default())
            .expect("catalog");
        let plan =
            super::super::rename_tag(vault, &catalog, "old", "new", None, None).expect("plan");
        super::super::rename_tag(
            vault,
            &catalog,
            "old",
            "new",
            plan.plan_hash.as_deref(),
            Some(intent_log),
        )
        .expect("rename tag");
    });
}

#[test]
fn a_tag_delete_stopped_at_any_step_is_finished_by_the_repair() {
    let setup = |root: &Path| {
        for number in 0..4 {
            fs::write(
                root.join(format!("n{number}.md")),
                format!("---\ntags: [gone, kept]\n---\n# Note {number}\n"),
            )
            .expect("note");
        }
    };
    interrupted_at_every_step(&setup, &|vault, intent_log| {
        let catalog = VaultIndex::build_catalog_with_config(vault, &VaultScanConfig::default())
            .expect("catalog");
        let plan = super::super::delete_tag(vault, &catalog, "gone", None, None).expect("plan");
        super::super::delete_tag(
            vault,
            &catalog,
            "gone",
            plan.plan_hash.as_deref(),
            Some(intent_log),
        )
        .expect("delete tag");
    });
}

/// Run a rename of `Hub` with four backlinks, stopped once the note has moved
/// and one backlink is rewritten.
fn rename_stopped_part_way() -> Workspace {
    let workspace = workspace(&hub_with_backlinks);
    let index = build(&workspace.vault);
    let hub = index.find_by_slug("hub").expect("hub");
    crash_after_steps(Some(2));
    let crashed = catch_unwind(AssertUnwindSafe(|| {
        super::super::move_or_rename_note(
            &workspace.vault,
            &index,
            hub,
            "Hub Renamed.md",
            &content_hash("# Hub\n"),
            Some(&workspace.intent_log),
        )
    }))
    .is_err();
    crash_after_steps(None);
    assert!(crashed);
    workspace
}

fn links_to_old_name(vault: &Path) -> usize {
    (0..4)
        .filter(|number| {
            fs::read_to_string(vault.join(format!("n{number}.md")))
                .expect("note")
                .contains("[[Hub]]")
        })
        .count()
}

#[test]
fn a_backlink_edited_after_the_crash_is_left_alone_and_named() {
    let workspace = rename_stopped_part_way();
    assert_eq!(links_to_old_name(&workspace.vault), 3, "one rewrite landed");
    let edited = "# Note 3\nSee [[Hub]] here, and my own edit.\n";
    fs::write(workspace.vault.join("n3.md"), edited).expect("outside edit");

    let repair = repair_interrupted_writes(&workspace.vault, &workspace.intent_log);

    assert!(repair.rolled_forward);
    assert_eq!(repair.needs_attention, vec!["n3.md".to_string()]);
    assert_eq!(
        fs::read_to_string(workspace.vault.join("n3.md")).expect("n3"),
        edited
    );
    assert_eq!(
        links_to_old_name(&workspace.vault),
        1,
        "only the edited note"
    );
    assert!(workspace.vault.join("Hub Renamed.md").is_file());
    assert!(!workspace.vault.join("Hub.md").exists());
    assert!(!workspace.intent_log.path().exists());
}

#[test]
fn a_repair_keeps_each_rewritten_notes_modification_time() {
    let workspace = rename_stopped_part_way();
    let note = workspace.vault.join("n3.md");
    let before = fs::metadata(&note).expect("n3").modified().expect("mtime");

    repair_interrupted_writes(&workspace.vault, &workspace.intent_log);

    assert!(
        fs::read_to_string(&note)
            .expect("n3")
            .contains("[[Hub Renamed]]")
    );
    assert_eq!(
        fs::metadata(&note).expect("n3").modified().expect("mtime"),
        before
    );
}

#[test]
fn a_move_gate_left_before_the_exchange_is_removed_and_nothing_moves() {
    let workspace = rename_stopped_part_way();
    // Put the Vault back to the instant between the gate's creation and the
    // exchange: the note still at its name, an empty folder holding the new
    // one, and no rewrite landed.
    fs::rename(
        workspace.vault.join("Hub Renamed.md"),
        workspace.vault.join("Hub.md"),
    )
    .expect("unmove");
    fs::create_dir(workspace.vault.join("Hub Renamed.md")).expect("gate");
    hub_with_backlinks(&workspace.vault);

    let repair = repair_interrupted_writes(&workspace.vault, &workspace.intent_log);

    assert_eq!(
        repair,
        WriteRepair {
            operation: Some("move note".to_string()),
            rolled_forward: false,
            completed: Vec::new(),
            needs_attention: Vec::new(),
            kept_for_retry: false,
            removed_leftovers: vec!["Hub Renamed.md".to_string()],
        }
    );
    assert!(!workspace.vault.join("Hub Renamed.md").exists());
    assert!(workspace.vault.join("Hub.md").is_file());
    assert_eq!(links_to_old_name(&workspace.vault), 4);
}

#[test]
fn a_move_gate_left_after_the_exchange_is_removed_and_the_rename_finished() {
    let workspace = workspace(&hub_with_backlinks);
    let index = build(&workspace.vault);
    let hub = index.find_by_slug("hub").expect("hub");
    // Stopped after the record and before the move, then put at the instant
    // after the exchange: the note at its new name, the gate at its old one.
    crash_after_steps(Some(0));
    let crashed = catch_unwind(AssertUnwindSafe(|| {
        super::super::move_or_rename_note(
            &workspace.vault,
            &index,
            hub,
            "Hub Renamed.md",
            &content_hash("# Hub\n"),
            Some(&workspace.intent_log),
        )
    }))
    .is_err();
    crash_after_steps(None);
    assert!(crashed);
    fs::rename(
        workspace.vault.join("Hub.md"),
        workspace.vault.join("Hub Renamed.md"),
    )
    .expect("move");
    fs::create_dir(workspace.vault.join("Hub.md")).expect("gate");

    let repair = repair_interrupted_writes(&workspace.vault, &workspace.intent_log);

    assert!(repair.rolled_forward);
    assert_eq!(repair.removed_leftovers, vec!["Hub.md".to_string()]);
    assert!(repair.needs_attention.is_empty(), "{repair:?}");
    assert!(!workspace.vault.join("Hub.md").exists());
    assert_eq!(links_to_old_name(&workspace.vault), 0);
}

#[test]
fn leftover_temporary_files_are_moved_out_of_the_vault_and_kept() {
    let workspace = workspace(&hub_with_backlinks);
    let random = "0123456789abcdef0123456789abcdef";
    let leftover = format!("Notes/.n1.md.hatchdoor-tmp-{random}");
    fs::create_dir_all(workspace.vault.join("Notes")).expect("notes");
    fs::write(workspace.vault.join(&leftover), "half a write").expect("leftover");
    let in_git = format!(".git/.n1.md.hatchdoor-tmp-{random}");
    fs::create_dir_all(workspace.vault.join(".git")).expect("git");
    fs::write(workspace.vault.join(&in_git), "not ours").expect("git file");
    let lookalike = "Notes/.n1.md.hatchdoor-tmp-notours";
    fs::write(workspace.vault.join(lookalike), "a person's file").expect("lookalike");

    let repair = repair_interrupted_writes(&workspace.vault, &workspace.intent_log);

    assert_eq!(repair.removed_leftovers, vec![leftover.clone()]);
    assert_eq!(repair.operation, None);
    assert!(!workspace.vault.join(&leftover).exists());
    // It can hold the only copy of an edit made in another editor, so it is
    // kept beside the record rather than deleted.
    let kept = workspace
        .intent_log
        .path()
        .with_extension("leftovers")
        .join(&leftover);
    assert_eq!(fs::read_to_string(kept).expect("kept"), "half a write");
    assert!(workspace.vault.join(&in_git).exists());
    assert!(workspace.vault.join(lookalike).exists());
}

#[test]
fn a_write_whose_plan_cannot_be_recorded_changes_nothing() {
    let workspace = workspace(&hub_with_backlinks);
    let blocked = workspace.intent_log.path().parent().expect("folder");
    fs::write(blocked, "a file where the record folder goes").expect("block");
    let before = files(&workspace.vault);
    let index = build(&workspace.vault);
    let hub = index.find_by_slug("hub").expect("hub");

    let error = super::super::move_or_rename_note(
        &workspace.vault,
        &index,
        hub,
        "Hub Renamed.md",
        &content_hash("# Hub\n"),
        Some(&workspace.intent_log),
    )
    .expect_err("refused");

    assert!(
        matches!(&error, WriteError::Io(message)
            if message.contains("could not record this change")),
        "{error:?}"
    );
    assert_eq!(files(&workspace.vault), before);
}

#[test]
fn a_write_that_fails_and_is_undone_leaves_no_record() {
    let workspace = workspace(&hub_with_backlinks);
    let vault = &workspace.vault;
    let rewrite = |name: &str, original: &str| TextRewrite {
        path: vault.join(name),
        original_hash: content_hash(original),
        content: "# rewritten\n".to_string(),
    };
    // The second note was saved elsewhere after the plan read it, so the
    // journal refuses it and undoes the first.
    let rewrites = vec![
        rewrite("n0.md", "# Note 0\nSee [[Hub]] here.\n"),
        rewrite("n1.md", "an older copy"),
    ];
    let mut journal = MutationJournal::new(vault, Some(&workspace.intent_log));
    journal.begin("move note", &[], &rewrites).expect("record");
    assert!(workspace.intent_log.path().exists());
    let error = journal.apply_rewrites(rewrites).expect_err("conflict");
    let error = journal.rollback(error);

    assert!(matches!(error, WriteError::Conflict(_)), "{error:?}");
    assert!(!workspace.intent_log.path().exists());
    assert_eq!(
        fs::read_to_string(vault.join("n0.md")).expect("n0"),
        "# Note 0\nSee [[Hub]] here.\n"
    );
}

#[test]
fn a_record_from_a_newer_hatchdoor_is_left_alone() {
    let workspace = workspace(&hub_with_backlinks);
    let record = workspace.intent_log.path();
    fs::create_dir_all(record.parent().expect("folder")).expect("folder");
    let newer = r#"{"schema_version":2,"operation":"move note","moves":[],"rewrites":[]}"#;
    fs::write(record, newer).expect("record");

    let repair = repair_interrupted_writes(&workspace.vault, &workspace.intent_log);

    assert_eq!(repair.operation, None);
    assert_eq!(fs::read_to_string(record).expect("record"), newer);
}

/// Restore write permission on drop, so a failed assertion cannot leave a
/// temporary folder the test run cannot delete.
struct ReadOnly<'a>(&'a Path);

impl<'a> ReadOnly<'a> {
    fn new(path: &'a Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o555)).expect("read-only");
        Self(path)
    }
}

impl Drop for ReadOnly<'_> {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
    }
}

#[test]
fn a_repair_that_hits_a_filesystem_error_keeps_the_record_and_finishes_later() {
    let workspace = rename_stopped_part_way();
    if nix_is_root() {
        // Root writes into a read-only folder, so there is no error to hit.
        return;
    }
    let repair = {
        let _read_only = ReadOnly::new(&workspace.vault);
        repair_interrupted_writes(&workspace.vault, &workspace.intent_log)
    };
    assert!(repair.kept_for_retry, "{repair:?}");
    assert!(repair.needs_attention.is_empty(), "{repair:?}");
    assert!(
        workspace.intent_log.path().exists(),
        "kept for the next start"
    );
    assert_eq!(links_to_old_name(&workspace.vault), 3);

    let repair = repair_interrupted_writes(&workspace.vault, &workspace.intent_log);

    assert!(!repair.kept_for_retry, "{repair:?}");
    assert!(!workspace.intent_log.path().exists());
    assert_eq!(links_to_old_name(&workspace.vault), 0);
}

fn nix_is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn a_new_write_is_refused_while_an_earlier_record_waits() {
    let workspace = rename_stopped_part_way();
    let before = files(&workspace.vault);
    let index = build(&workspace.vault);
    let other = index.find_by_slug("n0").expect("n0");

    let error = super::super::move_or_rename_note(
        &workspace.vault,
        &index,
        other,
        "n0 moved.md",
        &content_hash(&fs::read_to_string(workspace.vault.join("n0.md")).expect("n0")),
        Some(&workspace.intent_log),
    )
    .expect_err("refused");

    assert!(
        matches!(&error, WriteError::Io(message) if message.contains("still waiting")),
        "{error:?}"
    );
    assert_eq!(files(&workspace.vault), before);
    assert!(
        workspace.intent_log.path().exists(),
        "the waiting record is kept"
    );
}

#[test]
fn backlinks_are_not_rewritten_when_the_move_cannot_be_finished() {
    let workspace = workspace(&hub_with_backlinks);
    let index = build(&workspace.vault);
    let hub = index.find_by_slug("hub").expect("hub");
    // Stopped once the note has moved, before any backlink.
    crash_after_steps(Some(1));
    let crashed = catch_unwind(AssertUnwindSafe(|| {
        super::super::move_or_rename_note(
            &workspace.vault,
            &index,
            hub,
            "Hub Renamed.md",
            &content_hash("# Hub\n"),
            Some(&workspace.intent_log),
        )
    }))
    .is_err();
    crash_after_steps(None);
    assert!(crashed);
    // Then someone put the renamed note back by hand while Hatchdoor was
    // down, and something else took the new name.
    fs::rename(
        workspace.vault.join("Hub Renamed.md"),
        workspace.vault.join("Hub.md"),
    )
    .expect("put back");
    fs::write(workspace.vault.join("Hub Renamed.md"), "someone else").expect("other");

    let repair = repair_interrupted_writes(&workspace.vault, &workspace.intent_log);

    assert_eq!(repair.needs_attention, vec!["Hub.md".to_string()]);
    assert_eq!(
        links_to_old_name(&workspace.vault),
        4,
        "no backlink rewritten"
    );
    assert_eq!(
        fs::read_to_string(workspace.vault.join("Hub Renamed.md")).expect("other"),
        "someone else"
    );
    assert!(!workspace.intent_log.path().exists());
}
