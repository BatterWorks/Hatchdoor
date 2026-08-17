use super::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn normalize_title_trims_and_lowercases() {
    assert_eq!(normalize_title("  My NOTE  "), "my note");
}

#[test]
fn strip_md_extension_only_removes_suffix() {
    assert_eq!(strip_md_extension("Note.md"), "Note");
    assert_eq!(strip_md_extension("Note.markdown"), "Note.markdown");
}

#[test]
fn slugify_reduces_symbols_to_clean_slug() {
    assert_eq!(slugify("  My Great_Note!!  "), "my-great-note");
}

#[test]
fn normalize_link_target_strips_md_and_normalizes_separators() {
    assert_eq!(
        normalize_link_target(r"Folder\My Note.md"),
        "Folder/My Note"
    );
}

#[test]
fn build_indexes_markdown_files_only() {
    let dir = tempdir().expect("temp dir");
    fs::write(dir.path().join("Home.md"), "# Home").expect("write note");
    fs::write(dir.path().join("readme.txt"), "ignore").expect("write text");
    fs::create_dir_all(dir.path().join(".hatchdoor-trash")).expect("create trash");
    fs::write(dir.path().join(".hatchdoor-trash/Deleted.md"), "# Deleted").expect("write trash");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    assert_eq!(vault.total_notes(), 1);
    assert!(vault.resolve_wikilink("Home").is_some());
    let found = vault.find_by_slug("home").expect("home by slug");
    assert_eq!(found.relative_path, "Home");
}

#[test]
fn resolve_wikilink_supports_title_slug_and_md_suffix() {
    let dir = tempdir().expect("temp dir");
    fs::write(dir.path().join("Second Note.md"), "content").expect("write note");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    assert!(vault.resolve_wikilink("Second Note").is_some());
    assert!(vault.resolve_wikilink("second-note").is_some());
    assert!(vault.resolve_wikilink("Second Note.md").is_some());
}

#[test]
fn resolve_wikilink_strips_heading_and_block_anchors() {
    let dir = tempdir().expect("temp dir");
    fs::write(dir.path().join("My Quotes.md"), "content").expect("write note");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let result = vault
        .resolve_wikilink("My Quotes#Marcus Aurelius")
        .expect("heading anchor should resolve to note");
    assert_eq!(result.slug, "my-quotes");

    let result = vault
        .resolve_wikilink("My Quotes^block-id")
        .expect("block anchor should resolve to note");
    assert_eq!(result.slug, "my-quotes");
}

#[test]
fn duplicate_titles_get_unique_slugs() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir(dir.path().join("a")).expect("create dir a");
    fs::create_dir(dir.path().join("b")).expect("create dir b");
    fs::write(dir.path().join("a").join("Note.md"), "first").expect("write first");
    fs::write(dir.path().join("b").join("Note.md"), "second").expect("write second");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    assert!(vault.find_by_slug("note").is_some());
    assert!(vault.find_by_slug("note-2").is_some());
}

#[test]
fn resolve_wikilink_duplicate_title_uses_deterministic_first_path() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir(dir.path().join("b")).expect("create dir b");
    fs::create_dir(dir.path().join("a")).expect("create dir a");
    fs::write(dir.path().join("b").join("Note.md"), "b").expect("write b");
    fs::write(dir.path().join("a").join("Note.md"), "a").expect("write a");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let resolved = vault.resolve_wikilink("Note").expect("note exists");
    assert_eq!(resolved.path, dir.path().join("a").join("Note.md"));
}

#[test]
fn resolve_wikilink_supports_folder_qualified_targets() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir(dir.path().join("folder")).expect("create folder");
    fs::write(dir.path().join("folder").join("Doc.md"), "content").expect("write doc");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let resolved = vault
        .resolve_wikilink("folder/Doc")
        .expect("folder-qualified match");
    assert_eq!(resolved.path, dir.path().join("folder").join("Doc.md"));
}

#[test]
fn explorer_tree_preserves_folder_structure() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("projects/rust")).expect("create nested folders");
    fs::write(dir.path().join("Home.md"), "home").expect("write home");
    fs::write(dir.path().join("projects").join("Plan.md"), "plan").expect("write plan");
    fs::write(dir.path().join("projects/rust").join("Notes.md"), "notes").expect("write notes");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let tree = vault.explorer_tree();

    assert_eq!(tree.name, "Vault");
    assert_eq!(tree.notes.len(), 1);
    assert_eq!(tree.notes[0].title, "Home");
    assert_eq!(tree.folders.len(), 1);
    assert_eq!(tree.folders[0].name, "projects");
    assert_eq!(tree.folders[0].notes[0].title, "Plan");
    assert_eq!(tree.folders[0].folders[0].name, "rust");
    assert_eq!(tree.folders[0].folders[0].notes[0].title, "Notes");
}

#[test]
fn read_note_by_slug_reads_existing_and_handles_missing() {
    let dir = tempdir().expect("temp dir");
    fs::write(dir.path().join("Home.md"), "hello").expect("write note");
    let vault = VaultIndex::build(dir.path()).expect("build vault");

    let note = vault
        .read_note_by_slug("home")
        .expect("read success")
        .expect("note exists");
    assert_eq!(note.title, "Home");
    assert_eq!(note.relative_path, "Home");
    assert_eq!(note.content, "hello");

    let missing = vault.read_note_by_slug("missing").expect("read success");
    assert!(missing.is_none());
}

#[test]
fn search_finds_title_and_path_matches() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("Projects")).expect("create dir");
    fs::write(dir.path().join("Projects").join("Plan.md"), "alpha").expect("write plan");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let by_title = vault.search("plan", false, 10);
    assert_eq!(by_title.len(), 1);
    assert_eq!(by_title[0].match_kind, "title");

    let by_path = vault.search("projects", false, 10);
    assert_eq!(by_path.len(), 1);
    assert_eq!(by_path[0].match_kind, "path");
}

#[test]
fn search_content_extension_returns_snippet() {
    let dir = tempdir().expect("temp dir");
    fs::write(
        dir.path().join("Home.md"),
        "Line 1\nSecret token here\nLine 3",
    )
    .expect("write note");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let hits = vault.search("token", true, 10);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].match_kind, "content");
    assert_eq!(hits[0].snippet.as_deref(), Some("Secret token here"));
}

#[test]
fn search_returns_empty_for_blank_query_and_zero_limit() {
    let dir = tempdir().expect("temp dir");
    fs::write(dir.path().join("Home.md"), "secret token").expect("write note");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    assert!(vault.search("   ", true, 10).is_empty());
    assert!(vault.search("token", true, 0).is_empty());
}

#[test]
fn search_content_snippet_truncates_long_lines() {
    let dir = tempdir().expect("temp dir");
    let long_line = format!("{} token", "A".repeat(220));
    fs::write(dir.path().join("Home.md"), long_line).expect("write note");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let hits = vault.search("token", true, 10);
    assert_eq!(hits.len(), 1);
    let snippet = hits[0].snippet.clone().expect("snippet");
    assert!(snippet.ends_with("..."));
    assert_eq!(snippet.chars().count(), 180);
}

#[test]
fn explorer_tree_keeps_notes_with_case_only_path_differences() {
    let dir = tempdir().expect("temp dir");
    fs::write(dir.path().join("Foo.md"), "upper").expect("write upper");
    fs::write(dir.path().join("foo.md"), "lower").expect("write lower");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let tree = vault.explorer_tree();

    assert_eq!(tree.notes.len(), 2);
    assert_eq!(tree.notes[0].title, "Foo");
    assert_eq!(tree.notes[1].title, "foo");
}

#[test]
fn note_links_returns_outgoing_and_backlinks() {
    let dir = tempdir().expect("temp dir");
    fs::write(
        dir.path().join("Home.md"),
        "[[Plan]]\n[[Docs/Guide]]\n![[Image.png]]",
    )
    .expect("write home");
    fs::write(dir.path().join("Plan.md"), "[[Home]]").expect("write plan");
    fs::create_dir_all(dir.path().join("Docs")).expect("create docs dir");
    fs::write(dir.path().join("Docs/Guide.md"), "Guide body").expect("write guide");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let home_links = vault.note_links("home").expect("home links");

    assert_eq!(home_links.outgoing.len(), 2);
    assert_eq!(home_links.outgoing[0].slug, "guide");
    assert_eq!(home_links.outgoing[1].slug, "plan");
    assert_eq!(home_links.backlinks.len(), 1);
    assert_eq!(home_links.backlinks[0].slug, "plan");

    let guide_links = vault.note_links("guide").expect("guide links");
    assert_eq!(guide_links.backlinks.len(), 1);
    assert_eq!(guide_links.backlinks[0].slug, "home");
    assert!(vault.note_links("missing").is_none());
}

#[test]
fn note_links_ignore_wikilinks_in_fenced_and_inline_code() {
    let dir = tempdir().expect("temp dir");
    fs::write(
        dir.path().join("Home.md"),
        [
            "```md",
            "[[Plan]]",
            "```",
            "",
            "Inline `[[Plan]]` sample.",
            "",
            "Real [[Guide]] link.",
        ]
        .join("\n"),
    )
    .expect("write home");
    fs::write(dir.path().join("Plan.md"), "Plan body").expect("write plan");
    fs::write(dir.path().join("Guide.md"), "Guide body").expect("write guide");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let home_links = vault.note_links("home").expect("home links");

    assert_eq!(home_links.outgoing.len(), 1);
    assert_eq!(home_links.outgoing[0].slug, "guide");

    let plan_links = vault.note_links("plan").expect("plan links");
    assert!(plan_links.backlinks.is_empty());
}

#[test]
fn note_links_dedup_targets_and_ignore_self_references() {
    let dir = tempdir().expect("temp dir");
    fs::write(
        dir.path().join("Home.md"),
        "[[Plan]]\n[[Plan#Heading]]\n[[Plan^block]]\n[[Home]]",
    )
    .expect("write home");
    fs::write(dir.path().join("Plan.md"), "plan").expect("write plan");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    let links = vault.note_links("home").expect("home links");
    assert_eq!(links.outgoing.len(), 1);
    assert_eq!(links.outgoing[0].slug, "plan");
}

#[test]
fn read_note_by_slug_surfaces_io_error_for_deleted_file() {
    let dir = tempdir().expect("temp dir");
    let note_path = dir.path().join("Home.md");
    fs::write(&note_path, "hello").expect("write note");

    let vault = VaultIndex::build(dir.path()).expect("build vault");
    fs::remove_file(note_path).expect("remove note");

    let result = vault.read_note_by_slug("home");
    assert!(result.is_err());
}

#[test]
fn build_assigns_layers_and_skips_noise() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("sources")).expect("dirs");
    fs::create_dir_all(dir.path().join("wiki")).expect("dirs");
    fs::create_dir_all(dir.path().join(".obsidian")).expect("dirs");
    fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
    fs::write(dir.path().join("sources/Clipping.md"), "# Clipping").expect("note");
    fs::write(dir.path().join("wiki/Topic.md"), "# Topic").expect("note");
    fs::write(dir.path().join(".obsidian/Plugin Notes.md"), "# Noise").expect("note");
    fs::write(dir.path().join("wiki/Scratch.tmp"), "ignored").expect("tmp");

    let index = VaultIndex::build(dir.path()).expect("index");

    let layer_of = |title: &str| {
        index
            .by_slug
            .values()
            .find(|entry| entry.title == title)
            .map(|entry| entry.layer.clone())
    };

    assert_eq!(layer_of("Clipping"), Some(Some("sources".to_string())));
    assert_eq!(layer_of("Topic"), Some(None));
    assert_eq!(layer_of("Plugin Notes"), None, "noise must not be indexed");
    assert_eq!(index.layers.layer_names(), vec!["sources".to_string()]);
}

#[test]
fn build_gives_the_unsuffixed_slug_to_the_default_surface() {
    // A compiled page named after the source it compiles is the normal case in
    // a layered vault, and `sources/` sorts before `wiki/`. Without precedence
    // every [[Melatonin]] would resolve to the clipping.
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("sources")).expect("dirs");
    fs::create_dir_all(dir.path().join("wiki")).expect("dirs");
    fs::write(dir.path().join("sources/.hatchdoor-layer"), "sources").expect("marker");
    fs::write(dir.path().join("sources/Melatonin.md"), "# Source").expect("note");
    fs::write(dir.path().join("wiki/Melatonin.md"), "# Compiled").expect("note");

    let index = VaultIndex::build(dir.path()).expect("index");

    let compiled = index.find_by_slug("melatonin").expect("slug melatonin");
    assert_eq!(compiled.relative_path, "wiki/Melatonin");
    assert_eq!(compiled.layer, None);

    let source = index.find_by_slug("melatonin-2").expect("slug melatonin-2");
    assert_eq!(source.relative_path, "sources/Melatonin");
    assert_eq!(source.layer.as_deref(), Some("sources"));

    assert_eq!(
        index.resolve_wikilink("Melatonin").map(|e| e.slug.as_str()),
        Some("melatonin")
    );
}

#[test]
fn seeding_is_not_suppressed_by_noise_only_markdown() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join(".obsidian")).expect("dirs");
    fs::write(dir.path().join(".obsidian/Plugin Notes.md"), "# Noise").expect("note");

    // The vault has no real content, so the starter vault must still be written.
    let seeded =
        crate::vault::seed_empty_vault(dir.path(), &crate::vault::ExcludeMatcher::default())
            .expect("seed");
    assert!(seeded, "noise-only markdown must not count as content");
}

#[test]
fn build_with_config_honours_user_supplied_exclude_patterns() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("drafts")).expect("dirs");
    fs::create_dir_all(dir.path().join("wiki")).expect("dirs");
    fs::write(dir.path().join("drafts/Scratch.md"), "# Scratch").expect("note");
    fs::write(dir.path().join("wiki/Keep.md"), "# Keep").expect("note");

    let config = VaultScanConfig {
        exclude: ExcludeMatcher::new(&["drafts/".to_string()]).expect("valid pattern"),
    };

    let index = VaultIndex::build_with_config(dir.path(), &config).expect("index");

    assert!(
        index.find_by_slug("keep").is_some(),
        "Keep should be indexed"
    );
    assert!(
        index.find_by_slug("scratch").is_none(),
        "Scratch should be excluded"
    );
}

#[test]
fn resolve_asset_finds_a_bare_filename_anywhere_in_the_vault() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("97_Notes")).expect("dirs");
    fs::create_dir_all(dir.path().join("98_Attachments")).expect("dirs");
    fs::write(
        dir.path().join("97_Notes/Some note.md"),
        "![[Some document.pdf]]",
    )
    .expect("note");
    fs::write(dir.path().join("98_Attachments/Some document.pdf"), b"pdf").expect("asset");

    let index = VaultIndex::build(dir.path()).expect("index");

    assert_eq!(
        index.resolve_asset("Some document.pdf", "97_Notes"),
        Some("98_Attachments/Some document.pdf")
    );
}

#[test]
fn resolve_asset_prefers_the_notes_own_folder_over_a_distant_namesake() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("projects")).expect("dirs");
    fs::create_dir_all(dir.path().join("98_Attachments")).expect("dirs");
    fs::write(dir.path().join("projects/diagram.png"), b"near").expect("asset");
    fs::write(dir.path().join("98_Attachments/diagram.png"), b"far").expect("asset");

    let index = VaultIndex::build(dir.path()).expect("index");

    assert_eq!(
        index.resolve_asset("diagram.png", "projects"),
        Some("projects/diagram.png")
    );
    assert_eq!(
        index.resolve_asset("diagram.png", ""),
        Some("98_Attachments/diagram.png")
    );
}

#[test]
fn resolve_asset_reads_a_leading_slash_as_vault_root_relative() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("a/b/98_Attachments")).expect("dirs");
    fs::create_dir_all(dir.path().join("98_Attachments")).expect("dirs");
    fs::write(dir.path().join("98_Attachments/plan.pdf"), b"root").expect("asset");
    // A namesake the note-relative reading would reach first, so the assertion
    // fails if the leading slash is not honoured.
    fs::write(dir.path().join("a/b/98_Attachments/plan.pdf"), b"near").expect("asset");

    let index = VaultIndex::build(dir.path()).expect("index");

    assert_eq!(
        index.resolve_asset("/98_Attachments/plan.pdf", "a/b"),
        Some("98_Attachments/plan.pdf")
    );
}

#[test]
fn resolve_asset_still_honours_relative_paths_written_for_hatchdoor() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("97_Notes/inline")).expect("dirs");
    fs::create_dir_all(dir.path().join("inline")).expect("dirs");
    fs::create_dir_all(dir.path().join("98_Attachments")).expect("dirs");
    fs::write(dir.path().join("98_Attachments/shot.png"), b"png").expect("asset");
    // Same relative path from the note's folder and from the Vault root, so the
    // note's own folder has to win for the first assertion to hold.
    fs::write(dir.path().join("97_Notes/inline/near.png"), b"near").expect("asset");
    fs::write(dir.path().join("inline/near.png"), b"root").expect("asset");

    let index = VaultIndex::build(dir.path()).expect("index");

    assert_eq!(
        index.resolve_asset("inline/near.png", "97_Notes"),
        Some("97_Notes/inline/near.png")
    );
    assert_eq!(
        index.resolve_asset("../98_Attachments/shot.png", "97_Notes"),
        Some("98_Attachments/shot.png")
    );
    assert_eq!(
        index.resolve_asset("98_Attachments/shot.png", "97_Notes"),
        Some("98_Attachments/shot.png")
    );
}

#[test]
fn resolve_asset_rejects_targets_that_escape_the_vault_root() {
    let dir = tempdir().expect("temp dir");
    fs::create_dir_all(dir.path().join("notes")).expect("dirs");
    fs::write(dir.path().join("notes/secret.png"), b"png").expect("asset");

    let index = VaultIndex::build(dir.path()).expect("index");

    assert_eq!(index.resolve_asset("../../notes/secret.png", "notes"), None);
}

#[test]
fn resolve_asset_returns_none_for_an_absent_or_unservable_target() {
    let dir = tempdir().expect("temp dir");
    fs::write(dir.path().join("notes.txt"), b"text").expect("file");

    let index = VaultIndex::build(dir.path()).expect("index");

    assert_eq!(index.resolve_asset("missing.png", ""), None);
    assert_eq!(index.resolve_asset("notes.txt", ""), None);
}

#[test]
fn resolve_asset_ignores_anchors_and_query_suffixes_on_the_target() {
    let dir = tempdir().expect("temp dir");
    fs::write(dir.path().join("plan.pdf"), b"pdf").expect("asset");

    let index = VaultIndex::build(dir.path()).expect("index");

    assert_eq!(index.resolve_asset("plan.pdf#page=3", ""), Some("plan.pdf"));
}
