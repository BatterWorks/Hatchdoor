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
    let mut results = evaluate_note_at(&markdown, notes, at, ceiling).queries;
    assert_eq!(results.len(), 1, "one block in, one result out");
    results.remove(0).outcome
}

/// Every saved query in `markdown`, evaluated against `notes` at a fixed time.
fn evaluate_note(
    markdown: &str,
    notes: &[VaultSnapshotNote],
    ceiling: SavedQueryCeiling,
) -> EvaluatedSavedQueries {
    evaluate_note_at(markdown, notes, "2026-09-18T12:00:00", ceiling)
}

fn evaluate_note_at(
    markdown: &str,
    notes: &[VaultSnapshotNote],
    at: &str,
    ceiling: SavedQueryCeiling,
) -> EvaluatedSavedQueries {
    evaluate_saved_queries(
        saved_query_blocks(markdown),
        vault_id(),
        notes,
        &clock(at),
        ceiling,
    )
}

fn blocks(markdown: &str) -> Vec<SavedQueryBlock> {
    saved_query_blocks(markdown).blocks
}

/// The rows an evaluated saved query produced, whether it found some or
/// none. Tests about the three states match the variants themselves.
fn table(outcome: SavedQueryOutcome) -> SavedQueryTable {
    match outcome {
        SavedQueryOutcome::Populated(table) => table,
        SavedQueryOutcome::Empty(empty) => SavedQueryTable {
            view_name: empty.view_name,
            columns: empty.columns,
            rows: Vec::new(),
            truncated: None,
            ignored: empty.ignored,
        },
        other => panic!("expected an evaluated table, got {other:?}"),
    }
}

fn titles(table: &SavedQueryTable) -> Vec<&str> {
    table.rows.iter().map(|row| row.title.as_str()).collect()
}

fn refusal(outcome: SavedQueryOutcome) -> SavedQueryRefusal {
    match outcome {
        SavedQueryOutcome::Refused(refusal) => refusal,
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
    let blocks = blocks(markdown);
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
    let names: Vec<_> = blocks(markdown)
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
fn the_frontmatter_is_not_searched_for_blocks() {
    let markdown = "---\nnote: |\n  ```base\n  filters: 'a == 1'\n  ```\n---\nBody\n";
    assert!(blocks(markdown).is_empty());
}

#[test]
fn an_indented_fence_loses_its_indent_and_an_unclosed_one_runs_to_the_end() {
    let markdown = "  ```base\n  filters: 'a == 1'\n  views: []\n";
    let blocks = blocks(markdown);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].source, "filters: 'a == 1'\nviews: []\n");
}

#[test]
fn crlf_line_endings_do_not_leak_into_the_source() {
    let blocks = blocks("```base\r\nfilters: 'a == 1'\r\n```\r\n");
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
    )
    .queries;
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
    )
    .queries;
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
    )
    .queries;
    assert_eq!(results.len(), MAX_SAVED_QUERIES_PER_NOTE + 2);
    let stopped = results
        .iter()
        .filter(|result| matches!(result.outcome, SavedQueryOutcome::Stopped { .. }))
        .count();
    assert_eq!(stopped, 2);
    assert!(matches!(
        results[MAX_SAVED_QUERIES_PER_NOTE - 1].outcome,
        SavedQueryOutcome::Populated(_)
    ));
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
    )
    .queries;
    assert!(matches!(
        results[0].outcome,
        SavedQueryOutcome::Populated(_)
    ));
    assert!(matches!(
        results[1].outcome,
        SavedQueryOutcome::Populated(_)
    ));
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
    let markdown = "```base\nformulas: {}\n```\n```base\nviews: []\n```\n";
    let results = evaluate_saved_queries(
        saved_query_blocks(markdown),
        vault_id(),
        &subscriptions(),
        &clock("2026-09-18T12:00:00"),
        ceiling,
    )
    .queries;
    assert!(matches!(results[0].outcome, SavedQueryOutcome::Refused(_)));
    assert!(matches!(
        results[1].outcome,
        SavedQueryOutcome::Populated(_)
    ));
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
        ("views:\n  - type: table\n  - type: table", "2 views"),
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
        let refusal = refusal(evaluate_one(source, &notes, at));
        assert!(
            refusal.message.contains(named),
            "{source:?} should be refused naming {named:?}, got {refusal:?}"
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
    ))
    .message;
    assert!(message.contains("nests"), "{message}");

    let negations = format!("filters: '{}price == 1'", "!".repeat(MAX_NESTING + 5));
    let message = refusal(evaluate_one(
        &negations,
        &subscriptions(),
        "2026-09-18T12:00:00",
    ))
    .message;
    assert!(message.contains("nests"), "{message}");
}

// --- Refused, empty and populated (#276) ----------------------------------

#[test]
fn malformed_yaml_is_refused_naming_the_problem_rather_than_drawn_empty() {
    let refusal = refusal(evaluate_one(
        "filters:\n  and:\n    - 'price > 1'\n   - broken: [",
        &subscriptions(),
        "2026-09-18T12:00:00",
    ));
    assert_eq!(refusal.construct, "YAML");
    assert!(refusal.message.contains("not valid YAML"), "{refusal:?}");
    // The parser's own account of where it failed reaches the reader.
    assert!(refusal.message.contains("line"), "{refusal:?}");
}

#[test]
fn a_refusal_names_the_construct_it_could_not_use() {
    let notes = subscriptions();
    let at = "2026-09-18T12:00:00";
    for (source, construct) in [
        ("filters: 'daysUntil(next_payment) < 7'", "daysUntil()"),
        (
            "filters:\n  and:\n    - file.hasTag(\"type/entity/subscription\")\n    - 'price.contains(\"x\")'",
            "contains()",
        ),
        ("filters: 'file.hasLink(\"x\")'", "file.hasLink"),
        ("filters: 'formula.total > 1'", "formula"),
        ("filters: 'price * 2 > 4'", "*"),
        ("filters:\n  xor:\n    - 'price == 1'", "xor"),
        ("formulas:\n  total: 'price * 12'", "formulas"),
        ("views:\n  - type: table\n    sort: [price]", "sort"),
        (
            "views:\n  - type: table\n    order: [formula.total]",
            "formula.total",
        ),
        ("views:\n  - type: table\n  - type: table", "views"),
        ("filters: 'price > 1 &&'", "price > 1 &&"),
    ] {
        let refusal = refusal(evaluate_one(source, &notes, at));
        assert_eq!(refusal.construct, construct, "{source:?}: {refusal:?}");
        assert!(
            refusal.message.contains(construct.trim_end_matches("()")),
            "the message names {construct:?} too: {refusal:?}"
        );
    }
}

#[test]
fn an_unsupported_function_inside_a_filter_refuses_the_whole_saved_query() {
    // The first condition alone selects four notes. Evaluating only the part
    // that was understood would draw them; nothing is drawn instead.
    let source = "filters:\n  and:\n    - file.hasTag(\"type/entity/subscription\")\n    - 'daysUntil(next_payment) < 7'";
    let outcome = evaluate_one(source, &subscriptions(), "2026-09-18T12:00:00");
    let SavedQueryOutcome::Refused(refusal) = outcome else {
        panic!("expected a refusal, got {outcome:?}");
    };
    assert!(refusal.message.contains("daysUntil"), "{refusal:?}");
}

#[test]
fn presentation_only_instructions_are_ignored_by_name_and_every_row_still_drawn() {
    let notes = subscriptions();
    let at = "2026-09-18T12:00:00";
    let plain = table(evaluate_one(
        "filters: 'file.hasTag(\"type/entity/subscription\")'",
        &notes,
        at,
    ));
    for (source, instruction) in [
        (
            "filters: 'file.hasTag(\"type/entity/subscription\")'\nviews:\n  - type: table\n    groupBy:\n      property: billing_period\n      direction: ASC",
            "groupBy",
        ),
        (
            "filters: 'file.hasTag(\"type/entity/subscription\")'\nviews:\n  - type: cards",
            "type: cards",
        ),
        (
            "filters: 'file.hasTag(\"type/entity/subscription\")'\nviews:\n  - type: table\n    summaries:\n      price: Sum",
            "summaries",
        ),
        (
            "filters: 'file.hasTag(\"type/entity/subscription\")'\nsummaries:\n  total: 'values.sum()'",
            "summaries",
        ),
    ] {
        let outcome = evaluate_one(source, &notes, at);
        let SavedQueryOutcome::Populated(drawn) = outcome else {
            panic!("{source:?} should draw its rows, got {outcome:?}");
        };
        assert_eq!(
            titles(&drawn),
            titles(&plain),
            "{source:?}: same rows as without it"
        );
        assert_eq!(
            drawn
                .ignored
                .iter()
                .map(|ignored| ignored.instruction.as_str())
                .collect::<Vec<_>>(),
            vec![instruction],
            "{source:?}"
        );
        assert!(!drawn.ignored[0].message.is_empty());
    }
}

#[test]
fn an_instruction_given_twice_is_reported_once() {
    let drawn = table(evaluate_one(
        "summaries:\n  total: 'values.sum()'\nviews:\n  - type: list\n    groupBy: price\n    summaries:\n      price: total",
        &subscriptions(),
        "2026-09-18T12:00:00",
    ));
    let instructions: Vec<_> = drawn
        .ignored
        .iter()
        .map(|ignored| ignored.instruction.as_str())
        .collect();
    assert_eq!(instructions, vec!["summaries", "type: list", "groupBy"]);
}

#[test]
fn a_refusing_construct_wins_over_an_ignorable_one() {
    let outcome = evaluate_one(
        "filters: 'daysUntil(next_payment) < 7'\nviews:\n  - type: table\n    groupBy: price",
        &subscriptions(),
        "2026-09-18T12:00:00",
    );
    assert!(
        matches!(outcome, SavedQueryOutcome::Refused(ref refusal) if refusal.construct == "daysUntil()"),
        "{outcome:?}"
    );
}

#[test]
fn a_valid_definition_matching_nothing_is_empty_not_refused() {
    let outcome = evaluate_one(
        "filters: 'price > 1000'\nviews:\n  - type: table\n    name: Dear ones\n    order: [file.name, price]\n    groupBy: price",
        &subscriptions(),
        "2026-09-18T12:00:00",
    );
    let SavedQueryOutcome::Empty(empty) = outcome else {
        panic!("expected empty, got {outcome:?}");
    };
    assert_eq!(empty.view_name.as_deref(), Some("Dear ones"));
    assert_eq!(empty.columns.len(), 2);
    assert_eq!(
        empty.ignored.len(),
        1,
        "an empty answer still says what it ignored"
    );
}

#[test]
fn no_outcome_carries_zero_rows_without_saying_which_state_produced_them() {
    let notes = subscriptions();
    let at = "2026-09-18T12:00:00";
    for source in [
        "filters: 'price > 1000'",
        "filters: 'price > 1'",
        "filters: 'price > 1'\nviews:\n  - type: table\n    limit: 1",
        "views: []",
        "filters: 'nope('",
        "filters: 'price > 1'\nviews:\n  - type: cards",
    ] {
        match evaluate_one(source, &notes, at) {
            SavedQueryOutcome::Populated(table) => {
                assert!(!table.rows.is_empty(), "{source:?}: populated with no rows");
            }
            SavedQueryOutcome::Empty(_)
            | SavedQueryOutcome::Refused(_)
            | SavedQueryOutcome::Stopped { .. } => {}
        }
    }
}

#[test]
fn outcomes_serialize_with_a_status_a_reader_can_branch_on() {
    let result = |outcome| SavedQueryResult {
        name: Some("named".to_string()),
        source: "s".to_string(),
        outcome,
    };
    assert_eq!(
        serde_json::to_value(result(SavedQueryOutcome::Refused(refuse(
            "daysUntil()",
            "no"
        ))))
        .expect("serialize"),
        json!({"name": "named", "source": "s", "status": "refused", "construct": "daysUntil()", "message": "no"})
    );
    assert_eq!(
        serde_json::to_value(result(SavedQueryOutcome::Empty(SavedQueryEmpty {
            view_name: None,
            columns: Vec::new(),
            ignored: vec![ignore_grouping()],
        })))
        .expect("serialize"),
        json!({
            "name": "named",
            "source": "s",
            "status": "empty",
            "columns": [],
            "ignored": [{"instruction": "groupBy", "message": "Grouping is not supported, so the rows are shown ungrouped."}],
        })
    );
    let populated =
        serde_json::to_value(result(SavedQueryOutcome::Populated(table(evaluate_one(
            "filters: 'price == 8'",
            &subscriptions(),
            "2026-09-18T12:00:00",
        )))))
        .expect("serialize");
    assert_eq!(populated["status"], "populated");
    assert_eq!(populated["rows"].as_array().map(Vec::len), Some(1));
    assert!(populated.get("ignored").is_none());
}

// --- Marker problems (#276) -----------------------------------------------

#[test]
fn a_marker_followed_by_no_block_is_reported_where_it_sits_and_nothing_else_changes() {
    let markdown = "\
---
tags: [x]
---
# Heading

<!-- hatchdoor-query: before-prose -->
Some prose.

<!-- hatchdoor-query: before-rust -->
```rust
fn main() {}
```

<!-- hatchdoor-query: kept -->

```base
filters: 'price == 8'
```

<!-- hatchdoor-query: Replaced -->
<!-- hatchdoor-query: at-the-end -->
";
    let found = saved_query_blocks(markdown);
    assert_eq!(
        found.orphaned_markers,
        vec![
            OrphanedMarker {
                name: "before-prose".to_string(),
                line: 6
            },
            OrphanedMarker {
                name: "before-rust".to_string(),
                line: 9
            },
            OrphanedMarker {
                name: "Replaced".to_string(),
                line: 20
            },
            OrphanedMarker {
                name: "at-the-end".to_string(),
                line: 21
            },
        ]
    );

    let evaluated = evaluate_note(markdown, &subscriptions(), SavedQueryCeiling::ENFORCED);
    assert_eq!(evaluated.queries.len(), 1);
    assert_eq!(evaluated.queries[0].name.as_deref(), Some("kept"));
    assert_eq!(
        titles(&table(evaluated.queries[0].outcome.clone())),
        vec!["Newspaper"]
    );
    assert_eq!(evaluated.marker_problems.len(), 4);
    let SavedQueryMarkerProblem::Orphaned {
        name,
        line,
        message,
    } = &evaluated.marker_problems[0]
    else {
        panic!("{:?}", evaluated.marker_problems);
    };
    assert_eq!((name.as_str(), *line), ("before-prose", 6));
    assert!(message.contains("before-prose"), "{message}");
}

#[test]
fn a_marker_with_an_unusable_name_is_reported_and_its_block_drawn_unnamed() {
    let markdown = "<!-- hatchdoor-query: Active Subs -->\n```base\nfilters: 'price == 8'\n```\n";
    let evaluated = evaluate_note(markdown, &subscriptions(), SavedQueryCeiling::ENFORCED);
    assert_eq!(evaluated.queries[0].name, None);
    assert_eq!(
        titles(&table(evaluated.queries[0].outcome.clone())),
        vec!["Newspaper"]
    );
    let [
        SavedQueryMarkerProblem::UnusableName {
            name,
            query,
            message,
        },
    ] = evaluated.marker_problems.as_slice()
    else {
        panic!("{:?}", evaluated.marker_problems);
    };
    assert_eq!((name.as_str(), *query), ("Active Subs", 0));
    assert!(message.contains("Active Subs"), "{message}");
}

#[test]
fn two_blocks_claiming_one_name_both_draw_and_the_collision_names_them_both() {
    let markdown = "<!-- hatchdoor-query: same -->\n```base\nviews: []\n```\n<!-- hatchdoor-query: other -->\n```base\nviews: []\n```\n<!-- hatchdoor-query: same -->\n```base\nfilters: 'price == 8'\n```\n";
    let evaluated = evaluate_note(markdown, &subscriptions(), SavedQueryCeiling::ENFORCED);
    assert_eq!(evaluated.queries.len(), 3);
    assert_eq!(table(evaluated.queries[0].outcome.clone()).rows.len(), 5);
    assert_eq!(
        titles(&table(evaluated.queries[2].outcome.clone())),
        vec!["Newspaper"]
    );
    // Both keep the name they claim; the collision, not a silent winner, is
    // what tells an addresser that it names neither.
    assert_eq!(evaluated.queries[0].name.as_deref(), Some("same"));
    assert_eq!(evaluated.queries[2].name.as_deref(), Some("same"));
    let [
        SavedQueryMarkerProblem::DuplicateName {
            name,
            queries,
            message,
        },
    ] = evaluated.marker_problems.as_slice()
    else {
        panic!("{:?}", evaluated.marker_problems);
    };
    assert_eq!(name, "same");
    assert_eq!(queries, &vec![0, 2]);
    assert!(
        message.contains("\"same\"") && message.contains("none of them"),
        "{message}"
    );
}

#[test]
fn marker_problems_serialize_with_a_kind_a_reader_can_branch_on() {
    let problem = SavedQueryMarkerProblem::Orphaned {
        name: "n".to_string(),
        line: 3,
        message: "m".to_string(),
    };
    assert_eq!(
        serde_json::to_value(&problem).expect("serialize"),
        json!({"problem": "orphaned", "name": "n", "line": 3, "message": "m"})
    );
}

// ---------------------------------------------------------------------------
// Addressing one saved query (#277)
// ---------------------------------------------------------------------------

const THREE_QUERIES: &str = "<!-- hatchdoor-query: active -->\n```base\nfilters: 'finished == null'\n```\n\n```base\nfilters: 'price == 8'\n```\n\n<!-- hatchdoor-query: finished -->\n```base\nfilters: 'finished < now()'\n```\n";

fn select(markdown: &str, name: Option<&str>) -> Result<SelectedSavedQuery, SelectionRefusal> {
    select_saved_query(saved_query_blocks(markdown), name)
}

#[test]
fn a_notes_saved_queries_are_listed_by_name_and_an_unnamed_one_as_present() {
    let summaries = saved_query_summaries(THREE_QUERIES);
    assert_eq!(
        summaries,
        vec![
            SavedQuerySummary {
                name: Some("active".to_string())
            },
            SavedQuerySummary { name: None },
            SavedQuerySummary {
                name: Some("finished".to_string())
            },
        ]
    );
    assert_eq!(
        serde_json::to_value(&summaries[1]).expect("serialize"),
        json!({"name": null}),
        "an unnamed saved query is reported present, with its name explicitly null"
    );
    assert!(saved_query_summaries("# Plain\n\n```yaml\na: 1\n```\n").is_empty());
}

#[test]
fn a_name_picks_its_saved_query_wherever_it_sits_in_the_note() {
    let selected = select(THREE_QUERIES, Some("finished")).expect("selected");
    assert_eq!(selected.name.as_deref(), Some("finished"));
    assert_eq!(selected.source, "filters: 'finished < now()'");

    // Reordering the note changes nothing about what the name answers.
    let reordered = "<!-- hatchdoor-query: finished -->\n```base\nfilters: 'finished < now()'\n```\n\n<!-- hatchdoor-query: active -->\n```base\nfilters: 'finished == null'\n```\n";
    let again = select(reordered, Some("finished")).expect("selected");
    assert_eq!(again.source, selected.source);
}

#[test]
fn the_name_may_be_left_out_only_when_the_note_holds_exactly_one_saved_query() {
    let one = "# One\n\n```base\nfilters: 'price == 8'\n```\n";
    let selected = select(one, None).expect("the only saved query");
    assert_eq!(selected.name, None);
    assert_eq!(selected.source, "filters: 'price == 8'");

    let refusal = select(THREE_QUERIES, None).expect_err("ambiguous without a name");
    assert_eq!(refusal.code(), "saved_query_name_required");
    let message = refusal.message();
    assert!(
        message.contains("3 saved queries")
            && message.contains("\"active\"")
            && message.contains("\"finished\"")
            && message.contains("1 has no name"),
        "{message}"
    );
}

#[test]
fn an_unknown_name_is_refused_and_lists_the_names_there_are() {
    let refusal = select(THREE_QUERIES, Some("activ")).expect_err("no such name");
    assert_eq!(refusal.code(), "saved_query_not_found");
    let message = refusal.message();
    assert!(
        message.contains("\"activ\"") && message.contains("\"active\""),
        "{message}"
    );

    // A name never reaches an unnamed saved query, even the only one.
    let one = "```base\nfilters: 'price == 8'\n```\n";
    assert_eq!(
        select(one, Some("anything")).expect_err("unnamed").code(),
        "saved_query_not_found"
    );
}

#[test]
fn a_name_two_saved_queries_share_is_refused_rather_than_resolved_to_either() {
    let markdown = "<!-- hatchdoor-query: same -->\n```base\nviews: []\n```\n<!-- hatchdoor-query: same -->\n```base\nfilters: 'price == 8'\n```\n";
    let refusal = select(markdown, Some("same")).expect_err("collision");
    assert_eq!(refusal.code(), "saved_query_name_ambiguous");
    assert!(
        refusal.message().contains("\"same\""),
        "{}",
        refusal.message()
    );
}

#[test]
fn an_unusable_marker_leaves_its_saved_query_unnamed_for_addressing_too() {
    let markdown = "<!-- hatchdoor-query: Active Subs -->\n```base\nviews: []\n```\n";
    assert_eq!(
        saved_query_summaries(markdown),
        vec![SavedQuerySummary { name: None }]
    );
    assert_eq!(
        select(markdown, Some("Active Subs"))
            .expect_err("unusable")
            .code(),
        "saved_query_not_found"
    );
    assert!(select(markdown, None).is_ok());
}

#[test]
fn a_note_with_no_saved_query_is_refused_whatever_is_asked() {
    for name in [None, Some("active")] {
        let refusal = select("# Plain\n\nNo queries.\n", name).expect_err("nothing to evaluate");
        assert_eq!(refusal.code(), "no_saved_queries");
        assert!(
            refusal.message().contains("no saved query"),
            "{}",
            refusal.message()
        );
    }
}

#[test]
fn one_addressed_saved_query_is_evaluated_alone_to_rows_or_a_refusal() {
    let at = clock("2026-09-06T12:00:00");
    let vault = vault_id();
    let populated = select(THREE_QUERIES, Some("finished"))
        .expect("selected")
        .evaluate(vault, &subscriptions(), &at, SavedQueryCeiling::ENFORCED)
        .into_rows()
        .expect("evaluated");
    let SavedQueryRows::Populated(table) = populated else {
        panic!("expected rows, got {populated:?}");
    };
    assert!(table.rows.iter().all(|row| row.vault_id == vault));
    assert!(!table.rows.is_empty());

    let empty = select("```base\nfilters: 'price == 12345'\n```\n", None)
        .expect("selected")
        .evaluate(vault, &subscriptions(), &at, SavedQueryCeiling::ENFORCED)
        .into_rows()
        .expect("evaluated");
    assert!(matches!(empty, SavedQueryRows::Empty(_)), "{empty:?}");

    let refused = select("```base\nfilters: 'daysUntil(renewal) < 7'\n```\n", None)
        .expect("selected")
        .evaluate(vault, &subscriptions(), &at, SavedQueryCeiling::ENFORCED)
        .into_rows()
        .expect_err("a broken definition is never rows");
    assert_eq!(refused.code(), "saved_query_refused");
    assert!(
        refused.message().contains("daysUntil()"),
        "{}",
        refused.message()
    );

    let tight = SavedQueryCeiling {
        max_scanned_notes: 2,
        max_rows: 500,
    };
    let stopped = select(THREE_QUERIES, Some("active"))
        .expect("selected")
        .evaluate(vault, &subscriptions(), &at, tight)
        .into_rows()
        .expect_err("past the scan ceiling");
    assert_eq!(stopped.code(), "saved_query_stopped");
}

#[test]
fn an_addressed_saved_query_past_the_per_note_limit_is_stopped_as_it_is_on_the_page() {
    let mut markdown = String::new();
    for index in 0..=MAX_SAVED_QUERIES_PER_NOTE {
        markdown.push_str(&format!(
            "<!-- hatchdoor-query: q{index} -->\n```base\nviews: []\n```\n\n"
        ));
    }
    let last = format!("q{MAX_SAVED_QUERIES_PER_NOTE}");
    let stopped = select(&markdown, Some(&last))
        .expect("selected")
        .evaluate(
            vault_id(),
            &subscriptions(),
            &clock("2026-09-06T12:00:00"),
            SavedQueryCeiling::ENFORCED,
        )
        .into_rows()
        .expect_err("past the per-note limit");
    assert_eq!(stopped.code(), "saved_query_stopped");
}

#[test]
fn an_evaluation_serializes_its_status_beside_the_rows() {
    let evaluation = SavedQueryEvaluation {
        vault_id: vault_id(),
        slug: "dashboard".to_string(),
        name: None,
        rows: SavedQueryRows::Empty(SavedQueryEmpty {
            view_name: None,
            columns: vec![],
            ignored: vec![],
        }),
    };
    let value = serde_json::to_value(&evaluation).expect("serialize");
    assert_eq!(value["status"], "empty");
    assert_eq!(value["name"], serde_json::Value::Null);
    assert!(
        value.get("rows").is_none(),
        "an empty answer carries no row list: {value}"
    );
}
