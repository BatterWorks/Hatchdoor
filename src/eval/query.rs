use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Query {
    pub id: String,
    pub query: String,
    pub expected_notes: Vec<String>,
    #[serde(default)]
    pub expected_heading_path: Option<String>,
    #[serde(default)]
    pub anti_expected: Vec<String>,
}

pub fn load_jsonl(path: &std::path::Path) -> Result<Vec<Query>, String> {
    let contents =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (idx, line) in contents.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let q: Query = serde_json::from_str(trimmed)
            .map_err(|e| format!("{} line {line_no}: {e}", path.display()))?;
        out.push(q);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_two_queries_with_optional_fields_handled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("q.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"id":"U1","query":"a","expected_notes":["n1","n2"]}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"id":"U2","query":"b","expected_notes":["n3"],"anti_expected":["x"]}}"#
        )
        .unwrap();
        let qs = load_jsonl(&path).expect("load");
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0].id, "U1");
        assert_eq!(qs[0].expected_notes, vec!["n1", "n2"]);
        assert!(qs[0].anti_expected.is_empty());
        assert_eq!(qs[1].anti_expected, vec!["x"]);
    }

    #[test]
    fn rejects_malformed_line_with_line_number() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("q.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"id":"U1","query":"a","expected_notes":["n"]}}"#).unwrap();
        writeln!(f, "this is not json").unwrap();
        let err = load_jsonl(&path).unwrap_err();
        assert!(err.contains("line 2"), "expected line 2 in error: {err}");
    }
}
