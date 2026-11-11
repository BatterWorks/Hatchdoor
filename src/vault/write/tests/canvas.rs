//! What a rename, move, archive or delete does to the Vault's canvases
//! (#579, ADR-53). The editor's own cases live beside it in `canvas.rs`.

use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::{
    archive_note, build, delete_attachment, delete_note, move_attachment, move_or_rename_note,
    rename_attachment,
};
use crate::cache::parse::content_hash;

/// A canvas nobody would write by hand or by tool: an unknown top-level key,
/// keys out of order, mixed indentation, a CRLF and no trailing newline.
fn odd_canvas(file: &str) -> String {
    format!(
        "{{\r\n\t\"future\":{{\"keep\":[1, 2,3]}},\n  \"nodes\" : [\n    {{\"id\":\"a\",\"file\" :  \"{file}\", \"type\":\"file\",\"x\":-120,\"y\":0,\"width\":400,\"height\":400,\"unknown\":null}},\n    {{\"id\":\"b\",\"type\":\"text\",\"text\":\"mentions {file}\",\"x\":0,\"y\":0,\"width\":1,\"height\":1}}\n  ],\n  \"edges\":[]}}"
    )
}

#[test]
fn renaming_a_note_rewrites_only_the_path_of_the_card_that_names_it() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Notes")).expect("notes");
    fs::create_dir_all(root.join("Boards")).expect("boards");
    fs::write(root.join("Notes/Idea.md"), "body").expect("note");
    fs::write(root.join("Boards/Plan.canvas"), odd_canvas("Notes/Idea.md")).expect("canvas");
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    let outcome = move_or_rename_note(
        root,
        &index,
        entry,
        "Notes/Better idea.md",
        &content_hash("body"),
    )
    .expect("rename");

    // Byte for byte the canvas it was, but for the one path. The text card
    // that only mentions the path keeps it.
    let expected = odd_canvas("Notes/Idea.md").replacen(
        "\"file\" :  \"Notes/Idea.md\"",
        "\"file\" :  \"Notes/Better idea.md\"",
        1,
    );
    assert_eq!(
        fs::read(root.join("Boards/Plan.canvas")).expect("canvas"),
        expected.into_bytes()
    );
    assert_eq!(outcome.rewritten_notes, 0, "a canvas is not a note");
    assert!(outcome.quality_warnings.is_empty());
    assert!(
        outcome
            .affected_paths
            .contains(&root.join("Boards/Plan.canvas")),
        "the canvas is a changed file: {:?}",
        outcome.affected_paths
    );
}

#[test]
fn a_canvas_naming_an_unrelated_file_is_untouched() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Idea.md"), "body").expect("note");
    fs::write(root.join("Other.md"), "other").expect("other");
    let canvas = odd_canvas("Other.md");
    fs::write(root.join("Plan.canvas"), &canvas).expect("canvas");
    let before = fs::metadata(root.join("Plan.canvas"))
        .expect("metadata")
        .modified()
        .expect("modified");
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    let outcome = move_or_rename_note(root, &index, entry, "Renamed.md", &content_hash("body"))
        .expect("rename");

    assert_eq!(
        fs::read_to_string(root.join("Plan.canvas")).expect("canvas"),
        canvas
    );
    assert_eq!(
        fs::metadata(root.join("Plan.canvas"))
            .expect("metadata")
            .modified()
            .expect("modified"),
        before
    );
    assert!(!outcome.affected_paths.contains(&root.join("Plan.canvas")));
}

#[test]
fn archiving_a_note_rewrites_the_canvas_that_names_it() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Notes")).expect("notes");
    fs::write(root.join("Notes/Idea.md"), "body").expect("note");
    fs::write(root.join("Plan.canvas"), odd_canvas("Notes/Idea.md")).expect("canvas");
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    archive_note(root, &index, entry, "Archive", &content_hash("body")).expect("archive");

    assert_eq!(
        fs::read_to_string(root.join("Plan.canvas")).expect("canvas"),
        odd_canvas("Notes/Idea.md").replacen(
            "\"file\" :  \"Notes/Idea.md\"",
            "\"file\" :  \"Archive/Idea.md\"",
            1
        )
    );
}

#[test]
fn deleting_a_note_or_an_attachment_leaves_the_canvas_byte_identical() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Idea.md"), "body").expect("note");
    fs::write(root.join("photo.png"), b"png").expect("image");
    let canvas = format!(
        "{}\n{}",
        odd_canvas("Idea.md"),
        r#"{"nodes":[{"type":"file","file":"photo.png"}]}"#
    );
    // Two documents in one file is not JSON, which a delete must not care
    // about: it reads no canvas at all.
    fs::write(root.join("Plan.canvas"), &canvas).expect("canvas");
    fs::write(root.join("Valid.canvas"), odd_canvas("Idea.md")).expect("valid canvas");
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    let outcome = delete_note(root, &index, entry, &content_hash("body")).expect("delete note");
    assert!(outcome.quality_warnings.is_empty());
    let index = build(root);
    let attachment = delete_attachment(root, &index, "photo.png").expect("delete attachment");
    assert_eq!(attachment.cleanup_warning, None);

    assert_eq!(
        fs::read_to_string(root.join("Plan.canvas")).expect("canvas"),
        canvas
    );
    assert_eq!(
        fs::read_to_string(root.join("Valid.canvas")).expect("valid canvas"),
        odd_canvas("Idea.md")
    );
}

#[test]
fn moving_or_renaming_an_attachment_rewrites_the_canvas_that_names_it() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("media")).expect("media");
    fs::write(root.join("media/photo.png"), b"png").expect("image");
    fs::write(root.join("Note.md"), "![[media/photo.png]]").expect("note");
    fs::write(root.join("Plan.canvas"), odd_canvas("media/photo.png")).expect("canvas");
    let index = build(root);

    let outcome =
        move_attachment(root, &index, "media/photo.png", "pictures/photo.png").expect("move");

    assert_eq!(outcome.rewritten_notes, 1, "the note, not the canvas");
    assert_eq!(outcome.cleanup_warning, None);
    assert!(outcome.affected_paths.contains(&root.join("Plan.canvas")));
    let moved = odd_canvas("media/photo.png").replacen(
        "\"file\" :  \"media/photo.png\"",
        "\"file\" :  \"pictures/photo.png\"",
        1,
    );
    assert_eq!(
        fs::read_to_string(root.join("Plan.canvas")).expect("canvas"),
        moved
    );

    let index = build(root);
    rename_attachment(root, &index, "pictures/photo.png", "sunset.png").expect("rename");

    assert_eq!(
        fs::read_to_string(root.join("Plan.canvas")).expect("canvas"),
        moved.replacen("pictures/photo.png", "pictures/sunset.png", 1)
    );
}

#[test]
fn an_asset_travelling_with_its_note_is_repointed_on_the_canvas_too() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Trip")).expect("trip");
    fs::write(root.join("Trip/Log.md"), "![[photo.png]]").expect("note");
    fs::write(root.join("Trip/photo.png"), b"png").expect("image");
    fs::write(
        root.join("Plan.canvas"),
        r#"{"nodes":[{"type":"file","file":"Trip/Log.md"},{"type":"file","file":"Trip/photo.png"}]}"#,
    )
    .expect("canvas");
    let index = build(root);
    let entry = index.find_by_slug("log").expect("log");

    let outcome = move_or_rename_note(
        root,
        &index,
        entry,
        "Done/Log.md",
        &content_hash("![[photo.png]]"),
    )
    .expect("move");

    assert_eq!(outcome.moved_assets, 1);
    assert_eq!(
        fs::read_to_string(root.join("Plan.canvas")).expect("canvas"),
        r#"{"nodes":[{"type":"file","file":"Done/Log.md"},{"type":"file","file":"Done/photo.png"}]}"#
    );
}

#[test]
fn a_canvas_that_moves_is_rewritten_where_it_lands() {
    // The canvas sits in the note's folder and is embedded by it, so it
    // travels with the note, and it names the note.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Trip")).expect("trip");
    fs::write(root.join("Trip/Log.md"), "![[Board.canvas]]").expect("note");
    fs::write(
        root.join("Trip/Board.canvas"),
        r#"{"nodes":[{"type":"file","file":"Trip/Log.md"}]}"#,
    )
    .expect("canvas");
    let index = build(root);
    let entry = index.find_by_slug("log").expect("log");

    move_or_rename_note(
        root,
        &index,
        entry,
        "Done/Log.md",
        &content_hash("![[Board.canvas]]"),
    )
    .expect("move");

    assert!(!root.join("Trip/Board.canvas").exists());
    assert_eq!(
        fs::read_to_string(root.join("Done/Board.canvas")).expect("canvas"),
        r#"{"nodes":[{"type":"file","file":"Done/Log.md"}]}"#
    );
}

#[test]
fn a_canvas_that_is_not_json_is_skipped_and_reported_and_the_rename_goes_on() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Idea.md"), "body").expect("note");
    fs::write(root.join("Backlink.md"), "See [[Idea]]").expect("backlink");
    let broken = r#"{"nodes":[{"type":"file","file":"Idea.md"},]}"#;
    fs::write(root.join("Broken.canvas"), broken).expect("broken");
    // Not JSON either, but it does not mention the note, so it is not named.
    fs::write(root.join("Unrelated.canvas"), "{ nope").expect("unrelated");
    fs::write(root.join("Binary.canvas"), b"\xff\xfe Idea.md").expect("binary");
    fs::write(root.join("Good.canvas"), odd_canvas("Idea.md")).expect("good");
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    let outcome = move_or_rename_note(root, &index, entry, "Renamed.md", &content_hash("body"))
        .expect("the rename succeeds for the notes");

    assert_eq!(outcome.rewritten_notes, 1);
    assert_eq!(
        fs::read_to_string(root.join("Backlink.md")).expect("backlink"),
        "See [[Renamed]]"
    );
    assert_eq!(
        fs::read_to_string(root.join("Broken.canvas")).expect("broken"),
        broken
    );
    assert_eq!(
        fs::read(root.join("Binary.canvas")).expect("binary"),
        b"\xff\xfe Idea.md"
    );
    assert_eq!(
        fs::read_to_string(root.join("Good.canvas")).expect("good"),
        odd_canvas("Idea.md").replacen("\"file\" :  \"Idea.md\"", "\"file\" :  \"Renamed.md\"", 1)
    );
    assert_eq!(
        outcome.quality_warnings,
        vec![
            "Canvas 'Binary.canvas' was not updated: it is not valid UTF-8 text".to_string(),
            "Canvas 'Broken.canvas' was not updated: it is not valid JSON".to_string(),
        ]
    );
}

#[test]
fn an_attachment_move_reports_a_canvas_it_could_not_read() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("photo.png"), b"png").expect("image");
    fs::write(root.join("Broken.canvas"), "{\"file\":\"photo.png\"").expect("broken");
    let index = build(root);

    let outcome = move_attachment(root, &index, "photo.png", "media/photo.png").expect("move");

    assert_eq!(
        outcome.cleanup_warning.as_deref(),
        Some("Canvas 'Broken.canvas' was not updated: it is not valid JSON")
    );
    assert!(root.join("media/photo.png").exists());
}

#[test]
fn a_canvas_over_the_size_bound_is_never_read_and_always_reported() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Idea.md"), "body").expect("note");
    let huge = fs::File::create(root.join("Huge.canvas")).expect("huge");
    huge.set_len(crate::vault::MAX_NOTE_BYTES + 1)
        .expect("sparse");
    drop(huge);
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    let outcome = move_or_rename_note(root, &index, entry, "Renamed.md", &content_hash("body"))
        .expect("rename");

    assert_eq!(
        outcome.quality_warnings,
        vec![
            "Canvas 'Huge.canvas' was not updated: it is larger than 8 MB, so it was not read"
                .to_string()
        ]
    );
    assert_eq!(
        fs::metadata(root.join("Huge.canvas")).expect("huge").len(),
        crate::vault::MAX_NOTE_BYTES + 1
    );
}

#[test]
fn a_canvas_in_an_excluded_folder_is_left_alone() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join(".obsidian")).expect("obsidian");
    fs::write(root.join("Idea.md"), "body").expect("note");
    let canvas = odd_canvas("Idea.md");
    fs::write(root.join(".obsidian/Plan.canvas"), &canvas).expect("canvas");
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    move_or_rename_note(root, &index, entry, "Renamed.md", &content_hash("body")).expect("rename");

    assert_eq!(
        fs::read_to_string(root.join(".obsidian/Plan.canvas")).expect("canvas"),
        canvas
    );
}

#[test]
fn a_failure_after_the_canvas_rewrite_puts_the_canvas_back() {
    let tmp = TempDir::new().expect("tempdir");
    let root: &Path = tmp.path();
    fs::write(root.join("Idea.md"), "body").expect("note");
    let canvas = odd_canvas("Idea.md");
    fs::write(root.join("Plan.canvas"), &canvas).expect("canvas");
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    let error = super::super::notes::move_or_rename_note_with_failure(
        root,
        &index,
        entry,
        "Renamed.md",
        &content_hash("body"),
        |phase| match phase {
            super::super::types::MutationPhase::Rewrite => {
                Err(super::WriteError::Io("injected".to_string()))
            }
            _ => Ok(()),
        },
    )
    .expect_err("the injected failure fails the rename");

    assert!(matches!(error, super::WriteError::Io(_)), "got {error:?}");
    assert!(root.join("Idea.md").exists());
    assert_eq!(
        fs::read_to_string(root.join("Plan.canvas")).expect("canvas"),
        canvas
    );
}

#[test]
fn a_canvas_that_cannot_be_opened_is_reported() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Idea.md"), "body").expect("note");
    let canvas = root.join("Locked.canvas");
    fs::write(&canvas, odd_canvas("Idea.md")).expect("canvas");
    fs::set_permissions(&canvas, fs::Permissions::from_mode(0o000)).expect("lock");
    if fs::read(&canvas).is_ok() {
        // Root reads anything, so there is nothing to observe.
        return;
    }
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");

    let outcome = move_or_rename_note(root, &index, entry, "Renamed.md", &content_hash("body"))
        .expect("rename");

    assert_eq!(
        outcome.quality_warnings,
        vec!["Canvas 'Locked.canvas' was not updated: it could not be read".to_string()]
    );
}

#[test]
fn a_canvas_saved_elsewhere_after_the_plan_fails_the_rename_and_is_named_as_a_canvas() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Idea.md"), "body").expect("note");
    fs::write(root.join("Plan.canvas"), odd_canvas("Idea.md")).expect("canvas");
    let index = build(root);
    let entry = index.find_by_slug("idea").expect("idea");
    let saved_elsewhere = format!("{}\n", odd_canvas("Idea.md"));

    let error = super::super::notes::move_or_rename_note_with_failure(
        root,
        &index,
        entry,
        "Renamed.md",
        &content_hash("body"),
        |phase| {
            if phase == super::super::types::MutationPhase::Note {
                fs::write(root.join("Plan.canvas"), &saved_elsewhere).expect("outside save");
            }
            Ok(())
        },
    )
    .expect_err("a canvas that moved on is not overwritten");

    assert!(
        matches!(&error, super::WriteError::Conflict(message) if message.starts_with("Canvas 'Plan.canvas' changed")),
        "got {error:?}"
    );
    assert!(root.join("Idea.md").exists(), "the rename was undone");
    assert_eq!(
        fs::read_to_string(root.join("Plan.canvas")).expect("canvas"),
        saved_elsewhere
    );
}
