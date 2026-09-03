use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::*;
use crate::cache::parse::content_hash;
use crate::vault::types::{VaultIndex, VaultScanConfig};

fn build(root: &Path) -> VaultIndex {
    VaultIndex::build(root).expect("build index")
}

fn build_catalog(root: &Path) -> VaultIndex {
    VaultIndex::build_catalog_with_config(root, &VaultScanConfig::default()).expect("build catalog")
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
    let catalog = build(root);
    assert!(matches!(
        create_note(root, "../Escape.md", "no", false, &catalog),
        Err(WriteError::InvalidInput(_))
    ));
    create_note(root, "Projects/New", "# New", false, &catalog).expect("create");
    assert_eq!(
        fs::read_to_string(root.join("Projects/New.md")).expect("read"),
        "# New\n"
    );
}

#[test]
fn create_note_computes_its_slug_from_a_metadata_only_catalog() {
    // issue #101: the write API must fill in a create response's slug from
    // the pre-write catalog it already fetched, not a second full index
    // rescan after the write. A metadata-only catalog build never reads a
    // note's *content* (no `build_link_graph` pass) — proving create_note's
    // slug computation works from one anyway is proof it never needed the
    // content-reading pass that made the old post-write rescan expensive.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Existing.md"), "# Existing\n[[Existing]]").expect("write");
    let catalog = build_catalog(root);
    assert!(
        catalog.outgoing_by_slug.is_empty() && catalog.backlinks_by_slug.is_empty(),
        "a catalog build must not populate the wikilink graph"
    );

    let outcome =
        create_note(root, "Projects/New Note", "# New\n", false, &catalog).expect("create");
    assert_eq!(outcome.slug.as_deref(), Some("new-note"));
    assert_eq!(outcome.relative_path.as_deref(), Some("Projects/New Note"));
}

#[test]
fn create_note_disambiguates_a_slug_collision_against_the_pre_write_catalog() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Home.md"), "# Home").expect("write");
    let catalog = build_catalog(root);

    let outcome =
        create_note(root, "Other/Home", "# Other Home\n", false, &catalog).expect("create");
    assert_eq!(outcome.slug.as_deref(), Some("home-2"));
}

#[test]
fn move_or_rename_note_keeps_its_own_slug_when_the_new_title_slugifies_to_the_same_value() {
    // A rename that only changes case ("Home" -> "home") slugifies to the
    // same value as the note's own pre-existing slug. The note's own entry
    // is still sitting in the pre-write catalog's `by_slug` under that slug;
    // without excluding it from the collision check this would wrongly
    // disambiguate to "home-2" against itself.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Home.md"), "# Home").expect("write");
    let index = build(root);
    let entry = index.find_by_slug("home").expect("home");

    let outcome = move_or_rename_note(root, &index, entry, "home.md", &content_hash("# Home"))
        .expect("rename");
    assert_eq!(outcome.slug.as_deref(), Some("home"));
}

#[test]
fn move_or_rename_note_disambiguates_a_slug_collision_against_a_different_note() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Home.md"), "# Home").expect("write");
    fs::create_dir_all(root.join("Projects")).expect("mkdir");
    fs::write(root.join("Projects/Other.md"), "# Other").expect("write");
    let index = build(root);
    let entry = index.find_by_slug("other").expect("other");

    // "Home@" slugifies to "home", the same slug already held by the
    // *different*, still-present "Home.md" note. "Home@.md" sorts *after*
    // "Home.md" as an extension-bearing path (`@` 0x40 > `.` 0x2E — verified
    // with `PathBuf::from("Home@.md").cmp(&PathBuf::from("Home.md"))` ==
    // `Greater`), so in true build order "Home.md" is processed first and
    // keeps "home": a genuine collision that must still disambiguate the
    // moved note, unlike the self-collision case above.
    let outcome = move_or_rename_note(root, &index, entry, "Home@.md", &content_hash("# Other"))
        .expect("rename");
    assert_eq!(outcome.slug.as_deref(), Some("home-2"));

    // Ground truth: a real rebuild of the resulting vault state must agree,
    // not just this function's own self-consistency.
    let rebuilt = build(root);
    assert_eq!(
        rebuilt
            .find_by_slug("home")
            .map(|entry| entry.relative_path.as_str()),
        Some("Home")
    );
    assert_eq!(
        rebuilt
            .find_by_slug("home-2")
            .map(|entry| entry.relative_path.as_str()),
        Some("Home@")
    );
}

#[test]
fn move_or_rename_note_wins_a_slug_collision_when_it_sorts_before_the_existing_note() {
    // Regression test: a prior version of `slug_priority` compared
    // extension-*stripped* paths ("Home!!" > "Home" as strings/paths) instead
    // of the extension-*bearing* paths the real build's `markdown_paths.sort()`
    // actually sorts ("Home!!.md" < "Home.md", since `!` 0x21 sorts before
    // `.` 0x2E — verified with
    // `PathBuf::from("Home!!.md").cmp(&PathBuf::from("Home.md"))` == `Less`).
    // That reversed which note wins: the moved note must claim "home" here,
    // not the pre-existing "Home.md".
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Home.md"), "# Home").expect("write");
    fs::create_dir_all(root.join("Projects")).expect("mkdir");
    fs::write(root.join("Projects/Other.md"), "# Other").expect("write");
    let index = build(root);
    let entry = index.find_by_slug("other").expect("other");

    let outcome = move_or_rename_note(root, &index, entry, "Home!!.md", &content_hash("# Other"))
        .expect("rename");
    assert_eq!(
        outcome.slug.as_deref(),
        Some("home"),
        "\"Home!!.md\" sorts before \"Home.md\" as an extension-bearing path, so it wins the slug"
    );

    // Ground truth: a real rebuild of the resulting vault state must agree —
    // this is the assertion that would have failed against the old,
    // extension-stripped comparison (which predicted the opposite winner).
    let rebuilt = build(root);
    assert_eq!(
        rebuilt
            .find_by_slug("home")
            .map(|entry| entry.relative_path.as_str()),
        Some("Home!!")
    );
    assert_eq!(
        rebuilt
            .find_by_slug("home-2")
            .map(|entry| entry.relative_path.as_str()),
        Some("Home")
    );
}

#[test]
fn create_note_claims_a_contested_slug_ahead_of_an_existing_layered_note() {
    // A plain occupancy check against the pre-write catalog would see "home"
    // as already taken by the layered note and bump the new note to
    // "home-2". A true full rebuild processes default-surface notes before
    // layered ones on a title collision (`vault/index.rs`'s
    // `sort_by_cached_key(is_layered)`, "default-surface notes claim their
    // slugs first"), so the new default-surface note must claim "home"
    // outright, matching what a real rebuild would assign it.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("sources")).expect("mkdir");
    fs::write(root.join("sources/.hatchdoor-layer"), "name: sources\n").expect("marker");
    fs::write(root.join("sources/Home.md"), "# Home").expect("write");
    let catalog = build_catalog(root);
    assert_eq!(
        catalog
            .find_by_slug("home")
            .map(|entry| entry.relative_path.as_str()),
        Some("sources/Home"),
        "the only existing note holds \"home\" uncontested before the write"
    );

    let outcome = create_note(root, "Home", "# Root Home\n", false, &catalog).expect("create");
    assert_eq!(
        outcome.slug.as_deref(),
        Some("home"),
        "a new default-surface note must claim the contested slug ahead of an existing layered one"
    );
}

#[test]
fn move_or_rename_note_claims_a_contested_slug_ahead_of_an_existing_layered_note() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("sources")).expect("mkdir");
    fs::write(root.join("sources/.hatchdoor-layer"), "name: sources\n").expect("marker");
    fs::write(root.join("sources/Home.md"), "# Home").expect("write");
    fs::create_dir_all(root.join("Projects")).expect("mkdir");
    fs::write(root.join("Projects/Other.md"), "# Other").expect("write");
    let index = build(root);
    let entry = index.find_by_slug("other").expect("other");
    assert_eq!(
        index
            .find_by_slug("home")
            .map(|entry| entry.relative_path.as_str()),
        Some("sources/Home"),
        "the only existing note holds \"home\" uncontested before the write"
    );

    // Moving "Other" to the vault root makes it a default-surface note
    // contesting the layered note's "home" slug; the moved note must win.
    let outcome = move_or_rename_note(root, &index, entry, "Home.md", &content_hash("# Other"))
        .expect("rename");
    assert_eq!(
        outcome.slug.as_deref(),
        Some("home"),
        "a note moved onto the default surface must claim the contested slug ahead of an existing layered one"
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

fn frontmatter_entry(
    root: &Path,
    content: &str,
    name: &str,
) -> (VaultIndex, crate::vault::NoteEntry) {
    fs::write(root.join(format!("{name}.md")), content).expect("write note");
    let index = build(root);
    let entry = index
        .find_by_slug(&crate::vault::slugify(name))
        .unwrap_or_else(|| panic!("{} entry", crate::vault::slugify(name)))
        .clone();
    (index, entry)
}

#[test]
fn update_note_frontmatter_merges_top_level_keys_and_keeps_the_body_byte_for_byte() {
    let tmp = TempDir::new().expect("tempdir");
    let original = "---\ntitle: Home\ntags:\n  - alpha\nnested:\n  keep: me\n---\n\n# Body\nsecret body text\n";
    let (_index, entry) = frontmatter_entry(tmp.path(), original, "Home");
    let mut updates = serde_json::Map::new();
    updates.insert("status".to_string(), serde_json::json!("active"));
    updates.insert(
        "nested".to_string(),
        serde_json::json!({"replaced": "wholesale"}),
    );

    let outcome = update_note_frontmatter(&entry, updates, &content_hash(original))
        .expect("frontmatter update");

    let updated = fs::read_to_string(entry.path).expect("read");
    // serde_json maps serialize deterministically (keys sorted); nested values are replaced wholesale.
    assert_eq!(
        updated,
        "---\nnested:\n  replaced: wholesale\nstatus: active\ntags:\n- alpha\ntitle: Home\n---\n\n# Body\nsecret body text\n"
    );
    assert!(updated.ends_with("secret body text\n"), "body is unchanged");
    assert_eq!(
        outcome.content_hash.as_deref(),
        Some(content_hash(&updated).as_str())
    );
    assert_eq!(outcome.slug.as_deref(), Some("home"));
    assert_eq!(outcome.relative_path.as_deref(), Some("Home"));
}

#[test]
fn update_note_frontmatter_null_deletes_and_unmentioned_keys_survive() {
    let tmp = TempDir::new().expect("tempdir");
    let original = "---\nkeep: yes\ndrop: me\n---\nbody stays\n";
    let (_index, entry) = frontmatter_entry(tmp.path(), original, "Home");
    let mut updates = serde_json::Map::new();
    updates.insert("drop".to_string(), serde_json::Value::Null);
    updates.insert("added".to_string(), serde_json::json!(2));

    update_note_frontmatter(&entry, updates, &content_hash(original)).expect("update");

    let updated = fs::read_to_string(&entry.path).expect("read");
    assert!(updated.contains("added: 2"));
    assert!(updated.contains("keep: yes"));
    assert!(
        !updated.contains("drop"),
        "explicit null deletes the key: {updated}"
    );
    assert!(updated.ends_with("body stays\n"));
}

#[test]
fn update_note_frontmatter_creates_a_block_on_a_note_without_one() {
    let tmp = TempDir::new().expect("tempdir");
    let original = "# Body only\nplain body\n";
    let (_index, entry) = frontmatter_entry(tmp.path(), original, "Home");
    let mut updates = serde_json::Map::new();
    updates.insert("tags".to_string(), serde_json::json!(["one", "two"]));

    update_note_frontmatter(&entry, updates, &content_hash(original)).expect("update");

    let updated = fs::read_to_string(&entry.path).expect("read");
    assert!(
        updated.starts_with("---\ntags:\n"),
        "frontmatter block created: {updated}"
    );
    assert!(
        updated.ends_with("# Body only\nplain body\n"),
        "original content preserved as the body: {updated}"
    );
}

#[test]
fn update_note_frontmatter_strips_the_block_when_the_last_keys_are_deleted() {
    let tmp = TempDir::new().expect("tempdir");
    let original = "---\nonly: key\n---\n\nremaining body\n";
    let (_index, entry) = frontmatter_entry(tmp.path(), original, "Home");
    let mut updates = serde_json::Map::new();
    updates.insert("only".to_string(), serde_json::Value::Null);

    update_note_frontmatter(&entry, updates, &content_hash(original)).expect("update");

    let updated = fs::read_to_string(&entry.path).expect("read");
    assert!(
        !updated.contains("---"),
        "empty frontmatter block is removed entirely: {updated:?}"
    );
    assert!(updated.ends_with("remaining body\n"));
}

#[test]
fn update_note_frontmatter_rejects_empty_updates_and_all_null_creation() {
    let tmp = TempDir::new().expect("tempdir");
    let plain = "just a body\n";
    let (_index, entry) = frontmatter_entry(tmp.path(), plain, "Home");
    assert!(matches!(
        update_note_frontmatter(&entry, serde_json::Map::new(), &content_hash(plain)),
        Err(WriteError::InvalidInput(_))
    ));

    let mut nulls = serde_json::Map::new();
    nulls.insert("ghost".to_string(), serde_json::Value::Null);
    assert!(
        matches!(
            update_note_frontmatter(&entry, nulls, &content_hash(plain)),
            Err(WriteError::InvalidInput(_))
        ),
        "creating a frontmatter block from deletes-only is refused"
    );
}

#[test]
fn update_note_frontmatter_warns_when_duplicate_keys_are_collapsed() {
    let tmp = TempDir::new().expect("tempdir");
    // serde_yaml_ng parses this last-wins, so `keep: second` silently replaces
    // `keep: first` — surfaced as a quality warning like sibling primitives.
    let original = "---\nkeep: first\nkeep: second\n---\nbody\n";
    let (_index, entry) = frontmatter_entry(tmp.path(), original, "Home");
    let mut updates = serde_json::Map::new();
    updates.insert("added".to_string(), serde_json::json!(1));

    let outcome =
        update_note_frontmatter(&entry, updates, &content_hash(original)).expect("update");

    assert!(
        outcome
            .quality_warnings
            .iter()
            .any(|warning| warning.contains("duplicate key")),
        "duplicate-key collapse is warned about: {:?}",
        outcome.quality_warnings
    );
}

#[test]
fn update_note_frontmatter_round_trips_edge_case_values_without_changing_their_types() {
    // A merge re-serializes the whole block, so every untouched value makes a
    // parse -> serialize -> parse trip. Style may change (that is documented);
    // the value and its type may not, or a note gains or loses meaning behind
    // its author's back.
    let tmp = TempDir::new().expect("tempdir");
    let original = concat!(
        "---\n",
        "quoted_number: \"123\"\n",
        "bare_number: 123\n",
        "quoted_bool: \"true\"\n",
        "float: 1.50\n",
        "date: 2026-08-29\n",
        "unicode: \"réseau — 日本語 🌱\"\n",
        "colon_in_value: \"key: value\"\n",
        "multiline: |\n",
        "  first line\n",
        "  second line\n",
        "nested:\n",
        "  keep: me\n",
        "  deeper:\n",
        "    count: 2\n",
        "    label: \"2\"\n",
        // tags and aliases are the two keys the parser lifts out of
        // `properties`, so they need asserting separately from the loop below.
        "tags:\n",
        "  - alpha\n",
        "aliases:\n",
        "  - Alt Name\n",
        "---\n",
        "\n",
        "body text\n",
    );
    let (_index, entry) = frontmatter_entry(tmp.path(), original, "Home");
    let before = crate::cache::parse::parse_frontmatter_metadata(original)
        .expect("original frontmatter parses");

    let mut updates = serde_json::Map::new();
    updates.insert("status".to_string(), serde_json::json!("active"));
    update_note_frontmatter(&entry, updates, &content_hash(original)).expect("frontmatter update");

    let updated = fs::read_to_string(&entry.path).expect("read");
    let after =
        crate::cache::parse::parse_frontmatter_metadata(&updated).expect("updated frontmatter");

    for (key, value) in &before.properties {
        assert_eq!(
            after.properties.get(key),
            Some(value),
            "{key} changed across the merge round trip: {updated}"
        );
    }
    assert_eq!(after.tags, before.tags, "tags survive: {updated}");
    assert_eq!(after.aliases, before.aliases, "aliases survive: {updated}");
    assert_eq!(
        after.properties.get("status"),
        Some(&serde_json::json!("active")),
        "the merged key is present: {updated}"
    );

    // Reparsing with the same crate would hide an emitter change that is
    // stable under its own reader, so pin the bytes it writes as well. This
    // block is what serde_yaml 0.9 emitted for the same input; it is the
    // assertion that would fail if the swap were not behaviour preserving.
    assert_eq!(
        updated,
        concat!(
            "---\n",
            "aliases:\n",
            "- Alt Name\n",
            "bare_number: 123\n",
            "colon_in_value: 'key: value'\n",
            "date: 2026-08-29\n",
            "float: 1.5\n",
            "multiline: |\n",
            "  first line\n",
            "  second line\n",
            "nested:\n",
            "  deeper:\n",
            "    count: 2\n",
            "    label: '2'\n",
            "  keep: me\n",
            "quoted_bool: 'true'\n",
            "quoted_number: '123'\n",
            "status: active\n",
            "tags:\n",
            "- alpha\n",
            "unicode: réseau — 日本語 🌱\n",
            "---\n",
            "\n",
            "body text\n",
        ),
        "emitter output is byte-stable across the crate swap"
    );
}

#[test]
fn update_note_frontmatter_requires_matching_hash() {
    let tmp = TempDir::new().expect("tempdir");
    let original = "---\na: b\n---\nbody\n";
    let (_index, entry) = frontmatter_entry(tmp.path(), original, "Home");
    let mut updates = serde_json::Map::new();
    updates.insert("a".to_string(), serde_json::json!("c"));
    assert!(matches!(
        update_note_frontmatter(&entry, updates.clone(), "fnv1a64:deadbeef"),
        Err(WriteError::Conflict(_))
    ));
    update_note_frontmatter(&entry, updates, &content_hash(original))
        .expect("stale hash rejected, fresh accepted");
}

#[test]
fn update_note_frontmatter_refuses_invalid_existing_yaml_without_touching_it() {
    let tmp = TempDir::new().expect("tempdir");
    let original = "---\ntags: [broken\n---\nbody\n";
    let (_index, entry) = frontmatter_entry(tmp.path(), original, "Home");
    let mut updates = serde_json::Map::new();
    updates.insert("a".to_string(), serde_json::json!(1));
    let error = update_note_frontmatter(&entry, updates, &content_hash(original))
        .expect_err("malformed existing frontmatter must fail loudly");
    assert!(
        matches!(error, WriteError::InvalidInput(_)),
        "got {error:?}"
    );
    assert_eq!(fs::read_to_string(&entry.path).expect("read"), original);
}

#[cfg(unix)]
#[test]
fn attachment_overwrite_rejects_a_symlink_destination_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let vault = TempDir::new().expect("vault");
    let sentinel = vault.path().join("outside.txt");
    fs::write(&sentinel, "do not change").expect("sentinel");
    symlink(&sentinel, vault.path().join("image.png")).expect("attachment link");

    let error = import_attachment_bytes(vault.path(), "image.png", b"new image", 1024, true)
        .expect_err("an overwrite must not follow a symlink destination");

    assert!(matches!(error, WriteError::Conflict(_) | WriteError::Io(_)));
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "do not change");
}

#[test]
fn write_quality_normalizes_safe_markdown_formatting() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let catalog = build(root);
    let outcome =
        create_note(root, "Quality", "# Quality\r\nBody", false, &catalog).expect("create");

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
    let catalog = build(root);
    let outcome = create_note(
        root,
        "Quality",
        "---\ntags: [a]\ntags: [b]\n# Missing close\n",
        false,
        &catalog,
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
    let catalog = build(tmp.path());
    assert!(matches!(
        create_note(tmp.path(), "Bad", "hello\0world", false, &catalog),
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

/// A short JPEG header followed by bytes that are not valid UTF-8, standing in
/// for a real image the way the ASCII `"png"` fixtures never did (#220). Shared
/// with the `fs_ops` tests so both levels exercise the same bytes.
pub(super) const BINARY_ASSET: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0xFE, 0xFF, 0x80,
];

#[test]
fn move_attachment_carries_bytes_that_are_not_valid_utf8() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Media")).expect("media");
    fs::write(root.join("Media/photo.jpg"), BINARY_ASSET).expect("asset");
    fs::write(root.join("Note.md"), "![](Media/photo.jpg)").expect("note");
    let index = build(root);

    move_attachment(root, &index, "Media/photo.jpg", "Archive/photo.jpg").expect("move attachment");

    assert!(!root.join("Media/photo.jpg").exists());
    assert_eq!(
        fs::read(root.join("Archive/photo.jpg")).expect("moved asset"),
        BINARY_ASSET
    );
    assert_eq!(
        fs::read_to_string(root.join("Note.md")).expect("note"),
        "![](Archive/photo.jpg)"
    );
}

#[test]
fn rename_attachment_keeps_bytes_that_are_not_valid_utf8() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Media")).expect("media");
    fs::write(root.join("Media/photo.jpg"), BINARY_ASSET).expect("asset");
    fs::write(root.join("Note.md"), "![](Media/photo.jpg)").expect("note");
    let index = build(root);

    rename_attachment(root, &index, "Media/photo.jpg", "cover.jpg").expect("rename attachment");

    assert!(!root.join("Media/photo.jpg").exists());
    assert_eq!(
        fs::read(root.join("Media/cover.jpg")).expect("renamed asset"),
        BINARY_ASSET
    );
    assert_eq!(
        fs::read_to_string(root.join("Note.md")).expect("note"),
        "![](Media/cover.jpg)"
    );
}

#[test]
fn delete_attachment_trashes_bytes_that_are_not_valid_utf8() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Media")).expect("media");
    fs::write(root.join("Media/photo.jpg"), BINARY_ASSET).expect("asset");
    fs::write(root.join("Note.md"), "![](Media/photo.jpg)").expect("note");
    let index = build(root);

    let outcome = delete_attachment(root, &index, "Media/photo.jpg").expect("delete attachment");

    let trashed = outcome.trashed_path.expect("trash path");
    assert!(!root.join("Media/photo.jpg").exists());
    assert_eq!(
        fs::read(root.join(&trashed)).expect("trashed asset"),
        BINARY_ASSET
    );
}

#[test]
fn move_note_carries_a_sibling_asset_whose_bytes_are_not_valid_utf8() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Notes")).expect("notes");
    fs::write(root.join("Notes/Target.md"), "body\n![](photo.jpg)").expect("target");
    fs::write(root.join("Notes/photo.jpg"), BINARY_ASSET).expect("asset");
    fs::write(root.join("Backlink.md"), "![](Notes/photo.jpg) [[Target]]").expect("backlink");
    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");

    let outcome = move_or_rename_note(
        root,
        &index,
        entry,
        "Archive/Renamed.md",
        &content_hash("body\n![](photo.jpg)"),
    )
    .expect("move a note holding a binary attachment");

    assert_eq!(outcome.moved_assets, 1);
    assert!(root.join("Archive/Renamed.md").exists());
    assert_eq!(
        fs::read(root.join("Archive/photo.jpg")).expect("moved asset"),
        BINARY_ASSET
    );
    let backlink = fs::read_to_string(root.join("Backlink.md")).expect("backlink");
    assert!(backlink.contains("![](Archive/photo.jpg)"));
    assert!(backlink.contains("[[Archive/Renamed]]"));
}

/// A note in `Notes/` holding one attachment whose bytes are not valid UTF-8.
fn note_with_a_binary_asset(root: &Path) {
    fs::create_dir_all(root.join("Notes")).expect("notes");
    fs::write(root.join("Notes/Target.md"), BINARY_ASSET_NOTE).expect("target");
    fs::write(root.join("Notes/photo.jpg"), BINARY_ASSET).expect("asset");
}

const BINARY_ASSET_NOTE: &str = "body ![](photo.jpg)";

#[test]
fn archive_note_carries_an_asset_whose_bytes_are_not_valid_utf8() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    note_with_a_binary_asset(root);
    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");

    archive_note(
        root,
        &index,
        entry,
        "90-archive/",
        &content_hash(BINARY_ASSET_NOTE),
    )
    .expect("archive a note holding a binary attachment");

    assert!(!root.join("Notes/photo.jpg").exists());
    assert_eq!(
        fs::read(root.join("90-archive/photo.jpg")).expect("archived asset"),
        BINARY_ASSET
    );
}

#[test]
fn delete_note_trashes_an_asset_whose_bytes_are_not_valid_utf8() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    note_with_a_binary_asset(root);
    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");

    delete_note(root, &index, entry, &content_hash(BINARY_ASSET_NOTE))
        .expect("delete a note holding a binary attachment");

    assert!(!root.join("Notes/photo.jpg").exists());
    assert_eq!(
        fs::read(root.join(".hatchdoor-trash/photo.jpg")).expect("trashed asset"),
        BINARY_ASSET
    );
}

#[test]
fn move_note_reports_a_stale_hash_as_a_changed_note_not_an_unsafe_source() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Notes")).expect("notes");
    fs::write(root.join("Notes/Target.md"), "body").expect("target");
    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");

    let error = move_or_rename_note(
        root,
        &index,
        entry,
        "Archive/Target.md",
        &content_hash("something else"),
    )
    .expect_err("a stale hash must refuse the move");

    let WriteError::Conflict(message) = error else {
        panic!("expected a conflict");
    };
    assert!(
        message.contains("note changed since it was read"),
        "the optimistic-concurrency failure must stay distinct from an unsafe source: {message}"
    );
    assert!(root.join("Notes/Target.md").exists());
    assert!(!root.join("Archive/Target.md").exists());
}

#[test]
fn note_move_compensates_failures_after_every_completed_phase() {
    use super::types::MutationPhase;

    for failed_phase in [
        MutationPhase::Note,
        MutationPhase::Asset,
        MutationPhase::Rewrite,
    ] {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        fs::create_dir_all(root.join("Notes")).expect("notes");
        fs::write(root.join("Notes/Target.md"), "body\n![](image.png)").expect("target");
        fs::write(root.join("Notes/image.png"), "original asset").expect("asset");
        fs::write(
            root.join("Backlink.md"),
            "See [[Target]] and ![](Notes/image.png)",
        )
        .expect("backlink");
        let index = build(root);
        let entry = index.find_by_slug("target").expect("target");

        let error = super::notes::move_or_rename_note_with_failure(
            root,
            &index,
            entry,
            "Archive/Target.md",
            &content_hash("body\n![](image.png)"),
            |completed| {
                if completed == failed_phase {
                    Err(WriteError::Io(format!(
                        "injected failure after {completed:?} phase"
                    )))
                } else {
                    Ok(())
                }
            },
        )
        .expect_err("injected phase failure must abort the mutation");

        let WriteError::Io(message) = error else {
            panic!("expected injected I/O error");
        };
        assert!(message.contains("rollback succeeded"));
        assert_eq!(
            fs::read_to_string(root.join("Notes/Target.md")).expect("restored note"),
            "body\n![](image.png)"
        );
        assert_eq!(
            fs::read_to_string(root.join("Notes/image.png")).expect("restored asset"),
            "original asset"
        );
        assert_eq!(
            fs::read_to_string(root.join("Backlink.md")).expect("restored backlink"),
            "See [[Target]] and ![](Notes/image.png)"
        );
        assert!(!root.join("Archive/Target.md").exists());
        assert!(!root.join("Archive/image.png").exists());
    }
}

#[test]
fn attachment_move_compensates_failures_after_every_completed_phase() {
    use super::types::MutationPhase;

    for failed_phase in [MutationPhase::Asset, MutationPhase::Rewrite] {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        fs::create_dir_all(root.join("Media")).expect("media");
        fs::write(root.join("Media/image.png"), "original asset").expect("asset");
        fs::write(root.join("Note.md"), "![](Media/image.png)").expect("note");
        let index = build(root);

        let error = super::attachments::move_attachment_with_failure(
            root,
            &index,
            "Media/image.png",
            "Archive/image.png",
            |completed| {
                if completed == failed_phase {
                    Err(WriteError::Io(format!(
                        "injected failure after {completed:?} phase"
                    )))
                } else {
                    Ok(())
                }
            },
        )
        .expect_err("injected phase failure must abort the mutation");

        let WriteError::Io(message) = error else {
            panic!("expected injected I/O error");
        };
        assert!(message.contains("rollback succeeded"));
        assert_eq!(
            fs::read_to_string(root.join("Media/image.png")).expect("restored asset"),
            "original asset"
        );
        assert_eq!(
            fs::read_to_string(root.join("Note.md")).expect("restored reference"),
            "![](Media/image.png)"
        );
        assert!(!root.join("Archive/image.png").exists());
    }
}

#[test]
fn note_delete_compensates_a_failure_after_rewrites() {
    use super::types::MutationPhase;

    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("Target.md"), "body ![](asset.pdf)").expect("target");
    fs::write(root.join("asset.pdf"), "original asset").expect("asset");
    fs::write(
        root.join("Backlink.md"),
        "before [[Target]] after ![](asset.pdf)",
    )
    .expect("backlink");
    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");

    let error = super::notes::delete_note_with_failure(
        root,
        &index,
        entry,
        &content_hash("body ![](asset.pdf)"),
        |completed| {
            if completed == MutationPhase::Rewrite {
                Err(WriteError::Io(
                    "injected failure after delete rewrites".to_string(),
                ))
            } else {
                Ok(())
            }
        },
    )
    .expect_err("late delete failure must abort");

    let WriteError::Io(message) = error else {
        panic!("expected injected I/O error");
    };
    assert!(message.contains("rollback succeeded"));
    assert_eq!(
        fs::read_to_string(root.join("Target.md")).expect("restored note"),
        "body ![](asset.pdf)"
    );
    assert_eq!(
        fs::read_to_string(root.join("asset.pdf")).expect("restored asset"),
        "original asset"
    );
    assert_eq!(
        fs::read_to_string(root.join("Backlink.md")).expect("restored backlink"),
        "before [[Target]] after ![](asset.pdf)"
    );
    assert!(!root.join(".hatchdoor-trash/Target.md").exists());
    assert!(!root.join(".hatchdoor-trash/asset.pdf").exists());
}

#[test]
fn compensation_failure_surfaces_bounded_recovery_details_and_continues_rollback() {
    use super::types::MutationPhase;

    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("Notes")).expect("notes");
    fs::write(root.join("Notes/Target.md"), "body\n![](image.png)").expect("target");
    fs::write(root.join("Notes/image.png"), "original asset").expect("asset");
    fs::write(
        root.join("Backlink.md"),
        "See [[Target]] and ![](Notes/image.png)",
    )
    .expect("backlink");
    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");
    let backlink = root.join("Backlink.md");

    let error = super::notes::move_or_rename_note_with_failure(
        root,
        &index,
        entry,
        "Archive/Target.md",
        &content_hash("body\n![](image.png)"),
        |completed| {
            if completed == MutationPhase::Rewrite {
                fs::write(&backlink, "manual concurrent edit")
                    .expect("simulate external replacement");
                Err(WriteError::Io(
                    "injected failure after rewrite phase".to_string(),
                ))
            } else {
                Ok(())
            }
        },
    )
    .expect_err("incomplete compensation must be reported");

    let WriteError::Io(message) = error else {
        panic!("expected recovery-required I/O error");
    };
    assert!(message.contains("recovery required"));
    assert!(message.contains("Backlink.md"));
    assert!(message.contains("restore rewritten note"));
    assert!(
        !message.contains(&root.display().to_string()),
        "adapter-visible details must not contain the absolute vault root: {message}"
    );

    assert_eq!(
        fs::read_to_string(root.join("Notes/Target.md")).expect("restored note"),
        "body\n![](image.png)"
    );
    assert_eq!(
        fs::read_to_string(root.join("Notes/image.png")).expect("restored asset"),
        "original asset"
    );
    assert!(!root.join("Archive/Target.md").exists());
    assert!(!root.join("Archive/image.png").exists());
    assert_eq!(
        fs::read_to_string(backlink).expect("manual edit preserved"),
        "manual concurrent edit",
        "compensation must not overwrite a concurrent manual edit"
    );
}

/// A vault laid out the way Obsidian's default "keep attachments in one place"
/// setting produces: a shared `_system/` folder holding the image, and a note
/// elsewhere embedding it through a parent-relative reference (#225).
fn vault_with_a_shared_attachments_folder(root: &Path) -> String {
    let body = "# B\n![](../_system/image.png)\n";
    fs::create_dir_all(root.join("_system")).expect("system dir");
    fs::write(root.join("_system/image.png"), BINARY_ASSET).expect("asset");
    fs::create_dir_all(root.join("folder-x")).expect("folder-x");
    fs::write(root.join("folder-x/B.md"), body).expect("note");
    body.to_string()
}

/// Resolve a note's asset reference the way a Markdown renderer would, so a
/// test asserts the link still points at a real file rather than just matching
/// the string a particular implementation happens to emit.
fn embedded_asset_resolves_to(root: &Path, note_relative_path: &str, asset_relative_path: &str) {
    let note = root.join(note_relative_path);
    let content = fs::read_to_string(&note).expect("read note for reference check");
    let target = content
        .split_once("![](")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(target, _)| target.to_string())
        .unwrap_or_else(|| panic!("note '{note_relative_path}' has no markdown embed: {content}"));
    let resolved = note
        .parent()
        .expect("note parent")
        .join(&target)
        .canonicalize()
        .unwrap_or_else(|error| panic!("embed '{target}' does not resolve: {error}"));
    assert_eq!(
        resolved,
        root.join(asset_relative_path)
            .canonicalize()
            .expect("expected asset"),
        "embed '{target}' in '{note_relative_path}' must resolve to '{asset_relative_path}'"
    );
}

#[test]
fn move_note_leaves_an_asset_outside_its_folder_in_place_and_repoints_its_own_reference() {
    // #225: an asset the note merely references must not be dragged along by
    // an ordinary move, and the move must not fail either. Every destination
    // depth is covered because each computes a different relative reference.
    for target in [
        "folder-z/B.md",
        "folder-x/deeper/B.md",
        "B.md",
        "_system/B.md",
    ] {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path();
        let body = vault_with_a_shared_attachments_folder(root);
        let index = build(root);
        let entry = index.find_by_slug("b").expect("b");

        let outcome = move_or_rename_note(root, &index, entry, target, &content_hash(&body))
            .unwrap_or_else(|error| panic!("move to '{target}' must succeed: {error:?}"));

        assert_eq!(
            outcome.moved_assets, 0,
            "an asset outside the note's folder must not travel to '{target}'"
        );
        assert_eq!(
            fs::read(root.join("_system/image.png")).expect("asset stays put"),
            BINARY_ASSET
        );
        embedded_asset_resolves_to(root, target, "_system/image.png");
    }
}

#[test]
fn archive_note_leaves_an_asset_outside_its_folder_in_place() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let body = vault_with_a_shared_attachments_folder(root);
    let index = build(root);
    let entry = index.find_by_slug("b").expect("b");

    let outcome = archive_note(root, &index, entry, "90-archive/", &content_hash(&body))
        .expect("archive must succeed");

    assert_eq!(outcome.moved_assets, 0);
    assert!(root.join("_system/image.png").exists());
    embedded_asset_resolves_to(root, "90-archive/B.md", "_system/image.png");
}

#[test]
fn delete_note_leaves_an_asset_outside_its_folder_in_place_and_repoints_the_trashed_copy() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let body = vault_with_a_shared_attachments_folder(root);
    let index = build(root);
    let entry = index.find_by_slug("b").expect("b");

    let outcome = delete_note(root, &index, entry, &content_hash(&body)).expect("delete");

    assert_eq!(outcome.moved_assets, 0);
    assert!(root.join("_system/image.png").exists());
    let trashed = format!("{}.md", outcome.trashed_path.expect("trash path"));
    embedded_asset_resolves_to(root, &trashed, "_system/image.png");
}

#[test]
fn a_note_whose_reference_an_earlier_move_rewrote_can_still_be_moved_archived_and_deleted() {
    // The reporter's chain from #225: A and B share a sibling asset, A moves
    // away and takes the asset with it, and B is left pointing over the folder
    // boundary. B must stay fully mutable from there.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let a_body = "# A\n![](image.png)\n";
    let b_body = "# B\n![](image.png)\n";
    fs::create_dir_all(root.join("folder-x")).expect("folder-x");
    fs::write(root.join("folder-x/A.md"), a_body).expect("a");
    fs::write(root.join("folder-x/B.md"), b_body).expect("b");
    fs::write(root.join("folder-x/image.png"), BINARY_ASSET).expect("asset");

    let index = build(root);
    let a = index.find_by_slug("a").expect("a").clone();
    let moved_a = move_or_rename_note(root, &index, &a, "folder-y/A.md", &content_hash(a_body))
        .expect("moving A must still carry its sibling asset");
    assert_eq!(moved_a.moved_assets, 1);
    let rewritten_b = fs::read_to_string(root.join("folder-x/B.md")).expect("b");
    assert!(
        rewritten_b.contains("![](../folder-y/image.png)"),
        "B's reference must be repointed at the moved asset: {rewritten_b}"
    );

    let index = build(root);
    let b = index.find_by_slug("b").expect("b").clone();
    move_or_rename_note(
        root,
        &index,
        &b,
        "folder-z/B.md",
        &content_hash(&rewritten_b),
    )
    .expect("B must be movable after the rewrite");
    embedded_asset_resolves_to(root, "folder-z/B.md", "folder-y/image.png");

    let moved_b = fs::read_to_string(root.join("folder-z/B.md")).expect("b");
    let index = build(root);
    let b = index.find_by_slug("b").expect("b").clone();
    archive_note(root, &index, &b, "90-archive/", &content_hash(&moved_b))
        .expect("B must be archivable");
    embedded_asset_resolves_to(root, "90-archive/B.md", "folder-y/image.png");

    let archived_b = fs::read_to_string(root.join("90-archive/B.md")).expect("b");
    let index = build(root);
    let b = index.find_by_slug("b").expect("b").clone();
    let deleted =
        delete_note(root, &index, &b, &content_hash(&archived_b)).expect("B must be deletable");
    assert!(
        root.join("folder-y/image.png").exists(),
        "deleting B must not trash an asset it only references"
    );
    let trashed = format!("{}.md", deleted.trashed_path.expect("trash path"));
    embedded_asset_resolves_to(root, &trashed, "folder-y/image.png");
}

#[test]
fn move_note_carries_an_asset_held_in_a_subfolder_of_its_own_folder() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let body = "# Target\n![](media/image.png)\n";
    fs::create_dir_all(root.join("Notes/media")).expect("media dir");
    fs::write(root.join("Notes/Target.md"), body).expect("target");
    fs::write(root.join("Notes/media/image.png"), BINARY_ASSET).expect("asset");
    let index = build(root);
    let entry = index.find_by_slug("target").expect("target");

    let outcome = move_or_rename_note(
        root,
        &index,
        entry,
        "Archive/Target.md",
        &content_hash(body),
    )
    .expect("move");

    assert_eq!(outcome.moved_assets, 1);
    assert!(!root.join("Notes/media/image.png").exists());
    assert_eq!(
        fs::read(root.join("Archive/media/image.png")).expect("asset travelled"),
        BINARY_ASSET
    );
    embedded_asset_resolves_to(root, "Archive/Target.md", "Archive/media/image.png");
}

#[test]
fn move_note_refuses_an_asset_reference_that_escapes_the_vault() {
    let tmp = TempDir::new().expect("tempdir");
    let outside = tmp.path().join("outside");
    let root = tmp.path().join("vault");
    fs::create_dir_all(&outside).expect("outside dir");
    fs::create_dir_all(root.join("folder-x/media")).expect("folder-x");
    fs::write(outside.join("image.png"), BINARY_ASSET).expect("outside asset");
    fs::write(root.join("folder-x/media/own.png"), BINARY_ASSET).expect("own asset");
    // The travelling reference comes first, so the refusal has to be reached
    // before its destination folder would otherwise have been created.
    let body = "# B\n![](media/own.png)\n![](../../outside/image.png)\n";
    fs::write(root.join("folder-x/B.md"), body).expect("note");
    let index = build(&root);
    let entry = index.find_by_slug("b").expect("b");

    let error = move_or_rename_note(&root, &index, entry, "folder-z/B.md", &content_hash(body))
        .expect_err("a reference pointing out of the vault must be refused");

    assert!(
        matches!(&error, WriteError::InvalidInput(message) if message.contains("outside the vault")),
        "expected an invalid-input refusal, got {error:?}"
    );
    assert!(
        root.join("folder-x/B.md").exists(),
        "the note must not move"
    );
    assert!(
        !root.join("folder-z").exists(),
        "a refused plan must not create any part of the destination"
    );
    assert!(
        root.join("folder-x/media/own.png").exists(),
        "the note's own asset must not move either"
    );
    assert_eq!(
        fs::read(outside.join("image.png")).expect("outside asset untouched"),
        BINARY_ASSET
    );
}

#[test]
fn move_note_from_the_vault_root_carries_the_assets_it_references() {
    // A note in the Vault root has the whole Vault as its own folder, so by the
    // #225 rule every asset it references is inside that folder and travels.
    // Pinned deliberately: it is the one place where the rule and the "leave a
    // shared attachments folder alone" intent point in different directions,
    // and #225 put changing which assets travel from inside the note's own
    // folder out of scope.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    let body = "# B\n![](_system/image.png)\n";
    fs::create_dir_all(root.join("_system")).expect("system dir");
    fs::write(root.join("_system/image.png"), BINARY_ASSET).expect("asset");
    fs::write(root.join("B.md"), body).expect("note");
    let index = build(root);
    let entry = index.find_by_slug("b").expect("b");

    let outcome = move_or_rename_note(root, &index, entry, "folder-z/B.md", &content_hash(body))
        .expect("move");

    assert_eq!(outcome.moved_assets, 1);
    assert_eq!(
        fs::read(root.join("folder-z/_system/image.png")).expect("asset travelled"),
        BINARY_ASSET
    );
    embedded_asset_resolves_to(root, "folder-z/B.md", "folder-z/_system/image.png");
}

#[test]
fn every_path_a_planned_asset_move_hands_the_filesystem_is_accepted_by_the_move_primitive() {
    // #225's failure was a planned path the filesystem layer refuses: the
    // primitives walk a path one plain name at a time, so a `..` anywhere in a
    // plan makes it unexecutable. Asserting the plan's own paths are free of
    // `..` proves that structurally; driving each of them through the primitive
    // proves it against the thing that actually rejects them.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    fs::create_dir_all(root.join("_system")).expect("system dir");
    fs::write(root.join("_system/shared.png"), BINARY_ASSET).expect("shared asset");
    fs::create_dir_all(root.join("folder-x/media")).expect("media dir");
    fs::write(root.join("folder-x/media/own.png"), BINARY_ASSET).expect("own asset");
    fs::write(
        root.join("folder-x/B.md"),
        "![](../_system/shared.png)\n![](./media/own.png)\n",
    )
    .expect("note");

    let index = build(root);
    let entry = index.find_by_slug("b").expect("b").clone();
    let destination = root.join("deeper/nest/B.md");
    let (moves, _rewrites) =
        super::assets::asset_move_plan(root, &index, &entry, &destination, false, &[])
            .expect("plan");

    assert_eq!(moves.len(), 1, "only the note's own asset travels");
    for asset_move in &moves {
        for path in [&asset_move.source, &asset_move.destination] {
            assert!(
                !path
                    .components()
                    .any(|component| component == std::path::Component::ParentDir),
                "a planned path must be free of '..': {}",
                path.display()
            );
        }
        super::fs_ops::move_file_no_follow(&asset_move.source, &asset_move.destination)
            .expect("the move primitive must accept every path the planner produced");
    }
}
