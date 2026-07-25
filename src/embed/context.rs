//! Document-side embedding input construction, shared by the indexing pipeline
//! and the index microbenchmark so the two never drift apart.

/// The text embedded for a chunk: the note's contextual header (its title and
/// the chunk's heading path) followed by the chunk body. Anchoring each chunk to
/// its note and section keeps mid-note chunks from embedding as anonymous
/// fragments. The model's `doc_prefix` is applied around this; queries are
/// unchanged (asymmetric embedding, richer document side only).
pub fn contextual_document(title: &str, heading_path: Option<&str>, body: &str) -> String {
    let header = match heading_path {
        Some(path) if !path.is_empty() => format!("{title} > {path}"),
        _ => title.to_string(),
    };
    if header.is_empty() {
        body.to_string()
    } else {
        format!("{header}\n\n{body}")
    }
}

#[cfg(test)]
mod tests {
    use super::contextual_document;

    #[test]
    fn prepends_title_and_heading_path() {
        let doc = contextual_document(
            "Postgres runbook",
            Some("Backups > Restoring"),
            "Stop the service first.",
        );
        assert_eq!(
            doc,
            "Postgres runbook > Backups > Restoring\n\nStop the service first."
        );
    }

    #[test]
    fn without_heading_uses_title_only() {
        let doc = contextual_document("Postgres runbook", None, "Stop the service first.");
        assert_eq!(doc, "Postgres runbook\n\nStop the service first.");
    }

    #[test]
    fn empty_heading_path_is_treated_as_absent() {
        let doc = contextual_document("Note", Some(""), "body");
        assert_eq!(doc, "Note\n\nbody");
    }

    #[test]
    fn empty_title_and_no_heading_falls_back_to_body() {
        let doc = contextual_document("", None, "body");
        assert_eq!(doc, "body");
    }
}
