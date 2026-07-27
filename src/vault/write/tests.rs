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
fn list_note_attachments_reports_the_containing_folders_layer() {
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    fs::create_dir_all(root.join("sources")).expect("sources dir");
    fs::write(
        root.join("sources/.hatchdoor-layer"),
        "name: sources\ndescription: Raw clippings.\n",
    )
    .expect("marker");
    fs::write(root.join("sources/Clip.md"), "# Clip\n![](diagram.png)").expect("clip");
    fs::write(root.join("sources/diagram.png"), "png").expect("asset");
    fs::write(root.join("Wiki.md"), "# Wiki\n![](wiki.png)").expect("wiki");
    fs::write(root.join("wiki.png"), "png").expect("default asset");

    let index = build(root);

    let clip = index.find_by_slug("clip").expect("clip entry").clone();
    let clip_assets =
        list_note_attachments(root, &index.layers, &clip).expect("list clip attachments");
    assert_eq!(clip_assets.len(), 1);
    assert_eq!(
        clip_assets[0].layer.as_deref(),
        Some("sources"),
        "an asset in a demoted folder must report that folder's layer"
    );

    let wiki = index.find_by_slug("wiki").expect("wiki entry").clone();
    let wiki_assets =
        list_note_attachments(root, &index.layers, &wiki).expect("list wiki attachments");
    assert_eq!(wiki_assets.len(), 1);
    assert_eq!(
        wiki_assets[0].layer, None,
        "a default-surface asset reports a null layer"
    );
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
        "# New\n"
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
    assert_eq!(fs::read_to_string(path).expect("read"), "new\n");
}

#[test]
fn write_quality_normalizes_safe_markdown_formatting() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let outcome = create_note(root, "Quality", "# Quality\r\nBody", false).expect("create");

    assert_eq!(
        fs::read_to_string(root.join("Quality.md")).expect("read"),
        "# Quality\nBody\n"
    );
    assert_eq!(
        outcome.quality_warnings,
        vec![
            "normalized CRLF/CR line endings to LF".to_string(),
            "added final newline".to_string()
        ]
    );
    assert_eq!(
        outcome.content_hash,
        Some(content_hash("# Quality\nBody\n"))
    );
}

#[test]
fn write_quality_reports_frontmatter_warnings() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let outcome = create_note(
        root,
        "Quality",
        "---\ntags: [a]\ntags: [b]\n# Missing close\n",
        false,
    )
    .expect("create");

    assert_eq!(
        outcome.quality_warnings,
        vec![
            "frontmatter has duplicate key: tags".to_string(),
            "frontmatter opening marker has no closing marker".to_string(),
        ]
    );
}

#[test]
fn write_quality_rejects_nul_bytes() {
    let tmp = TempDir::new().expect("tempdir");
    assert!(matches!(
        create_note(tmp.path(), "Bad", "hello\0world", false),
        Err(WriteError::InvalidInput(message)) if message.contains("NUL")
    ));
    assert!(!tmp.path().join("Bad.md").exists());
}

#[test]
fn edit_note_replaces_unique_string() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    fs::write(&path, "alpha beta gamma").expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    let outcome = edit_note(
        entry,
        "beta",
        "BETA",
        &content_hash("alpha beta gamma"),
        false,
    )
    .expect("edit");

    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "alpha BETA gamma\n"
    );
    assert_eq!(
        outcome.content_hash,
        Some(content_hash("alpha BETA gamma\n"))
    );
}

#[test]
fn edit_note_rejects_missing_string() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    fs::write(&path, "alpha").expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    assert!(matches!(
        edit_note(entry, "missing", "x", &content_hash("alpha"), false),
        Err(WriteError::InvalidInput(_))
    ));
    assert_eq!(fs::read_to_string(&path).expect("read"), "alpha");
}

#[test]
fn edit_note_rejects_ambiguous_match_without_replace_all() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    fs::write(&path, "x and x").expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    assert!(matches!(
        edit_note(entry, "x", "y", &content_hash("x and x"), false),
        Err(WriteError::Conflict(_))
    ));
    assert_eq!(fs::read_to_string(&path).expect("read"), "x and x");
}

#[test]
fn edit_note_replace_all_replaces_every_occurrence() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    fs::write(&path, "x x x").expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    edit_note(entry, "x", "y", &content_hash("x x x"), true).expect("edit");

    assert_eq!(fs::read_to_string(&path).expect("read"), "y y y\n");
}

#[test]
fn edit_note_requires_matching_hash() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    fs::write(&path, "alpha").expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    assert!(matches!(
        edit_note(entry, "alpha", "beta", "fnv1a64:deadbeef", false),
        Err(WriteError::Conflict(_))
    ));
    assert_eq!(fs::read_to_string(&path).expect("read"), "alpha");
}

#[test]
fn replace_section_replaces_heading_and_body() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    let body = "# Title\n\n## One\nfirst\n\n## Two\nsecond\n";
    fs::write(&path, body).expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    replace_section(
        entry,
        "## One",
        SectionMode::Replace,
        "## One\nNEW\n",
        &content_hash(body),
    )
    .expect("replace");

    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "# Title\n\n## One\nNEW\n## Two\nsecond\n"
    );
}

#[test]
fn replace_section_before_inserts_above_heading() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    let body = "## One\nfirst\n## Two\nsecond\n";
    fs::write(&path, body).expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    replace_section(
        entry,
        "## Two",
        SectionMode::Before,
        "## Inserted\nx\n",
        &content_hash(body),
    )
    .expect("replace");

    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "## One\nfirst\n## Inserted\nx\n## Two\nsecond\n"
    );
}

#[test]
fn replace_section_after_inserts_below_section() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    let body = "## One\nfirst\n## Two\nsecond\n";
    fs::write(&path, body).expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    replace_section(
        entry,
        "## One",
        SectionMode::After,
        "## Inserted\nx\n",
        &content_hash(body),
    )
    .expect("replace");

    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "## One\nfirst\n## Inserted\nx\n## Two\nsecond\n"
    );
}

#[test]
fn replace_section_ignores_heading_inside_code_fence() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    let body = "## Real\nbefore\n```\n## Fake\n```\nafter\n## Next\nx\n";
    fs::write(&path, body).expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    replace_section(
        entry,
        "## Real",
        SectionMode::Replace,
        "## Real\nNEW\n",
        &content_hash(body),
    )
    .expect("replace");

    assert_eq!(
        fs::read_to_string(&path).expect("read"),
        "## Real\nNEW\n## Next\nx\n"
    );
}

#[test]
fn replace_section_rejects_missing_heading() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    let body = "## One\nfirst\n";
    fs::write(&path, body).expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    assert!(matches!(
        replace_section(
            entry,
            "## Missing",
            SectionMode::Replace,
            "x",
            &content_hash(body),
        ),
        Err(WriteError::InvalidInput(_))
    ));
    assert_eq!(fs::read_to_string(&path).expect("read"), body);
}

#[test]
fn replace_section_rejects_duplicate_heading() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("Home.md");
    let body = "## Dup\na\n## Dup\nb\n";
    fs::write(&path, body).expect("write");
    let index = build(tmp.path());
    let entry = index.find_by_slug("home").expect("home");

    assert!(matches!(
        replace_section(
            entry,
            "## Dup",
            SectionMode::Replace,
            "x",
            &content_hash(body),
        ),
        Err(WriteError::Conflict(_))
    ));
    assert_eq!(fs::read_to_string(&path).expect("read"), body);
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
fn archive_note_moves_to_archive_prefix_and_rewrites_backlinks() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("40-reference")).expect("mkdir");
    fs::write(root.join("40-reference/Idea.md"), "body").expect("target");
    fs::write(root.join("Backlink.md"), "See [[Idea]]").expect("backlink");
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    let outcome =
        archive_note(root, &index, entry, "90-archive/", &content_hash("body")).expect("archive");

    assert_eq!(outcome.relative_path, Some("90-archive/Idea".to_string()));
    assert_eq!(outcome.rewritten_notes, 1);
    assert!(!root.join("40-reference/Idea.md").exists());
    assert!(root.join("90-archive/Idea.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("Backlink.md")).expect("backlink"),
        "See [[90-archive/Idea]]"
    );
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
