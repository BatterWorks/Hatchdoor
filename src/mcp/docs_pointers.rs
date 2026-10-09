//! Which manual page explains a structured tool error (#423).
//!
//! A standalone tool call refused with one of the codes below carries a
//! `docs` field beside its `code`: the page of the bundled manual (ADR-38)
//! that explains the condition, as a name `read_docs` accepts, and the heading
//! on it. Only the codes the troubleshooting and Vault-state pages explain are
//! listed; every other error is left exactly as it was.
//!
//! The field is added to the tool result, never to `VaultOperationError`: that
//! type also serialises into HTTP bodies (ADR-19) and into `batch` item
//! errors, neither of which changes.

use serde::Serialize;
use serde_json::Value;

use super::protocol::tool_structured_error;

/// What [`DOCS_FIELD`] holds. `page` is a name `read_docs` accepts; `heading`
/// is a heading anchor on that page, in the note slug rule. Errors advertise
/// no schema, since `outputSchema` describes the success shape alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ErrorDocs {
    pub page: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<&'static str>,
}

/// The field a structured tool error carries its [`ErrorDocs`] in.
pub const DOCS_FIELD: &str = "docs";

const TROUBLESHOOTING: &str = "guides/how-to-troubleshoot-common-problems";
const VAULT_STATES: &str = "concepts/vault-lifecycle-states";

const BAD_STATE: ErrorDocs = ErrorDocs {
    page: TROUBLESHOOTING,
    heading: Some("a-vault-wont-index-or-stays-in-a-bad-state"),
};
const FOLDER_ACCESS: ErrorDocs = ErrorDocs {
    page: TROUBLESHOOTING,
    heading: Some("permission-denied-reading-or-writing-the-vault"),
};
const GIT_FAILING: ErrorDocs = ErrorDocs {
    page: TROUBLESHOOTING,
    heading: Some("git-sync-is-failing"),
};
const WRITE_FAILING: ErrorDocs = ErrorDocs {
    page: TROUBLESHOOTING,
    heading: Some("a-write-fails-for-some-other-reason"),
};
const RECOVERY: ErrorDocs = ErrorDocs {
    page: VAULT_STATES,
    heading: Some("registry-recovery"),
};

/// Every code that names a page, and the page.
const POINTERS: &[(&str, ErrorDocs)] = &[
    ("vault_unavailable", BAD_STATE),
    ("vault_read_unavailable", BAD_STATE),
    ("vault_path_unavailable", FOLDER_ACCESS),
    ("vault_path_unreadable", FOLDER_ACCESS),
    (
        "vault_disabled",
        ErrorDocs {
            page: "guides/how-to-manage-multiple-vaults",
            heading: Some("pause-and-resume-a-vault"),
        },
    ),
    // A Vault control or read the Vault cannot serve right now, such as
    // refreshing a Vault that cannot be browsed. A write to a read-only Vault
    // is a JSON-RPC invalid-params error instead, so it never reaches here.
    (
        "capability_unavailable",
        ErrorDocs {
            page: VAULT_STATES,
            heading: Some("what-capabilities-actually-come-from"),
        },
    ),
    ("write_failed", WRITE_FAILING),
    ("write_recovery_required", WRITE_FAILING),
    ("managed_git_authentication_failed", GIT_FAILING),
    ("managed_git_remote_unreachable", GIT_FAILING),
    ("managed_git_install_failed", GIT_FAILING),
    ("managed_git_push_rejected", GIT_FAILING),
    ("managed_git_dirty_working_copy", GIT_FAILING),
    ("managed_git_operation_in_progress", GIT_FAILING),
    ("managed_git_pull_only_local_commits", GIT_FAILING),
    (
        "existing_git_local_history_manual_recovery_required",
        GIT_FAILING,
    ),
    (
        "managed_git_conflict",
        ErrorDocs {
            page: TROUBLESHOOTING,
            heading: Some("resolving-a-sync-conflict"),
        },
    ),
    ("vault_registry_recovery_required", RECOVERY),
];

/// The page that explains `code`, if the manual has one.
pub fn for_code(code: &str) -> Option<ErrorDocs> {
    POINTERS
        .iter()
        .find(|(listed, _)| *listed == code)
        .map(|(_, docs)| *docs)
}

/// `result` with [`DOCS_FIELD`] added when it is a structured tool error
/// whose `code` names a page. Anything else comes back unchanged: a success,
/// a plain-text tool error, an error for another code, and a `batch` result,
/// whose item errors sit inside a successful call.
pub fn attach(result: Value) -> Value {
    if result.get("isError") != Some(&Value::Bool(true)) {
        return result;
    }
    let Some(mut payload) = result.get("structuredContent").cloned() else {
        return result;
    };
    let Some(docs) = payload
        .get("code")
        .and_then(Value::as_str)
        .and_then(for_code)
    else {
        return result;
    };
    let Some(object) = payload.as_object_mut() else {
        return result;
    };
    object.insert(
        DOCS_FIELD.to_string(),
        serde_json::to_value(docs).expect("ErrorDocs serializes"),
    );
    // Rebuilt rather than patched, so the text rendering carries the field too.
    tool_structured_error(payload)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::mcp::protocol::{tool_error, tool_success};
    use crate::vault::slugify;

    /// Heading anchors on `markdown`, in the note slug rule the Help reader
    /// and wikilinks use. Emphasis and code marks are dropped first, as they
    /// are for a heading in a note.
    fn heading_anchors(markdown: &str) -> Vec<String> {
        let mut fenced = false;
        let mut anchors = Vec::new();
        for line in markdown.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                fenced = !fenced;
                continue;
            }
            if fenced || !trimmed.starts_with('#') {
                continue;
            }
            let text = trimmed.trim_start_matches('#');
            if !text.starts_with(' ') {
                continue;
            }
            let text: String = text
                .trim()
                .trim_end_matches('#')
                .chars()
                .filter(|c| !matches!(c, '`' | '*' | '_' | '~'))
                .collect();
            anchors.push(slugify(&text));
        }
        anchors
    }

    #[test]
    fn every_pointer_names_a_bundled_page_and_one_of_its_headings() {
        for (code, docs) in POINTERS {
            let page = crate::docs_bundle::page(docs.page)
                .unwrap_or_else(|| panic!("{code} points at missing page {}", docs.page));
            assert_eq!(page.name, docs.page, "{code} must use the exact page name");
            if let Some(heading) = docs.heading {
                assert!(
                    heading_anchors(&page.markdown).iter().any(|a| a == heading),
                    "{code} points at #{heading}, which {} does not have",
                    docs.page
                );
            }
        }
    }

    /// "Editing a note fails but creating one works" covers one cause only,
    /// which 2.7.0 removed, so it tells an agent whose write just failed that
    /// the problem is gone (#527).
    #[test]
    fn a_failed_write_points_at_the_section_that_says_where_to_look() {
        assert_eq!(
            for_code("write_failed"),
            Some(ErrorDocs {
                page: "guides/how-to-troubleshoot-common-problems",
                heading: Some("a-write-fails-for-some-other-reason"),
            })
        );
    }

    #[test]
    fn no_code_is_listed_twice() {
        let mut codes: Vec<&str> = POINTERS.iter().map(|(code, _)| *code).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), before);
    }

    #[test]
    fn a_listed_code_gains_the_docs_field_in_both_renderings() {
        let result = attach(tool_structured_error(json!({
            "code": "vault_disabled",
            "message": "Vault is disabled",
            "retryable": false,
        })));

        let payload = &result["structuredContent"];
        assert_eq!(
            payload[DOCS_FIELD],
            json!({
                "page": "guides/how-to-manage-multiple-vaults",
                "heading": "pause-and-resume-a-vault",
            })
        );
        assert_eq!(payload["code"], "vault_disabled");
        assert_eq!(payload["ok"], false);
        assert_eq!(result["isError"], true);
        let text: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().expect("text"))
                .expect("the text is the payload");
        assert_eq!(&text, payload);
    }

    #[test]
    fn everything_else_is_left_byte_for_byte_alone() {
        for result in [
            tool_structured_error(json!({
                "code": "note_not_found",
                "message": "no such note",
                "retryable": false,
            })),
            tool_error("Hatchdoor is still being set up.".to_string()),
            tool_success(
                json!({ "items": [{ "ok": false, "error": { "code": "vault_disabled" } }] }),
            ),
            tool_structured_error(json!("just a string")),
        ] {
            assert_eq!(attach(result.clone()), result);
        }
    }
}
