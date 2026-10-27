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
//!
//! The Vault-state codes, such as a Git sync failure or a folder Hatchdoor
//! cannot open, never come back as a tool error: an agent meets them as a
//! Vault's status error inside a successful `list_vaults` (#583). That result
//! gains the same field on each such status error, through
//! [`attach_to_vault_listing`], and the HTTP Vault list keeps its shape.

use schemars::{JsonSchema, Schema};
use serde::Serialize;
use serde_json::Value;

use super::protocol::tool_structured_error;

/// What [`DOCS_FIELD`] holds. `page` is a name `read_docs` accepts; `heading`
/// is a heading anchor on that page, in the note slug rule. Errors advertise
/// no schema, since `outputSchema` describes the success shape alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ErrorDocs {
    pub page: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<&'static str>,
}

/// The field a structured tool error carries its [`ErrorDocs`] in.
pub const DOCS_FIELD: &str = "docs";

/// Where each Vault in a `list_vaults` result holds a status error, in the
/// `VaultRuntimeError` shape: four fields of its own, and the error of its
/// latest recovery-branch request.
const STATUS_ERROR_PATHS: &[&[&str]] = &[
    &["activation_error"],
    &["search_error"],
    &["git_error"],
    &["watcher_error"],
    &["recovery_branch", "error"],
];

/// The name `list_vaults`' `outputSchema` gives the status error shape.
const STATUS_ERROR_SCHEMA: &str = "VaultRuntimeError";

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

/// `listing`, a `list_vaults` payload, with [`DOCS_FIELD`] added to every
/// Vault status error whose `code` names a page. Status errors for other
/// codes, and everything else in the listing, come back unchanged.
pub fn attach_to_vault_listing(mut listing: Value) -> Value {
    let Some(vaults) = listing.get_mut("vaults").and_then(Value::as_array_mut) else {
        return listing;
    };
    for vault in vaults {
        for path in STATUS_ERROR_PATHS {
            let error = path
                .iter()
                .try_fold(&mut *vault, |value, field| value.get_mut(*field));
            let Some(error) = error.and_then(Value::as_object_mut) else {
                continue;
            };
            let Some(docs) = error.get("code").and_then(Value::as_str).and_then(for_code) else {
                continue;
            };
            error.insert(
                DOCS_FIELD.to_string(),
                serde_json::to_value(docs).expect("ErrorDocs serializes"),
            );
        }
    }
    listing
}

/// `schema`, `list_vaults`' generated `outputSchema`, made to describe the
/// optional [`DOCS_FIELD`] [`attach_to_vault_listing`] adds. The status error
/// type is shared with the HTTP Vault list, which never carries the field, so
/// the description is added here rather than to the type.
pub fn describe_in_vault_listing_schema(schema: &mut Schema) {
    let mut docs = schemars::schema_for!(ErrorDocs);
    docs.remove("$schema");
    docs.insert(
        "description".to_string(),
        Value::from(
            "Present when the manual explains this code: the page, as a name read_docs accepts, and the heading on it.",
        ),
    );
    let properties = schema
        .get_mut("$defs")
        .and_then(|defs| defs.get_mut(STATUS_ERROR_SCHEMA))
        .and_then(|error| error.get_mut("properties"))
        .and_then(Value::as_object_mut)
        .expect("list_vaults' schema defines the status error with properties");
    properties.insert(DOCS_FIELD.to_string(), docs.to_value());
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

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

    /// Every code a standalone tool call can be refused with that this table
    /// could name: the closed lists of the management and read cores, and
    /// whatever the mutation core makes of each kind of write failure.
    fn tool_error_codes() -> Vec<String> {
        use crate::vault::WriteError;

        let vault_id =
            crate::vault_registry::VaultId::from_str("12345678-1234-4567-89ab-1234567890ab")
                .expect("Vault ID");
        let mut codes: Vec<String> = crate::vault_management::MANAGEMENT_ERROR_CODES
            .iter()
            .map(|code| code.to_string())
            .collect();
        codes.extend(POINTERS.iter().filter_map(|(code, _)| {
            let read = crate::vault_read::VaultReadError {
                code: code.to_string(),
                message: String::new(),
                vault_id: None,
                retryable: false,
            };
            (read.public_code() == *code).then(|| code.to_string())
        }));
        for error in [
            WriteError::Conflict(String::new()),
            WriteError::InvalidInput(String::new()),
            WriteError::Io(String::new()),
            WriteError::recovery_required(String::new()),
            WriteError::LinkRewriteUnsupported(Vec::new()),
        ] {
            codes.push(crate::vault_mutation::write_operation_error(vault_id, error).code);
        }
        codes
    }

    /// The producers of a Vault's status errors, which `list_vaults` reports,
    /// without their tests. Status codes have no closed list, so a code
    /// counts as one when a producer spells it.
    fn status_error_producers() -> Vec<&'static str> {
        [
            include_str!("../vault_runtime.rs"),
            include_str!("../vault_executor.rs"),
            include_str!("../vault_management.rs"),
            include_str!("../git/managed_task.rs"),
            include_str!("../git/sync.rs"),
        ]
        .into_iter()
        .map(|source| {
            source
                .split("#[cfg(test)]\nmod tests")
                .next()
                .unwrap_or(source)
        })
        .collect()
    }

    /// A pointer for a code no agent can be sent is a promise the manual
    /// cannot keep (#583): each code must reach an agent as a standalone tool
    /// error or as a Vault status error in `list_vaults`.
    #[test]
    fn every_pointer_can_reach_an_agent() {
        let tool_codes = tool_error_codes();
        let producers = status_error_producers();
        for (code, _) in POINTERS {
            let quoted = format!("\"{code}\"");
            assert!(
                tool_codes.iter().any(|tool| tool == code)
                    || producers.iter().any(|source| source.contains(&quoted)),
                "{code} is neither a tool error code nor a Vault status error code"
            );
        }
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

    #[test]
    fn a_listed_status_error_gains_the_docs_field_wherever_it_sits() {
        let listing = attach_to_vault_listing(json!({
            "collection_revision": 3,
            "vaults": [
                {
                    "activation_error": {"code": "vault_path_unreadable", "message": "m", "retryable": true},
                    "git_error": {"code": "managed_git_conflict", "message": "m", "retryable": false},
                    "recovery_branch": {"error": {"code": "managed_git_push_rejected", "message": "m", "retryable": false}},
                    "watcher_error": {"code": "watcher_failed", "message": "m", "retryable": true},
                },
                {"name": "healthy"},
            ],
        }));

        let vault = &listing["vaults"][0];
        assert_eq!(
            vault["activation_error"][DOCS_FIELD]["heading"],
            "permission-denied-reading-or-writing-the-vault"
        );
        assert_eq!(
            vault["git_error"][DOCS_FIELD]["heading"],
            "resolving-a-sync-conflict"
        );
        assert_eq!(
            vault["recovery_branch"]["error"][DOCS_FIELD]["heading"],
            "git-sync-is-failing"
        );
        assert!(vault["watcher_error"].get(DOCS_FIELD).is_none());
        assert_eq!(listing["vaults"][1], json!({"name": "healthy"}));
        assert_eq!(listing["collection_revision"], 3);
    }
}
