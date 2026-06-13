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
}

/// Build a commit message from a batch of write records.
/// Title summarizes ops + file count; body lists agent-supplied summaries.
pub fn build_commit_message(records: &[WriteRecord]) -> String {
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
        WriteRecord {
            op: op.to_string(),
            target: target.to_string(),
            affected_paths: paths.iter().map(PathBuf::from).collect(),
            summary: summary.map(str::to_string),
        }
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
        let msg = build_commit_message(&records);
        assert_eq!(
            msg,
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
        let msg = build_commit_message(&records);
        assert!(msg.starts_with("hatchdoor: update \"Project X\" (1 file)"));
        assert!(msg.ends_with("- tighten intro"));
    }

    #[test]
    fn empty_batch_has_fallback_message() {
        assert_eq!(build_commit_message(&[]), "hatchdoor: vault update");
    }
}
