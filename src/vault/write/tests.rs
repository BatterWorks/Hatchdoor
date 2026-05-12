use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::*;
use crate::cache::parse::content_hash;
use crate::vault::types::VaultIndex;

fn build(root: &Path) -> VaultIndex {
    VaultIndex::build(root).expect("build index")
}

#[test]
fn create_note_rejects_traversal_and_writes_markdown() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    assert!(matches!(
        create_note(root, "../Escape.md", "no", false),
        Err(WriteError::InvalidInput(_))
    ));
    create_note(root, "Projects/New", "# New", false).expect("create");
    assert_eq!(
        fs::read_to_string(root.join("Projects/New.md")).expect("read"),
        "# New"
    );
}

#[test]
fn update_note_requires_matching_hash() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    fs::write(&path, "old").expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");
    assert!(matches!(
        update_note(entry, "new", "fnv1a64:deadbeef"),
        Err(WriteError::Conflict(_))
    ));
    update_note(entry, "new", &content_hash("old")).expect("update");
    assert_eq!(fs::read_to_string(path).expect("read"), "new");
}

#[test]
fn move_note_rewrites_backlinks_and_moves_referenced_assets() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Notes")).expect("mkdir");
    fs::write(root.join("Notes/Target.md"), "body\n![](image.png)").expect("target");
    fs::write(root.join("Notes/image.png"), "png").expect("asset");
    fs::create_dir_all(root.join("Other")).expect("other dir");
    fs::write(
        root.join("Backlink.md"),
        "[[Target|Alias]] and ![](Notes/image.png) and `[[Target]]`\n```\n[[Target]]\n![](Notes/image.png)\n```",
    )
    .expect("backlink");
    fs::write(
        root.join("Other/Shared.md"),
        "shared ![](../Notes/image.png) and `![](../Notes/image.png)`",
    )
    .expect("shared");
    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");

    let outcome = move_or_rename_note(
        root,
        &index,
        entry,
        "Archive/Renamed.md",
        &content_hash("body\n![](image.png)"),
    )
    .expect("move");

    assert_eq!(outcome.rewritten_notes, 2);
    assert_eq!(outcome.moved_assets, 1);
    assert!(root.join("Archive/Renamed.md").exists());
    assert!(root.join("Archive/image.png").exists());
    let backlink = fs::read_to_string(root.join("Backlink.md")).expect("read");
    assert!(backlink.contains("[[Archive/Renamed|Alias]]"));
    assert!(backlink.contains("![](Archive/image.png)"));
    assert!(backlink.contains("`[[Target]]`"));
    assert!(backlink.contains("```\n[[Target]]\n![](Notes/image.png)\n```"));
    let shared = fs::read_to_string(root.join("Other/Shared.md")).expect("shared");
    assert!(shared.contains("![](../Archive/image.png)"));
    assert!(shared.contains("`![](../Notes/image.png)`"));
}

#[test]
fn delete_note_moves_note_and_assets_to_trash_and_removes_backlinks() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Target.md"), "body ![](asset.pdf)").expect("target");
    fs::write(root.join("asset.pdf"), "pdf").expect("asset");
    fs::write(
        root.join("Backlink.md"),
        "before [[Target]] after ![](asset.pdf)",
    )
    .expect("backlink");
    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");

    let outcome =
        delete_note(root, &index, entry, &content_hash("body ![](asset.pdf)")).expect("delete");

    let trash = outcome.trashed_path.expect("trash path");
    assert!(!root.join("Target.md").exists());
    assert!(root.join(format!("{trash}.md")).exists());
    assert!(root.join(".hatchdoor-trash/asset.pdf").exists());
    let backlink = fs::read_to_string(root.join("Backlink.md")).expect("backlink");
    assert_eq!(backlink, "before  after ![](.hatchdoor-trash/asset.pdf)");
}

#[test]
fn delete_note_does_not_move_already_trashed_attachment_again() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Notes/Media")).expect("media");
    fs::write(root.join("Notes/Target.md"), "body ![](Media/image.png)").expect("target");
    fs::write(root.join("Notes/Media/image.png"), "png").expect("asset");

    let index = build(root);
    delete_attachment(root, &index, "Notes/Media/image.png").expect("delete attachment");
    let rewritten = fs::read_to_string(root.join("Notes/Target.md")).expect("target");
    assert!(rewritten.contains("![](../.hatchdoor-trash/Notes/Media/image.png)"));

    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");
    let outcome = delete_note(root, &index, entry, &content_hash(&rewritten)).expect("delete note");

    assert_eq!(outcome.moved_assets, 0);
    assert!(root.join(".hatchdoor-trash/Notes/Media/image.png").exists());
    assert!(
        !root
            .join(".hatchdoor-trash/.hatchdoor-trash/Notes/Media/image.png")
            .exists()
    );
    assert!(root.join(".hatchdoor-trash/Notes/Target.md").exists());
}
