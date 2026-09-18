use serde_json::json;

use super::*;
use crate::vault::NoteMetadata;

fn note(path: &str, title: &str, tags: &[&str], properties: Value) -> VaultSnapshotNote {
    VaultSnapshotNote {
        title: title.to_string(),
        slug: path
            .trim_end_matches(".md")
            .replace('/', "-")
            .to_lowercase(),
        // Snapshot rows carry the path without the `.md` every Note file has.
        relative_path: path.trim_end_matches(".md").to_string(),
        size_bytes: 0,
        mtime_ns: 0,
        layer: None,
        metadata: NoteMetadata {
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            aliases: Vec::new(),
            properties,
        },
    }
}

fn clock(instant: &str) -> EvaluationClock {
    EvaluationClock::at(
        chrono::NaiveDateTime::parse_from_str(instant, "%Y-%m-%dT%H:%M:%S").expect("instant"),
    )
}

fn vault_id() -> VaultId {
    VaultId::generate().expect("generate Vault id")
}

/// The reporter's fixture from #272: one note per subscription, some finished.
fn subscriptions() -> Vec<VaultSnapshotNote> {
    vec![
        note(
            "subscriptions/Netflix.md",
            "Netflix",
            &["type/entity/subscription"],
            json!({"price": 13.99, "billing_period": "monthly", "next_payment": "2026-10-01"}),
        ),
        note(
            "subscriptions/Gym.md",
            "Gym",
            &["type/entity/subscription"],
            json!({"price": 30, "billing_period": "monthly", "finished": "2026-06-30"}),
        ),
        note(
            "subscriptions/Cloud storage.md",
            "Cloud storage",
            &["type/entity/subscription"],
            json!({"price": 99, "billing_period": "yearly", "finished": "2027-01-31"}),
        ),
        note(
            "subscriptions/Newspaper.md",
            "Newspaper",
            &["type/entity/subscription"],
            json!({"price": 8, "billing_period": "monthly", "finished": null}),
        ),
        note(
            "projects/Budget.md",
            "Budget",
            &["type/project"],
            json!({"price": 1}),
        ),
    ]
}

const SUBSCRIPTIONS_EXAMPLE: &str = r#"filters:
  and:
    - file.hasTag("type/entity/subscription")
    - 'finished == null || finished > now()'
views:
  - type: table
    name: Active subscriptions
    order:
      - file.name
      - price
      - billing_period
      - next_payment
      - finished"#;

fn evaluate_one(source: &str, notes: &[VaultSnapshotNote], at: &str) -> SavedQueryOutcome {
    evaluate_with(source, notes, at, SavedQueryCeiling::ENFORCED)
}

fn evaluate_with(
    source: &str,
    notes: &[VaultSnapshotNote],
    at: &str,
    ceiling: SavedQueryCeiling,
) -> SavedQueryOutcome {
    let markdown = format!("```base\n{source}\n```\n");
    let mut results = evaluate_saved_queries(
        saved_query_blocks(&markdown),
        vault_id(),
        notes,
        &clock(at),
        ceiling,
    );
    assert_eq!(results.len(), 1, "one block in, one result out");
    results.remove(0).outcome
}

fn table(outcome: SavedQueryOutcome) -> SavedQueryTable {
    match outcome {
        SavedQueryOutcome::Table(table) => table,
        other => panic!("expected a table, got {other:?}"),
    }
}

fn titles(table: &SavedQueryTable) -> Vec<&str> {
    table.rows.iter().map(|row| row.title.as_str()).collect()
}

fn refusal(outcome: SavedQueryOutcome) -> String {
    match outcome {
        SavedQueryOutcome::Refused { message } => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// --- Finding blocks -------------------------------------------------------

#[test]
fn every_base_block_is_found_in_document_order_and_other_fences_are_skipped() {
    let markdown = "\
# Heading

```base
filters: 'price > 1'
```

```rust
// ```base inside a rust block is code, not a saved query
```

~~~~ base
views: []
~~~~
";
    let blocks = saved_query_blocks(markdown);
    assert_eq!(
        blocks
            .iter()
            .map(|block| block.source.as_str())
            .collect::<Vec<_>>(),
        vec!["filters: 'price > 1'", "views: []"]
    );
}

#[test]
fn a_marker_names_the_block_after_it_across_blank_lines_only() {
    let markdown = "\
<!-- hatchdoor-query: active-subscriptions -->

```base
filters: 'a == 1'
```

<!-- hatchdoor-query: detached -->
Some prose in between.
```base
filters: 'b == 1'
```

```base
filters: 'c == 1'
```
";
    let names: Vec<_> = saved_query_blocks(markdown)
        .into_iter()
        .map(|block| block.name)
        .collect();
    assert_eq!(
        names,
        vec![
            MarkerName::Named("active-subscriptions".to_string()),
            MarkerName::Absent,
            MarkerName::Absent
        ]
    );
}

#[test]
fn a_marker_with_an_unusable_name_is_set_aside_and_the_rows_still_computed() {
    let markdown = "<!-- hatchdoor-query: Active Subs -->\n```base\nfilters: 'price == 8'\n```\n";
    let results = evaluate_saved_queries(
        saved_query_blocks(markdown),
        vault_id(),
        &subscriptions(),
        &clock("2026-09-18T12:00:00"),
        SavedQueryCeiling::ENFORCED,
    );
    assert_eq!(results[0].name, None);
    assert_eq!(
        titles(&table(results[0].outcome.clone())),
        vec!["Newspaper"]
    );
    assert_eq!(results[0].notices.len(), 1);
    assert!(
        results[0].notices[0].contains("Active Subs"),
        "{:?}",
        results[0].notices
    );
}

#[test]
fn the_frontmatter_is_not_searched_for_blocks() {
    let markdown = "---\nnote: |\n  ```base\n  filters: 'a == 1'\n  ```\n---\nBody\n";
    assert!(saved_query_blocks(markdown).is_empty());
}

#[test]
fn an_indented_fence_loses_its_indent_and_an_unclosed_one_runs_to_the_end() {
    let markdown = "  ```base\n  filters: 'a == 1'\n  views: []\n";
    let blocks = saved_query_blocks(markdown);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].source, "filters: 'a == 1'\nviews: []\n");
}

#[test]
fn crlf_line_endings_do_not_leak_into_the_source() {
    let blocks = saved_query_blocks("```base\r\nfilters: 'a == 1'\r\n```\r\n");
    assert_eq!(blocks[0].source, "filters: 'a == 1'");
}

// --- Evaluating -----------------------------------------------------------

#[test]
fn the_subscriptions_example_from_272_selects_the_active_ones() {
    let table = table(evaluate_one(
        SUBSCRIPTIONS_EXAMPLE,
        &subscriptions(),
        "2026-09-18T12:00:00",
    ));
    assert_eq!(table.view_name.as_deref(), Some("Active subscriptions"));
    // Gym finished in June; Budget is not a subscription at all.
    assert_eq!(
        titles(&table),
        vec!["Cloud storage", "Netflix", "Newspaper"]
    );
    assert_eq!(
        table
            .columns
            .iter()
            .map(|column| (column.id.as_str(), column.label.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("file.name", "name"),
            ("price", "price"),
            ("billing_period", "billing_period"),
            ("next_payment", "next_payment"),
            ("finished", "finished"),
        ]
    );
    assert_eq!(
        table.rows[1].cells,
        vec![
            json!("Netflix.md"),
            json!(13.99),
            json!("monthly"),
            json!("2026-10-01"),
            Value::Null
        ]
    );
    assert_eq!(table.truncated, None);
}

#[test]
fn a_filter_against_the_current_time_changes_its_answer_as_time_passes() {
    let before = table(evaluate_one(
        SUBSCRIPTIONS_EXAMPLE,
        &subscriptions(),
        "2026-09-18T12:00:00",
    ));
    let after = table(evaluate_one(
        SUBSCRIPTIONS_EXAMPLE,
        &subscriptions(),
        "2027-02-01T09:00:00",
    ));
    assert!(titles(&before).contains(&"Cloud storage"));
    assert!(!titles(&after).contains(&"Cloud storage"));
}

#[test]
fn today_is_the_date_alone() {
    let notes = vec![note("a.md", "A", &[], json!({"due": "2026-09-18"}))];
    let due_today = table(evaluate_one(
        "filters: 'due == today()'",
        &notes,
        "2026-09-18T23:59:00",
    ));
    assert_eq!(titles(&due_today), vec!["A"]);
}

#[test]
fn column_order_follows_the_definition_and_defaults_to_the_file_name() {
    let notes = subscriptions();
    let reordered = table(evaluate_one(
        "views:\n  - type: table\n    order: [note.billing_period, file.basename, file.folder, file.path, file.tags]",
        &notes,
        "2026-09-18T12:00:00",
    ));
    assert_eq!(
        reordered
            .columns
            .iter()
            .map(|column| column.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "note.billing_period",
            "file.basename",
            "file.folder",
            "file.path",
            "file.tags"
        ]
    );
    let budget = &reordered.rows[0];
    assert_eq!(budget.title, "Budget");
    assert_eq!(
        budget.cells,
        vec![
            Value::Null,
            json!("Budget"),
            json!("projects"),
            json!("projects/Budget.md"),
            json!(["type/project"])
        ]
    );

    let bare = table(evaluate_one(
        "filters: 'price > 50'",
        &notes,
        "2026-09-18T12:00:00",
    ));
    assert_eq!(
        bare.columns,
        vec![SavedQueryColumn {
            id: "file.name".to_string(),
            label: "name".to_string()
        }]
    );
}

#[test]
fn the_definitions_row_limit_is_respected_and_reported() {
    let table = table(evaluate_one(
        "filters: 'file.hasTag(\"type/entity/subscription\")'\nviews:\n  - type: table\n    limit: 2",
        &subscriptions(),
        "2026-09-18T12:00:00",
    ));
    assert_eq!(titles(&table), vec!["Cloud storage", "Gym"]);
    assert_eq!(
        table.truncated,
        Some(SavedQueryTruncation {
            reason: SavedQueryTruncationReason::DefinitionLimit,
            shown: 2
        })
    );
}

#[test]
fn rows_carry_their_notes_identity_so_they_can_link_to_them() {
    let id = vault_id();
    let results = evaluate_saved_queries(
        saved_query_blocks("```base\nfilters: 'price == 8'\n```"),
        id,
        &subscriptions(),
        &clock("2026-09-18T12:00:00"),
        SavedQueryCeiling::ENFORCED,
    );
    let table = table(results[0].outcome.clone());
    let row = &table.rows[0];
    assert_eq!(row.vault_id, id);
    assert_eq!(row.slug, "subscriptions-newspaper");
    assert_eq!(row.relative_path, "subscriptions/Newspaper");
}

#[test]
fn identical_vault_state_gives_an_identical_order_whatever_order_notes_arrive_in() {
    let mut notes = vec![
        note("b/Same.md", "Same", &["t"], json!({})),
        note("a/Same.md", "Same", &["t"], json!({})),
        note("c/alpha.md", "alpha", &["t"], json!({})),
        note("d/Beta.md", "Beta", &["t"], json!({})),
    ];
    let source = "filters: 'file.hasTag(\"t\")'\nviews:\n  - type: table\n    order: [file.path]";
    let first = table(evaluate_one(source, &notes, "2026-09-18T12:00:00"));
    notes.reverse();
    let second = table(evaluate_one(source, &notes, "2026-09-18T12:00:00"));
    let paths = |table: &SavedQueryTable| {
        table
            .rows
            .iter()
            .map(|row| row.relative_path.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(paths(&first), vec!["c/alpha", "d/Beta", "a/Same", "b/Same"]);
    assert_eq!(paths(&first), paths(&second));
}

#[test]
fn two_blocks_in_one_note_are_evaluated_independently() {
    let markdown = "\
```base
filters: 'price > 50'
```

Prose between the two.

<!-- hatchdoor-query: cheap -->
```base
filters: 'price < 10'
views:
  - type: table
    order: [price]
```
";
    let results = evaluate_saved_queries(
        saved_query_blocks(markdown),
        vault_id(),
        &subscriptions(),
        &clock("2026-09-18T12:00:00"),
        SavedQueryCeiling::ENFORCED,
    );
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].name, None);
    assert_eq!(
        titles(&table(results[0].outcome.clone())),
        vec!["Cloud storage"]
    );
    assert_eq!(results[1].name.as_deref(), Some("cheap"));
    assert_eq!(
        titles(&table(results[1].outcome.clone())),
        vec!["Budget", "Newspaper"]
    );
}

#[test]
fn boolean_groups_and_operators_select_what_bases_would() {
    let notes = subscriptions();
    let at = "2026-09-18T12:00:00";
    let select = |source: &str| titles(&table(evaluate_one(source, &notes, at))).join(",");

    assert_eq!(
        select("filters:\n  or:\n    - 'price == 8'\n    - 'price == 30'"),
        "Gym,Newspaper"
    );
    assert_eq!(
        select("filters:\n  not:\n    - file.hasTag(\"type/entity\")\n    - 'price > 50'"),
        "Budget"
    );
    assert_eq!(
        select("filters: '!(price >= 13) && billing_period != \"yearly\"'"),
        "Budget,Newspaper"
    );
    // A missing property is null to Bases, so != anything selects it.
    assert_eq!(
        select("filters: 'billing_period != \"monthly\"'"),
        "Budget,Cloud storage"
    );
    assert_eq!(select("filters: '10 < price'"), "Cloud storage,Gym,Netflix");
    assert_eq!(
        select("filters: 'finished.isEmpty() && price > 5'"),
        "Netflix,Newspaper"
    );
    assert_eq!(
        select("filters: 'note[\"next_payment\"] != null'"),
        "Netflix"
    );
    assert_eq!(select("filters: 'file.inFolder(\"projects\")'"), "Budget");
    assert_eq!(
        select("filters: 'file.name == \"Gym.md\" || file.basename == \"Netflix\"'"),
        "Gym,Netflix"
    );
    assert_eq!(
        select("filters: 'file.path == \"projects/Budget.md\" || file.folder == \"nowhere\"'"),
        "Budget"
    );
}

#[test]
fn a_view_filter_narrows_the_top_level_filter() {
    let table = table(evaluate_one(
        "filters: 'file.hasTag(\"type/entity/subscription\")'\nviews:\n  - type: table\n    filters: 'billing_period == \"yearly\"'",
        &subscriptions(),
        "2026-09-18T12:00:00",
    ));
    assert_eq!(titles(&table), vec!["Cloud storage"]);
}

#[test]
fn a_definition_with_no_filters_selects_every_note_within_the_ceiling() {
    let table = table(evaluate_one(
        "views:\n  - type: table",
        &subscriptions(),
        "2026-09-18T12:00:00",
    ));
    assert_eq!(table.rows.len(), 5);
}

#[test]
fn the_row_ceiling_holds_rows_back_and_says_so() {
    let ceiling = SavedQueryCeiling {
        max_scanned_notes: 100,
        max_rows: 2,
    };
    for source in [
        "views:\n  - type: table",
        "views:\n  - type: table\n    limit: 50",
    ] {
        let table = table(evaluate_with(
            source,
            &subscriptions(),
            "2026-09-18T12:00:00",
            ceiling,
        ));
        assert_eq!(table.rows.len(), 2);
        assert_eq!(
            table.truncated,
            Some(SavedQueryTruncation {
                reason: SavedQueryTruncationReason::Ceiling,
                shown: 2
            })
        );
    }
}

#[test]
fn a_vault_past_the_scan_ceiling_stops_the_query_rather_than_truncating() {
    let ceiling = SavedQueryCeiling {
        max_scanned_notes: 3,
        max_rows: 500,
    };
    let outcome = evaluate_with(
        "filters: 'price > 1'",
        &subscriptions(),
        "2026-09-18T12:00:00",
        ceiling,
    );
    assert!(matches!(
        outcome,
        SavedQueryOutcome::Stopped { ref message } if message.contains('5') && message.contains('3')
    ));
}

#[test]
fn a_note_cannot_multiply_its_way_past_the_ceiling() {
    let markdown = "```base\nviews: []\n```\n".repeat(MAX_SAVED_QUERIES_PER_NOTE + 2);
    let results = evaluate_saved_queries(
        saved_query_blocks(&markdown),
        vault_id(),
        &subscriptions(),
        &clock("2026-09-18T12:00:00"),
        SavedQueryCeiling::ENFORCED,
    );
    assert_eq!(results.len(), MAX_SAVED_QUERIES_PER_NOTE + 2);
    let stopped = results
        .iter()
        .filter(|result| matches!(result.outcome, SavedQueryOutcome::Stopped { .. }))
        .count();
    assert_eq!(stopped, 2);
    assert!(matches!(
        results[MAX_SAVED_QUERIES_PER_NOTE - 1].outcome,
        SavedQueryOutcome::Table(_)
    ));
}

#[test]
fn a_repeated_name_is_set_aside_on_the_later_block_whose_rows_still_appear() {
    let markdown = "<!-- hatchdoor-query: same -->\n```base\nviews: []\n```\n<!-- hatchdoor-query: same -->\n```base\nfilters: 'price == 8'\n```\n";
    let results = evaluate_saved_queries(
        saved_query_blocks(markdown),
        vault_id(),
        &subscriptions(),
        &clock("2026-09-18T12:00:00"),
        SavedQueryCeiling::ENFORCED,
    );
    assert_eq!(results[0].name.as_deref(), Some("same"));
    assert!(results[0].notices.is_empty());
    assert_eq!(results[1].name, None);
    assert!(
        results[1].notices[0].contains("same"),
        "{:?}",
        results[1].notices
    );
    assert_eq!(
        titles(&table(results[1].outcome.clone())),
        vec!["Newspaper"]
    );
}

#[test]
fn the_scan_ceiling_is_one_budget_for_the_whole_note() {
    // Five notes per query against a budget of twelve: two queries fit, the
    // third would take the note past it.
    let ceiling = SavedQueryCeiling {
        max_scanned_notes: 12,
        max_rows: 500,
    };
    let markdown = "```base\nviews: []\n```\n".repeat(3);
    let results = evaluate_saved_queries(
        saved_query_blocks(&markdown),
        vault_id(),
        &subscriptions(),
        &clock("2026-09-18T12:00:00"),
        ceiling,
    );
    assert!(matches!(results[0].outcome, SavedQueryOutcome::Table(_)));
    assert!(matches!(results[1].outcome, SavedQueryOutcome::Table(_)));
    assert!(matches!(
        &results[2].outcome,
        SavedQueryOutcome::Stopped { message } if message.contains("10") && message.contains("12")
    ));
}

#[test]
fn a_refused_query_spends_none_of_the_notes_scan_budget() {
    let ceiling = SavedQueryCeiling {
        max_scanned_notes: 5,
        max_rows: 500,
    };
    let markdown = "```base\nsummaries: {}\n```\n```base\nviews: []\n```\n";
    let results = evaluate_saved_queries(
        saved_query_blocks(markdown),
        vault_id(),
        &subscriptions(),
        &clock("2026-09-18T12:00:00"),
        ceiling,
    );
    assert!(matches!(
        results[0].outcome,
        SavedQueryOutcome::Refused { .. }
    ));
    assert!(matches!(results[1].outcome, SavedQueryOutcome::Table(_)));
}

#[test]
fn now_compares_with_a_time_written_with_a_space() {
    let notes = vec![note("a.md", "A", &[], json!({"due": "2026-09-18 15:00"}))];
    let source = "filters: 'due > now()'";
    assert_eq!(
        titles(&table(evaluate_one(source, &notes, "2026-09-18T12:00:00"))),
        vec!["A"]
    );
    assert!(titles(&table(evaluate_one(source, &notes, "2026-09-18T16:00:00"))).is_empty());
}

#[test]
fn anything_outside_the_subset_is_refused_by_name_rather_than_partly_applied() {
    let notes = subscriptions();
    let at = "2026-09-18T12:00:00";
    for (source, named) in [
        ("formulas:\n  total: 'price * 12'", "formulas"),
        (
            "properties:\n  price:\n    displayName: Price",
            "properties",
        ),
        ("summaries: {}", "summaries"),
        ("views:\n  - type: cards", "cards"),
        ("views:\n  - type: table\n  - type: table", "2 views"),
        ("views:\n  - type: table\n    groupBy: price", "groupBy"),
        ("views:\n  - type: table\n    sort: [price]", "sort"),
        ("views:\n  - type: table\n    limit: 0", "limit"),
        (
            "views:\n  - type: table\n    order: [formula.total]",
            "formula.total",
        ),
        ("filters: 'file.hasLink(\"x\")'", "file.hasLink"),
        ("filters: 'price.contains(\"x\")'", "contains"),
        ("filters: 'date(\"2026-01-01\") < now()'", "date"),
        ("filters: 'price * 2 > 4'", "*"),
        ("filters: 'price'", "comparison"),
        ("filters: 'tags == \"x\"'", "file.hasTag"),
        ("filters: 'this.price == 1'", "this"),
        ("filters: 'price > null'", "null"),
        ("filters:\n  xor:\n    - 'price == 1'", "xor"),
        ("filters:\n  and: []", "empty"),
        ("filters: 'price == 1 &&'", "price == 1 &&"),
        ("- just a list", "set of keys"),
        ("filters: [unclosed", "YAML"),
        ("", "empty"),
    ] {
        let message = refusal(evaluate_one(source, &notes, at));
        assert!(
            message.contains(named),
            "{source:?} should be refused naming {named:?}, got {message:?}"
        );
    }
}

#[test]
fn deeply_nested_expressions_are_refused_rather_than_overflowing() {
    let source = format!(
        "filters: '{}price == 1{}'",
        "(".repeat(MAX_NESTING + 5),
        ")".repeat(MAX_NESTING + 5)
    );
    let message = refusal(evaluate_one(
        &source,
        &subscriptions(),
        "2026-09-18T12:00:00",
    ));
    assert!(message.contains("nests"), "{message}");

    let negations = format!("filters: '{}price == 1'", "!".repeat(MAX_NESTING + 5));
    let message = refusal(evaluate_one(
        &negations,
        &subscriptions(),
        "2026-09-18T12:00:00",
    ));
    assert!(message.contains("nests"), "{message}");
}

#[test]
fn outcomes_serialize_with_a_status_a_reader_can_branch_on() {
    let refused = SavedQueryResult {
        name: Some("named".to_string()),
        source: "summaries: {}".to_string(),
        notices: Vec::new(),
        outcome: SavedQueryOutcome::Refused {
            message: "no".to_string(),
        },
    };
    assert_eq!(
        serde_json::to_value(&refused).expect("serialize"),
        json!({"name": "named", "source": "summaries: {}", "status": "refused", "message": "no"})
    );
    let empty = SavedQueryResult {
        name: None,
        source: String::new(),
        notices: vec!["set aside".to_string()],
        outcome: SavedQueryOutcome::Table(SavedQueryTable {
            view_name: None,
            columns: Vec::new(),
            rows: Vec::new(),
            truncated: None,
        }),
    };
    assert_eq!(
        serde_json::to_value(&empty).expect("serialize"),
        json!({"source": "", "notices": ["set aside"], "status": "table", "columns": [], "rows": []})
    );
}
