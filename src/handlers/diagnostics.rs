//! Layer/noise diagnostics: one surface (HTTP route + MCP tool) that explains
//! *why* the classifier does what it does, without touching the index. It
//! re-runs the noise and layer matchers on demand, dumps the active ruleset with
//! provenance, and tallies notes per layer alongside any conflicts. Disabled
//! under `demo_mode` because it would reveal demoted paths.

use std::path::Path;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use crate::api_types::ErrorResponse;
use crate::app_state::{AppState, run_blocking, sqlite_cache};
use crate::cache::SqliteCache;
use crate::vault::{LayerMap, MARKER_FILE_NAME, VaultScanConfig};

/// How an arbitrary path string classifies, computed by re-running the matchers
/// rather than looking the path up in the index (a noise-excluded path is absent
/// from the index, so a lookup would answer the wrong question).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathClassification {
    pub path: String,
    pub excluded_as_noise: bool,
    pub is_layer_marker: bool,
    /// The layer the path would classify into (`None` = default surface).
    pub layer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoisePattern {
    pub pattern: String,
    /// `built-in` or `HATCHDOOR_EXCLUDE`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarkerInfo {
    pub directory: String,
    /// `None` when the marker re-surfaces its subtree to the default layer.
    pub layer: Option<String>,
    pub description: Option<String>,
    /// True when the marker's own directory matches a noise pattern, so its
    /// notes are pruned from the index even though the marker is honoured.
    pub directory_noise_excluded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayerCount {
    /// `None` = the default surface.
    pub layer: Option<String>,
    pub note_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayerConflict {
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayerDiagnostics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<PathClassification>,
    pub noise_patterns: Vec<NoisePattern>,
    pub markers: Vec<MarkerInfo>,
    pub layers: Vec<LayerCount>,
    pub conflicts: Vec<LayerConflict>,
    pub embed_layers: bool,
}

/// Assemble the diagnostics from the live matchers (`scan_config`), a freshly
/// collected `LayerMap`, and the last populate's cache state. Pure and shared by
/// the HTTP route and the MCP tool.
pub fn build_layer_diagnostics(
    vault_path: &Path,
    scan_config: &VaultScanConfig,
    cache: &SqliteCache,
    classify_path: Option<&str>,
) -> Result<LayerDiagnostics, String> {
    let layers = LayerMap::collect(vault_path)?;
    let exclude = &scan_config.exclude;

    // (1) Classify an arbitrary path by re-running the matchers on the raw
    // string. `layer_for` resolves by directory prefix, so a full note path
    // (with or without an extension) lands in the right layer.
    let classification = classify_path
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            let is_layer_marker =
                Path::new(raw).file_name().and_then(|name| name.to_str()) == Some(MARKER_FILE_NAME);
            PathClassification {
                path: raw.to_string(),
                excluded_as_noise: exclude.is_excluded(Path::new(raw), false),
                is_layer_marker,
                layer: layers.layer_for(raw).map(str::to_string),
            }
        });

    // (2a) Active noise ruleset with provenance.
    let noise_patterns = exclude
        .configured_patterns()
        .into_iter()
        .map(|(pattern, source)| NoisePattern {
            pattern,
            source: source.to_string(),
        })
        .collect();

    // (2b) Discovered markers with their directory and resolved layer. A marker
    // directory that itself matches a noise pattern is flagged: it is honoured
    // (marker collection ignores noise) but its notes are pruned.
    let markers: Vec<MarkerInfo> = layers
        .marker_paths()
        .into_iter()
        .map(|directory| {
            let layer = layers.layer_for(&directory).map(str::to_string);
            let description = layer
                .as_deref()
                .and_then(|name| layers.description(name))
                .map(str::to_string);
            let directory_noise_excluded =
                !directory.is_empty() && exclude.is_excluded(Path::new(&directory), true);
            MarkerInfo {
                directory,
                layer,
                description,
                directory_noise_excluded,
            }
        })
        .collect();

    // (3) Per-layer note counts from the cache.
    let layer_counts = cache.layer_note_counts()?;
    let layers_out: Vec<LayerCount> = layer_counts
        .iter()
        .map(|(layer, note_count)| LayerCount {
            layer: layer.clone(),
            note_count: *note_count,
        })
        .collect();

    // (3, conflicts)
    let mut conflicts = Vec::new();
    for marker in &markers {
        if marker.directory_noise_excluded {
            conflicts.push(LayerConflict {
                kind: "layer_directory_noise_excluded".to_string(),
                detail: format!(
                    "marker directory '{}' matches a noise-exclusion pattern; the marker is \
                     honoured but its notes are pruned from the index",
                    marker.directory
                ),
            });
        }
    }
    let live_layers: std::collections::BTreeSet<String> =
        layers.layer_names().into_iter().collect();
    for (layer, count) in &layer_counts {
        if let Some(name) = layer
            && !live_layers.contains(name)
        {
            conflicts.push(LayerConflict {
                kind: "vanished_marker".to_string(),
                detail: format!(
                    "{count} note(s) remain on layer '{name}' but no marker declares it any \
                     more; they are retained (never silently promoted) and reachable only via \
                     layers:[\"all\"]"
                ),
            });
        }
    }
    for (name, descriptions) in LayerMap::description_conflicts(vault_path)? {
        conflicts.push(LayerConflict {
            kind: "disagreeing_descriptions".to_string(),
            detail: format!(
                "layer '{name}' has conflicting marker descriptions: {}",
                descriptions.join(" | ")
            ),
        });
    }

    let embed_layers = cache
        .get_metadata("embed_layers")?
        .map(|value| value != "false")
        .unwrap_or(true);

    Ok(LayerDiagnostics {
        classification,
        noise_patterns,
        markers,
        layers: layers_out,
        conflicts,
        embed_layers,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsQuery {
    /// Optional path string to classify against the live matchers.
    pub path: Option<String>,
}

pub async fn diagnostics_handler(
    State(state): State<AppState>,
    Query(query): Query<DiagnosticsQuery>,
) -> impl IntoResponse {
    // Disabled entirely under demo mode: the ruleset and marker dump would
    // reveal demoted paths that demo mode is meant to exclude.
    if state.demo_mode {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Diagnostics are disabled in demo mode".to_string(),
            }),
        )
            .into_response();
    }

    let cache = match sqlite_cache(&state).await {
        Ok(cache) => cache,
        Err(err) => return err.into_response(),
    };
    let vault_path = state.vault_path.clone();
    let scan_config = state.scan_config.clone();
    let path = query.path;

    match run_blocking(move || {
        build_layer_diagnostics(&vault_path, &scan_config, &cache, path.as_deref())
    })
    .await
    {
        Ok(diagnostics) => (StatusCode::OK, Json(diagnostics)).into_response(),
        Err(err) => err.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::StubEmbedder;
    use crate::vault::{ExcludeMatcher, VaultIndex};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn layered_cache(root: &Path) -> Arc<SqliteCache> {
        let embedder = StubEmbedder::new(384);
        let cache = Arc::new(SqliteCache::in_memory(384).expect("cache"));
        let index = VaultIndex::build(root).expect("index");
        cache
            .replace_from_index_with_embedder(&index, &embedder)
            .expect("populate");
        cache
    }

    fn write_marker(root: &Path, dir: &str, body: &str) {
        std::fs::create_dir_all(root.join(dir)).expect("dir");
        std::fs::write(root.join(dir).join(".hatchdoor-layer"), body).expect("marker");
    }

    #[test]
    fn classifies_paths_by_rerunning_the_matchers() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        write_marker(root, "sources", "name: sources\ndescription: Clippings.\n");
        std::fs::write(root.join("sources/Clip.md"), "# Clip").expect("clip");
        std::fs::write(root.join("Home.md"), "# Home").expect("home");
        let cache = layered_cache(root);
        let scan_config = VaultScanConfig::default();

        // A demoted-folder path classifies into its layer.
        let demoted = build_layer_diagnostics(root, &scan_config, &cache, Some("sources/Clip.md"))
            .expect("diag");
        let classification = demoted.classification.expect("classification");
        assert_eq!(classification.layer.as_deref(), Some("sources"));
        assert!(!classification.excluded_as_noise);

        // A noise path classifies as excluded — even though it is absent from the
        // index, the matcher still answers.
        let noise =
            build_layer_diagnostics(root, &scan_config, &cache, Some(".obsidian/workspace.json"))
                .expect("diag");
        assert!(
            noise
                .classification
                .expect("classification")
                .excluded_as_noise
        );

        // The marker file classifies as a marker and is never noise.
        let marker =
            build_layer_diagnostics(root, &scan_config, &cache, Some("sources/.hatchdoor-layer"))
                .expect("diag");
        let marker_class = marker.classification.expect("classification");
        assert!(marker_class.is_layer_marker);
        assert!(!marker_class.excluded_as_noise);
    }

    #[test]
    fn reports_ruleset_provenance_and_per_layer_counts() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        write_marker(root, "sources", "name: sources\ndescription: Clippings.\n");
        std::fs::write(root.join("sources/Clip.md"), "# Clip").expect("clip");
        std::fs::write(root.join("sources/Clip2.md"), "# Clip2").expect("clip2");
        std::fs::write(root.join("Home.md"), "# Home").expect("home");
        let cache = layered_cache(root);
        let scan_config = VaultScanConfig {
            exclude: ExcludeMatcher::new(&["build/".to_string()]).expect("matcher"),
        };

        let diag = build_layer_diagnostics(root, &scan_config, &cache, None).expect("diag");

        // Provenance: built-in defaults plus the user pattern.
        assert!(diag.noise_patterns.iter().any(|p| p.source == "built-in"));
        assert!(
            diag.noise_patterns
                .iter()
                .any(|p| p.pattern == "build/" && p.source == "HATCHDOOR_EXCLUDE")
        );

        // Markers: the discovered layer with its description.
        let marker = diag
            .markers
            .iter()
            .find(|m| m.directory == "sources")
            .expect("sources marker");
        assert_eq!(marker.layer.as_deref(), Some("sources"));
        assert_eq!(marker.description.as_deref(), Some("Clippings."));

        // Counts: two demoted notes, one default.
        let sources = diag
            .layers
            .iter()
            .find(|c| c.layer.as_deref() == Some("sources"))
            .expect("sources count");
        assert_eq!(sources.note_count, 2);
        let default = diag
            .layers
            .iter()
            .find(|c| c.layer.is_none())
            .expect("default count");
        assert_eq!(default.note_count, 1);
    }

    #[test]
    fn flags_a_vanished_marker_as_a_conflict() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        write_marker(root, "sources", "name: sources\n");
        std::fs::write(root.join("sources/Clip.md"), "# Clip").expect("clip");
        std::fs::write(root.join("Home.md"), "# Home").expect("home");
        // Populate with the marker present, so the note lands on the layer.
        let cache = layered_cache(root);
        // Then the marker vanishes on disk before diagnostics run.
        std::fs::remove_file(root.join("sources/.hatchdoor-layer")).expect("remove marker");
        let scan_config = VaultScanConfig::default();

        let diag = build_layer_diagnostics(root, &scan_config, &cache, None).expect("diag");
        assert!(
            diag.conflicts
                .iter()
                .any(|c| c.kind == "vanished_marker" && c.detail.contains("sources")),
            "a layer with notes but no live marker must be a conflict: {:?}",
            diag.conflicts
        );
    }

    #[test]
    fn flags_disagreeing_marker_descriptions() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        write_marker(root, "a", "name: shared\ndescription: First.\n");
        write_marker(root, "b", "name: shared\ndescription: Second.\n");
        std::fs::write(root.join("a/Note.md"), "# A").expect("a note");
        let cache = layered_cache(root);
        let scan_config = VaultScanConfig::default();

        let diag = build_layer_diagnostics(root, &scan_config, &cache, None).expect("diag");
        let conflict = diag
            .conflicts
            .iter()
            .find(|c| c.kind == "disagreeing_descriptions")
            .expect("description conflict");
        assert!(conflict.detail.contains("First."));
        assert!(conflict.detail.contains("Second."));
    }

    #[test]
    fn flags_a_noise_excluded_layer_directory() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path();
        write_marker(root, "drafts", "name: drafts\n");
        std::fs::write(root.join("drafts/Note.md"), "# Draft").expect("draft");
        let cache = layered_cache(root);
        // A user pattern that excludes the very directory a marker lives in.
        let scan_config = VaultScanConfig {
            exclude: ExcludeMatcher::new(&["drafts/".to_string()]).expect("matcher"),
        };

        let diag = build_layer_diagnostics(root, &scan_config, &cache, None).expect("diag");
        assert!(
            diag.conflicts
                .iter()
                .any(|c| c.kind == "layer_directory_noise_excluded"),
            "a marker directory matched by a noise pattern must be a conflict: {:?}",
            diag.conflicts
        );
    }
}
