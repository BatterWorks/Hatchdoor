//! Typed MCP tool results and their advertised `outputSchema`s.
//!
//! Every MCP tool's success response is produced from one Rust structure, and
//! the same structure generates the JSON Schema that `tools/list` advertises
//! under `outputSchema`. That single source of truth means an agent can parse
//! a tool result structurally against exactly what the server promised, with
//! prose descriptions riding along on the schema rather than replacing it.
//!
//! Two kinds of structures live here:
//!
//! - Result shapes owned by the MCP surface itself (the setup tools, the
//!   attachment upload capability report, and the note/attachment write
//!   receipts), defined in this module.
//! - Re-exports of the shared Vault response structures the read tools proxy
//!   through the V1 handlers, and of Vault collection management's own wire
//!   types for the management tools — those wire contracts are the contract,
//!   so their types are reused directly rather than mirrored.
//!
//! Wire compatibility is deliberate: every structure serializes to exactly
//! the shape these tools returned before schemas existed, so existing clients
//! see no change except the new `outputSchema` advertisement itself.

use schemars::{JsonSchema, Schema};
use serde::Serialize;
use serde_json::{Value, json};

use crate::search::vault_scoped::VaultSearchResponse;
use crate::vault::AttachmentInfo;
use crate::vault_management::{
    VaultDiscoveryResponse, VaultMutationResponse, VaultScheduleResponse,
};
use crate::vault_read::{
    VaultGraph, VaultQualifiedLinks, VaultReadProjection, VaultRecentNote, VaultResolveResponse,
    VaultStatistics, VaultTree,
};

// ---------------------------------------------------------------------------
// Read-tool result aliases (shared V1 handler wire contracts)
// ---------------------------------------------------------------------------

pub type ListVaultsResult = VaultDiscoveryResponse;
pub type SearchNotesResult = VaultReadProjection<VaultSearchResponse>;
pub type GetNoteResult = crate::vault_read::VaultQualifiedNote;
pub type GetNoteLinksResult = VaultQualifiedLinks;
pub type ResolveWikilinkResult = VaultResolveResponse;
pub type GetTreeResult = VaultReadProjection<Vec<VaultTree>>;
pub type GetStatsResult = VaultReadProjection<Vec<VaultStatistics>>;
pub type GetGraphResult = VaultReadProjection<Vec<VaultGraph>>;
pub type RecentlyModifiedResult = VaultReadProjection<Vec<VaultRecentNote>>;
pub type CreateVaultResult = VaultMutationResponse;
pub type EditVaultResult = VaultMutationResponse;
pub type EnableVaultResult = VaultMutationResponse;
pub type DisableVaultResult = VaultMutationResponse;
pub type DisconnectVaultResult = VaultMutationResponse;
pub type SyncVaultResult = VaultScheduleResponse;
pub type RetryVaultResult = VaultScheduleResponse;

// ---------------------------------------------------------------------------
// Setup-tool results (always advertised, answered before any Vault exists)
// ---------------------------------------------------------------------------

/// `get_model_setup_status`'s answer: where first-run embedding-model setup
/// stands, the Gemma terms links, and the local-data privacy notice.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ModelSetupStatusResult {
    /// The full lifecycle status payload (`state` plus live progress when a
    /// download or indexing pass is running).
    pub state: crate::startup::StartupStatusResponse,
    pub gemma: ModelSetupModelInfo,
    pub fallback: ModelSetupFallbackInfo,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ModelSetupModelInfo {
    pub model: &'static str,
    pub terms_url: Option<&'static str>,
    pub policy_url: Option<&'static str>,
    pub terms_version: &'static str,
    pub repository: &'static str,
    pub revision: &'static str,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ModelSetupFallbackInfo {
    pub model: &'static str,
    pub notice: String,
}

/// `accept_gemma_terms` / `decline_gemma_terms` success receipt.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ModelChoiceResult {
    pub accepted: bool,
    pub model: &'static str,
}

// ---------------------------------------------------------------------------
// Capability report and write receipts owned by the MCP surface
// ---------------------------------------------------------------------------

/// One way an agent may upload an attachment into a Vault. The two variants
/// carry different fields (an HTTP endpoint has a path and auth story; the
/// base64 fallback names its tool), so they are internally tagged on `id`,
/// which is also the discriminator a caller sees on the wire today.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "id")]
pub enum AttachmentImportMethod {
    #[serde(rename = "http_multipart")]
    HttpMultipart {
        role: &'static str,
        method: &'static str,
        path: String,
        path_note: &'static str,
        max_bytes: u64,
        recommended_for: &'static str,
        auth: &'static str,
        requires: &'static str,
        usage: &'static str,
    },
    #[serde(rename = "mcp_base64")]
    McpBase64 {
        tool: &'static str,
        role: &'static str,
        max_bytes: u64,
        recommended_for: &'static str,
        usage: &'static str,
    },
}

/// `get_attachment_import_config`'s answer: whether uploads are possible for
/// this Vault right now, and by which methods, with per-method size limits.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AttachmentImportConfigResult {
    pub vault_id: String,
    pub enabled: bool,
    pub write_mode_enabled: bool,
    pub vault_accepts_mutation: bool,
    pub allowed_extensions: Vec<String>,
    pub methods: Vec<AttachmentImportMethod>,
    pub usage: String,
}

/// `list_note_attachments`' answer: the existing attachments one Note
/// references, without returning the Note's full content.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NoteAttachmentsResult {
    pub vault_id: String,
    pub attachments: Vec<AttachmentInfo>,
}

/// One way `get_attachment` may deliver an attachment's bytes: an HTTP
/// download URL by default, or inline base64 content as the fallback when an
/// out-of-band HTTP request isn't possible, or the URL's own credential is
/// unavailable to this client. The two variants carry different fields (a
/// URL has its own path/auth story; base64 just carries content), so this is
/// internally tagged on `encoding` — the same field name `get_attachment`'s
/// own argument uses to choose between them.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "encoding")]
pub enum AttachmentContent {
    #[serde(rename = "url")]
    Url {
        download_url: String,
        path_note: &'static str,
        auth: &'static str,
    },
    #[serde(rename = "base64")]
    Base64 { content: String },
}

/// `get_attachment`'s answer: one attachment's bytes, addressed by the same
/// `relative_path` `list_note_attachments` reports.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetAttachmentResult {
    pub vault_id: String,
    pub relative_path: String,
    pub size_bytes: u64,
    pub content_type: String,
    pub content: AttachmentContent,
}

/// The receipt every note-mutation tool returns (`create_note` through
/// `delete_note`). `layer` reports the resulting surface of the written note
/// (`null` = default surface); it is always `null` after a delete, which
/// leaves no note behind.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NoteWriteResult {
    pub vault_id: String,
    pub ok: bool,
    pub slug: Option<String>,
    pub relative_path: Option<String>,
    pub content_hash: Option<String>,
    pub layer: Option<String>,
    pub quality_warnings: Vec<String>,
    pub rewritten_notes: usize,
    pub moved_assets: usize,
    pub trashed_path: Option<String>,
}

/// `get_frontmatter`'s answer: the note's frontmatter projection — tags,
/// aliases, and every remaining property — without the Markdown body. A
/// note with no frontmatter block answers `has_frontmatter: false` with an
/// empty projection rather than an error.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetFrontmatterResult {
    pub vault_id: String,
    pub slug: String,
    pub relative_path: String,
    pub has_frontmatter: bool,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub properties: serde_json::Map<String, Value>,
}

/// The receipt every attachment-mutation tool returns (`import_attachment`
/// through `delete_attachment`). A move/rename/delete carries no new
/// `attachment` metadata beyond identity, so only `import_attachment` fills
/// the full info block; the others repeat the source identity they acted on.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AttachmentWriteResult {
    pub vault_id: String,
    pub ok: bool,
    pub attachment: AttachmentInfo,
    pub rewritten_notes: usize,
    pub trashed_path: Option<String>,
    pub cleanup_warning: Option<String>,
}

/// One `batch` item's outcome. `result` carries the named tool's own
/// `structuredContent` on success; `error` carries a structured failure in
/// the same shape a standalone call to that tool would return (either the
/// domain error object, or `{code, message}` for a protocol-level failure
/// such as an unresolvable Vault).
///
/// `ok` is authoritative and exactly one of `result`/`error` is present with
/// it. This deliberately stays three flat fields rather than the tagged enum
/// [`AttachmentContent`] uses for its own either/or: `ok` plus `result`/`error`
/// is the shape MCP clients expect from a batch, and it is already the
/// advertised `outputSchema`. The invariant is therefore held by the two
/// construction sites in `mcp::tools::batch` — both set `ok` and its matching
/// field together — not by the type. Keep them in step when editing either.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BatchItemResult {
    pub index: usize,
    pub op: String,
    pub ok: bool,
    pub result: Option<Value>,
    pub error: Option<Value>,
}

/// `batch`'s answer: one ordered outcome per requested operation. Execution
/// is best-effort — an earlier item's failure never stops a later item from
/// running, and there is no rollback.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BatchResult {
    pub items: Vec<BatchItemResult>,
    pub succeeded: usize,
    pub failed: usize,
}

// ---------------------------------------------------------------------------
// outputSchema registry
// ---------------------------------------------------------------------------

macro_rules! output_schemas {
    ($($name:literal => $ty:ty),* $(,)?) => {
        /// The `outputSchema` a tool advertises, generated from the same Rust
        /// structure its success responses serialize from. Returns `None` for
        /// a tool name outside this catalogue (never happens for an advertised
        /// tool).
        pub fn output_schema_for(tool_name: &str) -> Option<Schema> {
            match tool_name {
                $($name => Some(schemars::schema_for!($ty)),)*
                _ => None,
            }
        }
    };
}

output_schemas! {
    // Setup tools
    "get_model_setup_status" => ModelSetupStatusResult,
    "accept_gemma_terms" => ModelChoiceResult,
    "decline_gemma_terms" => ModelChoiceResult,
    // Read tools
    "list_vaults" => ListVaultsResult,
    "search_notes" => SearchNotesResult,
    "get_note" => GetNoteResult,
    "get_note_links" => GetNoteLinksResult,
    "resolve_wikilink" => ResolveWikilinkResult,
    "get_tree" => GetTreeResult,
    "get_stats" => GetStatsResult,
    "get_graph" => GetGraphResult,
    "recently_modified" => RecentlyModifiedResult,
    "get_attachment_import_config" => AttachmentImportConfigResult,
    "list_note_attachments" => NoteAttachmentsResult,
    "get_attachment" => GetAttachmentResult,
    "get_frontmatter" => GetFrontmatterResult,
    "batch" => BatchResult,
    // Management tools
    "create_vault" => CreateVaultResult,
    "edit_vault" => EditVaultResult,
    "enable_vault" => EnableVaultResult,
    "disable_vault" => DisableVaultResult,
    "disconnect_vault" => DisconnectVaultResult,
    "sync_vault" => SyncVaultResult,
    "retry_vault" => RetryVaultResult,
    // Note/attachment write tools
    "create_note" => NoteWriteResult,
    "update_note" => NoteWriteResult,
    "append_to_note" => NoteWriteResult,
    "edit_note" => NoteWriteResult,
    "replace_section" => NoteWriteResult,
    "update_frontmatter" => NoteWriteResult,
    "rename_note" => NoteWriteResult,
    "move_note" => NoteWriteResult,
    "move_rename_note" => NoteWriteResult,
    "archive_note" => NoteWriteResult,
    "delete_note" => NoteWriteResult,
    "import_attachment" => AttachmentWriteResult,
    "move_attachment" => AttachmentWriteResult,
    "rename_attachment" => AttachmentWriteResult,
    "delete_attachment" => AttachmentWriteResult,
}

/// Serializes a typed tool result into the value embedded in a tool success
/// response. Infallible in practice: every result structure is plain serde
/// data, so serialization cannot fail.
pub fn result_to_value<T: Serialize>(result: &T) -> Value {
    serde_json::to_value(result)
        .unwrap_or_else(|error| json!({ "serialization_error": error.to_string() }))
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    use schemars::schema_for;
    use serde_json::json;

    fn validator<T: JsonSchema>() -> jsonschema::Validator {
        jsonschema::validator_for(&serde_json::to_value(schema_for!(T)).expect("schema"))
            .expect("valid generated schema")
    }

    /// The real catalogues — `setup_tools_list()` plus `tools_list()` under a
    /// write-enabled config, i.e. every tool this server can ever advertise —
    /// are the source here, so catalogue drift fails in CI rather than as a
    /// schemaless tool at request time. The count is derived from the lists,
    /// not hand-copied.
    #[test]
    fn every_advertised_tool_has_an_output_schema() {
        let write_enabled = crate::mcp::config::McpConfig {
            enabled: true,
            write_enabled: true,
            max_attachment_bytes: 10 * 1024 * 1024,
            max_base64_bytes: 5 * 1024 * 1024,
            bearer_token: Some("test-token".to_string()),
            allowed_origins: vec![],
            rate_limits_enabled: true,
        };
        let mut names: Vec<String> = crate::mcp::tools::setup_tools_list()
            .into_iter()
            .chain(crate::mcp::tools::tools_list(&write_enabled))
            .map(|tool| {
                tool["name"]
                    .as_str()
                    .expect("advertised tool has a name")
                    .to_string()
            })
            .collect();
        let total = names.len();
        assert_eq!(
            total, 39,
            "3 setup + 13 read + 1 batch + 7 management + 15 write tools"
        );
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 39, "tool names are unique across catalogues");

        for name in &names {
            assert!(
                output_schema_for(name).is_some(),
                "{name} must advertise an outputSchema"
            );
        }
        assert!(output_schema_for("not_a_tool").is_none());
    }

    #[test]
    fn note_write_receipt_validates_across_optional_variants() {
        let validator = validator::<NoteWriteResult>();
        // create/update/append/edit/replace_section/rename/move: a live note.
        let created = serde_json::to_value(NoteWriteResult {
            vault_id: "018f47a0-7768-4d0c-8da3-5aa28d1c31c7".to_string(),
            ok: true,
            slug: Some("clip".to_string()),
            relative_path: Some("sources/Clip".to_string()),
            content_hash: Some("fnv1a64:abc".to_string()),
            layer: Some("sources".to_string()),
            quality_warnings: vec![],
            rewritten_notes: 2,
            moved_assets: 0,
            trashed_path: None,
        })
        .expect("serialize");
        assert!(validator.is_valid(&created), "live-note receipt");

        // delete/archive: no note left behind, so slug/path/hash/layer null out
        // exactly as this receipt shape has always serialized them.
        let deleted = serde_json::to_value(NoteWriteResult {
            vault_id: created["vault_id"].as_str().unwrap().to_string(),
            ok: true,
            slug: None,
            relative_path: None,
            content_hash: None,
            layer: None,
            quality_warnings: vec![],
            rewritten_notes: 1,
            moved_assets: 3,
            trashed_path: Some(".hatchdoor-trash/sources/Clip".to_string()),
        })
        .expect("serialize");
        assert!(validator.is_valid(&deleted), "deleted-note receipt");
        for key in ["slug", "relative_path", "content_hash", "layer"] {
            assert_eq!(
                deleted[key],
                serde_json::Value::Null,
                "{key} serializes as null when absent, matching the pre-schema wire shape"
            );
        }
    }

    #[test]
    fn attachment_import_config_validates_both_postures() {
        let validator = validator::<AttachmentImportConfigResult>();
        let method_schema =
            serde_json::to_value(schema_for!(AttachmentImportMethod)).expect("serialize");
        let method_validator =
            jsonschema::validator_for(&serde_json::to_value(method_schema).expect("value"))
                .expect("valid method schema");

        let enabled = serde_json::to_value(AttachmentImportConfigResult {
            vault_id: "018f47a0-7768-4d0c-8da3-5aa28d1c31c7".to_string(),
            enabled: true,
            write_mode_enabled: true,
            vault_accepts_mutation: true,
            allowed_extensions: allowed_extension_samples(),
            methods: vec![
                AttachmentImportMethod::HttpMultipart {
                    role: "default",
                    method: "POST",
                    path: "/api/v1/vaults/x/attachments".to_string(),
                    path_note: "resolve against this MCP endpoint",
                    max_bytes: 100_000_000,
                    recommended_for: "the default",
                    auth: "bearer token",
                    requires: "HTTP",
                    usage: "POST multipart/form-data",
                },
                AttachmentImportMethod::McpBase64 {
                    tool: "import_attachment",
                    role: "fallback",
                    max_bytes: 8_000_000,
                    recommended_for: "no out-of-band HTTP",
                    usage: "call import_attachment",
                },
            ],
            usage: "Two upload methods are available.".to_string(),
        })
        .expect("serialize");
        assert!(validator.is_valid(&enabled), "enabled posture validates");
        for method in enabled["methods"].as_array().unwrap() {
            assert!(
                method_validator.is_valid(method),
                "each advertised method matches the tagged-variant schema: {method}"
            );
        }
        // The tag discriminator survives serialization.
        assert_eq!(enabled["methods"][0]["id"], "http_multipart");
        assert_eq!(enabled["methods"][1]["id"], "mcp_base64");

        let disabled = serde_json::to_value(AttachmentImportConfigResult {
            vault_id: enabled["vault_id"].as_str().unwrap().to_string(),
            enabled: false,
            write_mode_enabled: false,
            vault_accepts_mutation: false,
            allowed_extensions: allowed_extension_samples(),
            methods: vec![],
            usage: "upload is disabled".to_string(),
        })
        .expect("serialize");
        assert!(validator.is_valid(&disabled), "disabled posture validates");
    }

    fn allowed_extension_samples() -> Vec<String> {
        crate::vault::allowed_attachment_extensions()
            .iter()
            .map(|extension| extension.to_string())
            .collect()
    }

    #[test]
    fn get_attachment_result_validates_both_encodings() {
        let validator = validator::<GetAttachmentResult>();

        let url_variant = serde_json::to_value(GetAttachmentResult {
            vault_id: "018f47a0-7768-4d0c-8da3-5aa28d1c31c7".to_string(),
            relative_path: "Sources/diagram.png".to_string(),
            size_bytes: 1234,
            content_type: "image/png".to_string(),
            content: AttachmentContent::Url {
                download_url: "/api/v1/vaults/x/assets/Sources/diagram.png".to_string(),
                path_note: "resolve against this MCP endpoint",
                auth: "requires the web bearer token",
            },
        })
        .expect("serialize");
        assert!(validator.is_valid(&url_variant), "url encoding validates");
        assert_eq!(url_variant["content"]["encoding"], "url");

        let base64_variant = serde_json::to_value(GetAttachmentResult {
            vault_id: "018f47a0-7768-4d0c-8da3-5aa28d1c31c7".to_string(),
            relative_path: "Sources/diagram.png".to_string(),
            size_bytes: 3,
            content_type: "image/png".to_string(),
            content: AttachmentContent::Base64 {
                content: "cG5n".to_string(),
            },
        })
        .expect("serialize");
        assert!(
            validator.is_valid(&base64_variant),
            "base64 encoding validates"
        );
        assert_eq!(base64_variant["content"]["encoding"], "base64");
    }

    #[test]
    fn management_schemas_carry_revision_and_identity_shapes() {
        let mutation = serde_json::to_value(schema_for!(VaultMutationResponse)).expect("schema");
        let text = serde_json::to_string(&mutation).expect("text");
        assert!(
            text.contains("registry_revision") && text.contains("collection_revision"),
            "mutation results advertise the revisioned collection contract"
        );

        let discovery = serde_json::to_value(schema_for!(VaultDiscoveryResponse)).expect("schema");
        let text = serde_json::to_string(&discovery).expect("text");
        assert!(
            text.contains("registry_revision"),
            "discovery carries registry_revision"
        );
        assert!(
            text.contains("commit_identity"),
            "per-Vault commit identity is part of the advertised summary"
        );
        assert!(
            text.contains("credential_configured"),
            "credentials appear only as a redacted boolean"
        );
        assert!(
            !text.contains("\"token\""),
            "no credential token field is ever advertised"
        );

        // A representative mutation response instance validates, both with a
        // Vault (edit result) and without (a refused edit returns none).
        let validator = validator::<VaultMutationResponse>();
        let with_vault = json!({
            "vault": {
                "vault_id": "018f47a0-7768-4d0c-8da3-5aa28d1c31c7",
                "name": "Notes",
                "enabled": true,
                "source": {"type":"local","path":"/tmp/v"},
                "exclude_patterns": [],
                "credential_configured": false,
                "activation": "active",
                "local_content": "read_write",
                "search": "ready",
                "git": "disabled",
                "watcher": "running",
                "capabilities": {"browse": true, "search": true, "mutate": false, "pull": false, "push": false, "retry": false}
            },
            "registry_revision": 3,
            "collection_revision": 9
        });
        assert!(
            validator.is_valid(&with_vault),
            "full mutation response validates"
        );
    }

    #[test]
    fn schedule_results_validate_queued_and_coalesced() {
        let validator = validator::<VaultScheduleResponse>();
        for schedule in ["queued", "coalesced"] {
            let instance = serde_json::to_value(VaultScheduleResponse {
                vault_id: crate::vault_registry::VaultId::generate().expect("id"),
                schedule: schedule.to_string(),
            })
            .expect("serialize");
            assert!(validator.is_valid(&instance), "{schedule} validates");
        }
    }

    #[test]
    fn setup_and_choice_schemas_match_their_wire_payloads() {
        let status_validator = validator::<ModelSetupStatusResult>();
        let status = json!({
            "state": {"state": "terms_required"},
            "gemma": {
                "model": "embeddinggemma-300m-q4",
                "terms_url": "https://ai.google.dev/gemma/terms",
                "policy_url": "https://ai.google.dev/gemma/policy",
                "terms_version": "v1",
                "repository": "google/embeddinggemma",
                "revision": "main"
            },
            "fallback": {"model": "nomic-embed-text-v1.5", "notice": "English only."}
        });
        assert!(status_validator.is_valid(&status), "setup status validates");

        let choice_validator = validator::<ModelChoiceResult>();
        let accepted = serde_json::to_value(ModelChoiceResult {
            accepted: true,
            model: crate::model_setup::GEMMA_MODEL_ID,
        })
        .expect("serialize");
        assert!(choice_validator.is_valid(&accepted));
    }

    #[test]
    fn read_envelopes_validate_representative_variants() {
        let resolve = validator::<ResolveWikilinkResult>();
        let resolved_null = json!({
            "vault_id": "018f47a0-7768-4d0c-8da3-5aa28d1c31c7",
            "slug": null
        });
        let resolved_hit = json!({
            "vault_id": "018f47a0-7768-4d0c-8da3-5aa28d1c31c7",
            "slug": "plan"
        });
        assert!(resolve.is_valid(&resolved_null), "unresolved target");
        assert!(resolve.is_valid(&resolved_hit), "resolved target");

        let recent = validator::<RecentlyModifiedResult>();
        let projection = json!({
            "scope": "all",
            "collection_revision": 4,
            "partial": false,
            "participants": [
                {
                    "vault_id": "018f47a0-7768-4d0c-8da3-5aa28d1c31c7",
                    "vault_name": "Notes",
                    "state": "fresh"
                }
            ],
            "data": [
                {
                    "vault_id": "018f47a0-7768-4d0c-8da3-5aa28d1c31c7",
                    "title": "Home",
                    "slug": "home",
                    "relative_path": "Home",
                    "mtime_ns": 1700000000000000000_i64
                }
            ]
        });
        assert!(
            recent.is_valid(&projection),
            "collection envelope validates"
        );
    }

    /// The single source of truth claim, proven on one tool end to end: the
    /// schema `tools/list` advertises and the receipt `tools/call` returns
    /// come from the same structure, so the instance validates against the
    /// advertised schema.
    #[test]
    fn advertised_schema_accepts_the_result_the_same_type_produces() {
        let schema = serde_json::to_value(output_schema_for("create_note").expect("schema"))
            .expect("serialize");
        let validator = jsonschema::validator_for(&schema).expect("valid schema");
        let receipt = crate::mcp::results::result_to_value(&NoteWriteResult {
            vault_id: "018f47a0-7768-4d0c-8da3-5aa28d1c31c7".to_string(),
            ok: true,
            slug: Some("new".to_string()),
            relative_path: Some("Projects/New".to_string()),
            content_hash: Some("fnv1a64:xyz".to_string()),
            layer: None,
            quality_warnings: vec!["heading missing".to_string()],
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: None,
        });
        assert!(
            validator.is_valid(&receipt),
            "tool receipt validates against its own advertised outputSchema"
        );
    }
}
