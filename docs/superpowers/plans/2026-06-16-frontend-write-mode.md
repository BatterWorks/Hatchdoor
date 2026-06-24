# Frontend Write Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build inline frontend note management for editing, creating, renaming, moving, and deleting Markdown notes.

**Architecture:** Add dedicated browser REST write endpoints that reuse the existing vault write primitives. Keep MCP as an agent protocol and keep the React frontend on normal web API calls. The note page owns inline editing, while reusable helper modules own write API calls and local draft persistence.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio, React 19, TypeScript, Vite, Vitest, Testing Library.

---

## File Structure

- Create `src/handlers/write_api.rs`
  - Owns request/response types and web write handlers.
  - Maps `WriteError::Conflict` to HTTP `409`, `WriteError::InvalidInput` to `400`, missing slugs to `404`, and IO/refresh failures to `500`.
  - Reuses `create_note`, `update_note`, `move_or_rename_note`, `delete_note`, `refresh_now`, `VaultIndex::build`, and `AppState::record_vault_write`.
- Modify `src/handlers/mod.rs`
  - Exports the new write handlers.
- Modify `src/main.rs`
  - Wires write routes into the existing protected API router.
  - Adds router tests for method wiring and write route behavior.
- Modify `frontend/src/types.ts`
  - Adds `content_hash` to `Note`.
  - Adds write capability and write outcome types.
- Create `frontend/src/writeApi.ts`
  - Owns frontend write API functions.
- Create `frontend/src/writeDrafts.ts`
  - Owns local draft keys, serialization, loading, saving, clearing, and dirty checks.
- Create `frontend/src/writeDrafts.test.ts`
  - Tests draft persistence independently from React.
- Create `frontend/src/components/NoteEditor.tsx`
  - Focused inline Markdown editor component with Save, Cancel, and error/status presentation.
- Create `frontend/src/components/NoteActionsDialog.tsx`
  - Focused dialogs for create metadata, rename, move, and delete confirmation.
- Modify `frontend/src/components/NotePage.tsx`
  - Owns edit mode, load/save conflict behavior, draft restore prompt, and successful save transition.
- Modify `frontend/src/app/AppTopbar.tsx`
  - Adds Edit, New note, Rename, Move, and Delete actions to the existing action surface.
- Modify `frontend/src/App.tsx`
  - Fetches write capabilities, passes note-management actions, handles create route state, and navigates after create/rename/move/delete.
- Modify CSS files already used by note UI, preferably `frontend/src/styles/note-content.css` or `frontend/src/App.css`
  - Adds editor and dialog styling without introducing a new design system.
- Create `frontend/src/App.write-mode.test.tsx`
  - Covers user-facing write flows.

## Commands

- Backend targeted tests during backend tasks:
  - `cargo test router_ --bin hatchdoor`
  - `cargo test write_api --bin hatchdoor`
- Backend wider check after backend tasks:
  - `cargo test --bin hatchdoor`
- Frontend targeted tests during frontend tasks:
  - `npm --prefix frontend test -- --run frontend/src/writeDrafts.test.ts`
  - `npm --prefix frontend test -- --run frontend/src/App.write-mode.test.tsx`
- Frontend wider checks after frontend tasks:
  - `npm --prefix frontend run typecheck`
  - `npm --prefix frontend test`
  - `npm --prefix frontend run lint`

## Task 1: Backend Write API Foundation

**Files:**
- Create: `src/handlers/write_api.rs`
- Modify: `src/handlers/mod.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing router tests**

Add tests in `src/main.rs` under the existing `#[cfg(test)] mod tests` block:

```rust
#[tokio::test]
async fn router_wires_write_capabilities_route() {
    let (app, _tmp) = app_for_tests();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/write-capabilities")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["enabled"], true);
    assert!(
        payload["warnings"]
            .as_array()
            .expect("warnings")
            .iter()
            .any(|warning| warning.as_str().unwrap_or("").contains("unauthenticated"))
    );
}

```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test router_wires_write_capabilities_route --bin hatchdoor`

Expected: FAIL because `/api/write-capabilities` is not wired.

- [ ] **Step 3: Add handler module and route exports**

Create `src/handlers/write_api.rs`:

```rust
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;

use crate::app_state::AppState;

#[derive(Debug, Serialize)]
pub struct WriteCapabilitiesResponse {
    pub enabled: bool,
    pub warnings: Vec<String>,
}

pub async fn write_capabilities_handler(State(_state): State<AppState>) -> impl IntoResponse {
    let warnings = vec![
        "Frontend writes are enabled without requiring Hatchdoor web authentication; do not expose this deployment to untrusted networks.".to_string(),
    ];
    (
        StatusCode::OK,
        Json(WriteCapabilitiesResponse {
            enabled: true,
            warnings,
        }),
    )
        .into_response()
}
```

Modify `src/handlers/mod.rs`:

```rust
mod api;
mod assets;
mod downloads;
mod spa;
mod write_api;

pub use api::{
    graph_handler, health_handler, note_handler, note_links_handler, recently_modified_handler,
    refresh_handler, resolve_batch_handler, resolve_handler, search_handler, stats_handler,
    tree_handler, vault_events_handler,
};
pub use assets::vault_asset_handler;
pub use downloads::note_download_handler;
pub use spa::spa_index_handler;
pub use write_api::write_capabilities_handler;
```

Modify imports and protected routes in `src/main.rs`:

```rust
use hatchdoor::handlers::{
    graph_handler, health_handler, note_download_handler, note_handler, note_links_handler,
    recently_modified_handler, refresh_handler, resolve_batch_handler, resolve_handler,
    search_handler, spa_index_handler, stats_handler, tree_handler, vault_asset_handler,
    vault_events_handler, write_capabilities_handler,
};
```

Add this route to the protected router:

```rust
.route("/api/write-capabilities", get(write_capabilities_handler))
```

- [ ] **Step 4: Run route test**

Run: `cargo test router_wires_write_capabilities_route --bin hatchdoor`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/handlers/write_api.rs src/handlers/mod.rs src/main.rs
git commit -m "feat(api): expose write capabilities" -m "- Add a web write capabilities endpoint for frontend write mode." -m "- Return an unauthenticated write warning for deployments without separate write auth."
```

## Task 2: Backend Note Write Endpoints

**Files:**
- Modify: `src/handlers/write_api.rs`
- Modify: `src/handlers/mod.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing endpoint tests**

Add tests in `src/main.rs`:

```rust
#[tokio::test]
async fn write_api_updates_note_and_rejects_stale_hash() {
    let (app, _tmp) = app_for_tests();

    let note_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/note/home")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let note_body = to_bytes(note_response.into_body(), usize::MAX)
        .await
        .expect("note body");
    let note_payload: serde_json::Value = serde_json::from_slice(&note_body).expect("json");
    let hash = note_payload["note"]["content_hash"].as_str().expect("hash");

    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/note/home")
                .method("PUT")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"content":"# Home\nupdated\n","expected_content_hash":"{hash}"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(update.status(), StatusCode::OK);

    let stale = app
        .oneshot(
            Request::builder()
                .uri("/api/note/home")
                .method("PUT")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"content":"# Home\nstale overwrite\n","expected_content_hash":"{hash}"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(stale.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn write_api_creates_renames_moves_and_deletes_note() {
    let (app, _tmp) = app_for_tests();

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/note")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"relative_path":"Projects/New Note.md","content":"# New Note\n"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(create.status(), StatusCode::OK);
    let create_body = to_bytes(create.into_body(), usize::MAX)
        .await
        .expect("body");
    let created: serde_json::Value = serde_json::from_slice(&create_body).expect("json");
    let slug = created["slug"].as_str().expect("slug");
    let hash = created["content_hash"].as_str().expect("hash");

    let rename = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/note/{slug}/rename"))
                .method("PATCH")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"new_title":"Renamed Note","expected_content_hash":"{hash}"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(rename.status(), StatusCode::OK);
    let rename_body = to_bytes(rename.into_body(), usize::MAX)
        .await
        .expect("body");
    let renamed: serde_json::Value = serde_json::from_slice(&rename_body).expect("json");
    let renamed_slug = renamed["slug"].as_str().expect("renamed slug");
    let renamed_hash = renamed["content_hash"].as_str().expect("renamed hash");

    let move_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/note/{renamed_slug}/move"))
                .method("PATCH")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"target_folder":"Archive","expected_content_hash":"{renamed_hash}"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(move_response.status(), StatusCode::OK);
    let move_body = to_bytes(move_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let moved: serde_json::Value = serde_json::from_slice(&move_body).expect("json");
    let moved_slug = moved["slug"].as_str().expect("moved slug");
    let moved_hash = moved["content_hash"].as_str().expect("moved hash");

    let delete = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/note/{moved_slug}"))
                .method("DELETE")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"expected_content_hash":"{moved_hash}"}}"#
                )))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(delete.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `cargo test write_api_ --bin hatchdoor`

Expected: FAIL because write routes and handlers are missing.

- [ ] **Step 3: Implement request and response types**

Add to `src/handlers/write_api.rs`:

```rust
use axum::extract::Path;
use serde::Deserialize;
use serde_json::json;

use crate::app_state::refresh_now;
use crate::vault::{
    VaultIndex, WriteError, WriteOutcome, create_note, delete_note, move_or_rename_note,
    update_note,
};

#[derive(Debug, Deserialize)]
pub struct CreateNoteRequest {
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNoteRequest {
    pub content: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameNoteRequest {
    pub new_title: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct MoveNoteRequest {
    pub target_folder: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct MoveRenameNoteRequest {
    pub target_relative_path: String,
    pub expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteNoteRequest {
    pub expected_content_hash: String,
}
```

- [ ] **Step 4: Implement shared helpers**

Add to `src/handlers/write_api.rs`:

```rust
async fn current_index(state: &AppState) -> Result<VaultIndex, axum::response::Response> {
    let vault_path = state.vault_path.clone();
    match tokio::task::spawn_blocking(move || VaultIndex::build(&vault_path)).await {
        Ok(Ok(index)) => Ok(index),
        Ok(Err(error)) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed to index vault: {error}") })),
        )
            .into_response()),
        Err(error) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("vault index build panicked: {error}") })),
        )
            .into_response()),
    }
}

fn write_error_response(error: WriteError) -> axum::response::Response {
    match error {
        WriteError::Conflict(message) => {
            (StatusCode::CONFLICT, Json(json!({ "error": message }))).into_response()
        }
        WriteError::InvalidInput(message) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
        }
        WriteError::Io(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": message })),
        )
            .into_response(),
    }
}

fn note_not_found(slug: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": format!("Note not found: {slug}") })),
    )
        .into_response()
}

fn replace_filename(relative_path: &str, new_title: &str) -> String {
    let directory = relative_path.rsplit_once('/').map(|(dir, _)| dir);
    match directory {
        Some(dir) if !dir.is_empty() => format!("{dir}/{new_title}.md"),
        _ => format!("{new_title}.md"),
    }
}

async fn finish_write(
    state: &AppState,
    op: &str,
    outcome: WriteOutcome,
) -> axum::response::Response {
    if let Err((_status, body)) = refresh_now(state).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": body.0.error })),
        )
            .into_response();
    }

    let target = outcome
        .relative_path
        .clone()
        .or_else(|| outcome.slug.clone())
        .unwrap_or_else(|| "note".to_string());
    state.record_vault_write(crate::git::WriteRecord {
        op: op.to_string(),
        target,
        affected_paths: outcome.affected_paths.clone(),
        summary: None,
    });

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "slug": outcome.slug,
            "relative_path": outcome.relative_path,
            "content_hash": outcome.content_hash,
            "rewritten_notes": outcome.rewritten_notes,
            "moved_assets": outcome.moved_assets,
            "trashed_path": outcome.trashed_path,
        })),
    )
        .into_response()
}
```

- [ ] **Step 5: Implement handlers**

Add handlers to `src/handlers/write_api.rs`:

```rust
pub async fn create_note_handler(
    State(state): State<AppState>,
    Json(payload): Json<CreateNoteRequest>,
) -> impl IntoResponse {
    let _guard = state.vault_write_lock.clone().lock_owned().await;
    match create_note(&state.vault_path, &payload.relative_path, &payload.content, false) {
        Ok(outcome) => finish_write(&state, "create", outcome).await,
        Err(error) => write_error_response(error),
    }
}

pub async fn update_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<UpdateNoteRequest>,
) -> impl IntoResponse {
    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(response) => return response,
    };
    let Some(entry) = index.find_by_slug(slug.trim()).cloned() else {
        return note_not_found(&slug);
    };
    match update_note(&entry, &payload.content, &payload.expected_content_hash) {
        Ok(outcome) => finish_write(&state, "update", outcome).await,
        Err(error) => write_error_response(error),
    }
}

pub async fn rename_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<RenameNoteRequest>,
) -> impl IntoResponse {
    let new_title = payload.new_title.trim();
    if new_title.is_empty() || new_title.contains('/') || new_title.contains('\\') {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "new_title cannot be empty or contain path separators" })),
        )
            .into_response();
    }
    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(response) => return response,
    };
    let Some(entry) = index.find_by_slug(slug.trim()).cloned() else {
        return note_not_found(&slug);
    };
    let target = replace_filename(&entry.relative_path, new_title);
    match move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target,
        &payload.expected_content_hash,
    ) {
        Ok(outcome) => finish_write(&state, "rename", outcome).await,
        Err(error) => write_error_response(error),
    }
}
```

Add these remaining handlers:

```rust
pub async fn move_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<MoveNoteRequest>,
) -> impl IntoResponse {
    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(response) => return response,
    };
    let Some(entry) = index.find_by_slug(slug.trim()).cloned() else {
        return note_not_found(&slug);
    };
    let target_folder = payload.target_folder.trim().trim_matches('/');
    let file_name = entry
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&entry.relative_path);
    let target = if target_folder.is_empty() {
        file_name.to_string()
    } else {
        format!("{target_folder}/{file_name}")
    };
    match move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        &target,
        &payload.expected_content_hash,
    ) {
        Ok(outcome) => finish_write(&state, "move", outcome).await,
        Err(error) => write_error_response(error),
    }
}

pub async fn move_rename_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<MoveRenameNoteRequest>,
) -> impl IntoResponse {
    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(response) => return response,
    };
    let Some(entry) = index.find_by_slug(slug.trim()).cloned() else {
        return note_not_found(&slug);
    };
    match move_or_rename_note(
        &state.vault_path,
        &index,
        &entry,
        payload.target_relative_path.trim(),
        &payload.expected_content_hash,
    ) {
        Ok(outcome) => finish_write(&state, "move_rename", outcome).await,
        Err(error) => write_error_response(error),
    }
}

pub async fn delete_note_handler(
    Path(slug): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<DeleteNoteRequest>,
) -> impl IntoResponse {
    let _guard = state.vault_write_lock.clone().lock_owned().await;
    let index = match current_index(&state).await {
        Ok(index) => index,
        Err(response) => return response,
    };
    let Some(entry) = index.find_by_slug(slug.trim()).cloned() else {
        return note_not_found(&slug);
    };
    match delete_note(
        &state.vault_path,
        &index,
        &entry,
        &payload.expected_content_hash,
    ) {
        Ok(outcome) => finish_write(&state, "delete", outcome).await,
        Err(error) => write_error_response(error),
    }
}
```

- [ ] **Step 6: Wire routes**

Export handlers from `src/handlers/mod.rs`:

```rust
pub use write_api::{
    create_note_handler, delete_note_handler, move_note_handler, move_rename_note_handler,
    rename_note_handler, update_note_handler, write_capabilities_handler,
};
```

Update `src/main.rs` imports:

```rust
create_note_handler, delete_note_handler, move_note_handler, move_rename_note_handler,
rename_note_handler, update_note_handler,
```

Add routes:

```rust
.route("/api/note", post(create_note_handler))
.route("/api/note/{slug}", get(note_handler).put(update_note_handler).delete(delete_note_handler))
.route("/api/note/{slug}/rename", axum::routing::patch(rename_note_handler))
.route("/api/note/{slug}/move", axum::routing::patch(move_note_handler))
.route("/api/note/{slug}/move-rename", axum::routing::patch(move_rename_note_handler))
```

Update the `axum::routing` import to include `patch` if fully qualified use is not used.

- [ ] **Step 7: Run backend tests**

Run: `cargo test write_api_ --bin hatchdoor`

Expected: PASS.

Run: `cargo test router_ --bin hatchdoor`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/handlers/write_api.rs src/handlers/mod.rs src/main.rs
git commit -m "feat(api): add note write endpoints" -m "- Add create, update, rename, move, move-rename, and delete routes for web note management." -m "- Reuse vault write primitives with hash conflicts, refresh, vault events, and git write records."
```

## Task 3: Frontend Write Types, API Client, And Draft Store

**Files:**
- Modify: `frontend/src/types.ts`
- Create: `frontend/src/writeApi.ts`
- Create: `frontend/src/writeDrafts.ts`
- Create: `frontend/src/writeDrafts.test.ts`

- [ ] **Step 1: Write failing draft tests**

Create `frontend/src/writeDrafts.test.ts`:

```ts
import { afterEach, describe, expect, it } from "vitest";

import {
  clearNoteDraft,
  loadNoteDraft,
  noteDraftKey,
  saveNoteDraft,
} from "./writeDrafts";

afterEach(() => {
  window.localStorage.clear();
});

describe("writeDrafts", () => {
  it("persists and clears existing-note drafts by slug", () => {
    const key = noteDraftKey("home");
    expect(key).toBe("hatchdoor:draft:note:home");

    saveNoteDraft("home", {
      slug: "home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    expect(loadNoteDraft("home")).toEqual({
      slug: "home",
      content: "# Home\nDraft",
      baseContentHash: "abc123",
      savedAt: 1781630000000,
    });

    clearNoteDraft("home");
    expect(loadNoteDraft("home")).toBeNull();
  });

  it("returns null for malformed draft JSON", () => {
    window.localStorage.setItem("hatchdoor:draft:note:broken", "{");
    expect(loadNoteDraft("broken")).toBeNull();
  });
});
```

- [ ] **Step 2: Run tests to verify failure**

Run: `npm --prefix frontend test -- --run frontend/src/writeDrafts.test.ts`

Expected: FAIL because `writeDrafts.ts` does not exist.

- [ ] **Step 3: Add types**

Modify `frontend/src/types.ts`:

```ts
export type Note = {
  title: string;
  slug: string;
  relative_path: string;
  content: string;
  content_hash: string;
};

export type WriteCapabilities = {
  enabled: boolean;
  warnings: string[];
};

export type WriteOutcome = {
  ok: boolean;
  slug: string | null;
  relative_path: string | null;
  content_hash: string | null;
  rewritten_notes: number;
  moved_assets: number;
  trashed_path: string | null;
};
```

- [ ] **Step 4: Implement draft store**

Create `frontend/src/writeDrafts.ts`:

```ts
export type NoteDraft = {
  slug: string;
  content: string;
  baseContentHash: string;
  savedAt: number;
};

export function noteDraftKey(slug: string): string {
  return `hatchdoor:draft:note:${slug}`;
}

export function createDraftKey(): string {
  return "hatchdoor:draft:create";
}

export function loadNoteDraft(slug: string): NoteDraft | null {
  try {
    const raw = window.localStorage.getItem(noteDraftKey(slug));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<NoteDraft>;
    if (
      typeof parsed.slug !== "string" ||
      typeof parsed.content !== "string" ||
      typeof parsed.baseContentHash !== "string" ||
      typeof parsed.savedAt !== "number"
    ) {
      return null;
    }
    return {
      slug: parsed.slug,
      content: parsed.content,
      baseContentHash: parsed.baseContentHash,
      savedAt: parsed.savedAt,
    };
  } catch {
    return null;
  }
}

export function saveNoteDraft(slug: string, draft: NoteDraft): void {
  try {
    window.localStorage.setItem(noteDraftKey(slug), JSON.stringify(draft));
  } catch {
    // Storage can fail in private browsing or when quota is exceeded.
  }
}

export function clearNoteDraft(slug: string): void {
  try {
    window.localStorage.removeItem(noteDraftKey(slug));
  } catch {
    // Ignore storage failures.
  }
}
```

- [ ] **Step 5: Implement write API client**

Create `frontend/src/writeApi.ts`:

```ts
import { apiFetch } from "./api";
import type { WriteCapabilities, WriteOutcome } from "./types";

async function parseError(res: Response): Promise<string> {
  try {
    const json = (await res.json()) as { error?: unknown };
    if (typeof json.error === "string") return json.error;
  } catch {
    // Fall back to status text below.
  }
  return `${res.status} ${res.statusText}`.trim();
}

async function requestJson<T>(url: string, init: RequestInit): Promise<T> {
  const res = await apiFetch(url, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...((init.headers as Record<string, string>) ?? {}),
    },
  });
  if (!res.ok) {
    const error = new Error(await parseError(res));
    error.name = res.status === 409 ? "ConflictError" : "WriteApiError";
    throw error;
  }
  return (await res.json()) as T;
}

export async function getWriteCapabilities(): Promise<WriteCapabilities> {
  const res = await apiFetch("/api/write-capabilities");
  if (!res.ok) throw new Error(await parseError(res));
  return (await res.json()) as WriteCapabilities;
}

export function createNote(relativePath: string, content: string): Promise<WriteOutcome> {
  return requestJson("/api/note", {
    method: "POST",
    body: JSON.stringify({ relative_path: relativePath, content }),
  });
}

export function updateNote(
  slug: string,
  content: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`/api/note/${encodeURIComponent(slug)}`, {
    method: "PUT",
    body: JSON.stringify({
      content,
      expected_content_hash: expectedContentHash,
    }),
  });
}

export function renameNote(
  slug: string,
  newTitle: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`/api/note/${encodeURIComponent(slug)}/rename`, {
    method: "PATCH",
    body: JSON.stringify({
      new_title: newTitle,
      expected_content_hash: expectedContentHash,
    }),
  });
}

export function moveNote(
  slug: string,
  targetFolder: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`/api/note/${encodeURIComponent(slug)}/move`, {
    method: "PATCH",
    body: JSON.stringify({
      target_folder: targetFolder,
      expected_content_hash: expectedContentHash,
    }),
  });
}

export function deleteNote(
  slug: string,
  expectedContentHash: string,
): Promise<WriteOutcome> {
  return requestJson(`/api/note/${encodeURIComponent(slug)}`, {
    method: "DELETE",
    body: JSON.stringify({ expected_content_hash: expectedContentHash }),
  });
}
```

- [ ] **Step 6: Run frontend tests**

Run: `npm --prefix frontend test -- --run frontend/src/writeDrafts.test.ts`

Expected: PASS.

Run: `npm --prefix frontend run typecheck`

Expected: PASS after existing note fixtures in tests include `content_hash`.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/types.ts frontend/src/writeApi.ts frontend/src/writeDrafts.ts frontend/src/writeDrafts.test.ts
git commit -m "feat(frontend): add write API and drafts" -m "- Add typed frontend note write calls for create, update, rename, move, and delete." -m "- Add local draft persistence for inline note editing."
```

## Task 4: Inline Note Editor

**Files:**
- Create: `frontend/src/components/NoteEditor.tsx`
- Modify: `frontend/src/components/NotePage.tsx`
- Modify: `frontend/src/styles/note-content.css`
- Create: `frontend/src/App.write-mode.test.tsx`

- [ ] **Step 1: Write failing edit-mode test**

Create `frontend/src/App.write-mode.test.tsx`:

```tsx
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "./App";

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  vi.restoreAllMocks();
});

function mockReadAndWriteApi() {
  const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      if (url.includes("/api/write-capabilities")) {
        return new Response(JSON.stringify({ enabled: true, warnings: [] }), { status: 200 });
      }
      if (url.includes("/api/tree")) {
        return new Response(
          JSON.stringify({ name: "Vault", folders: [], notes: [{ title: "Home", slug: "home" }] }),
          { status: 200 },
        );
      }
      if (url.includes("/api/recently-modified")) {
        return new Response(JSON.stringify({ notes: [] }), { status: 200 });
      }
      if (url.includes("/api/note/home/links")) {
        return new Response(JSON.stringify({ links: { outgoing: [], backlinks: [] } }), { status: 200 });
      }
      if (url.includes("/api/note/home") && method === "GET") {
        return new Response(
          JSON.stringify({
            note: {
              title: "Home",
              slug: "home",
              relative_path: "Home",
              content: "# Home\nOriginal",
              content_hash: "hash-1",
            },
          }),
          { status: 200 },
        );
      }
      if (url.includes("/api/note/home") && method === "PUT") {
        return new Response(
          JSON.stringify({
            ok: true,
            slug: "home",
            relative_path: "Home",
            content_hash: "hash-2",
            rewritten_notes: 0,
            moved_assets: 0,
            trashed_path: null,
          }),
          { status: 200 },
        );
      }
      if (url.includes("/api/resolve-batch")) {
        return new Response(JSON.stringify({ results: [] }), { status: 200 });
      }
      return new Response("not found", { status: 404 });
    },
  );
  return fetchMock;
}

describe("App write mode", () => {
  it("edits the current note inline and saves with content hash", async () => {
    const fetchMock = mockReadAndWriteApi();

    render(
      <MemoryRouter initialEntries={["/n/home"]}>
        <App />
      </MemoryRouter>,
    );

    expect(await screen.findByRole("heading", { name: "Home" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Edit note" }));

    const editor = await screen.findByLabelText("Markdown content");
    fireEvent.change(editor, { target: { value: "# Home\nUpdated" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        "/api/note/home",
        expect.objectContaining({
          method: "PUT",
          body: JSON.stringify({
            content: "# Home\nUpdated",
            expected_content_hash: "hash-1",
          }),
        }),
      );
    });
  });
});
```

- [ ] **Step 2: Run test to verify failure**

Run: `npm --prefix frontend test -- --run frontend/src/App.write-mode.test.tsx`

Expected: FAIL because edit actions and editor do not exist.

- [ ] **Step 3: Add editor component**

Create `frontend/src/components/NoteEditor.tsx`:

```tsx
import { UiButton } from "./ui";

export function NoteEditor({
  value,
  saving,
  error,
  onChange,
  onSave,
  onCancel,
}: {
  value: string;
  saving: boolean;
  error: string | null;
  onChange: (value: string) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="note-editor">
      <div className="note-editor-toolbar">
        <UiButton onClick={onSave} disabled={saving}>
          {saving ? "Saving" : "Save"}
        </UiButton>
        <UiButton className="close-note" onClick={onCancel} disabled={saving}>
          Cancel
        </UiButton>
      </div>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <textarea
        className="note-editor-textarea"
        aria-label="Markdown content"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  );
}
```

- [ ] **Step 4: Wire edit mode in `NotePage`**

Change `NotePage` props to include:

```ts
  writeEnabled: boolean;
  editRequested: boolean;
  onEditRequestHandled: () => void;
```

Import:

```ts
import { updateNote } from "../writeApi";
import { clearNoteDraft, loadNoteDraft, saveNoteDraft } from "../writeDrafts";
import { NoteEditor } from "./NoteEditor";
```

Add state:

```ts
const [editing, setEditing] = useState(false);
const [draftContent, setDraftContent] = useState("");
const [saveError, setSaveError] = useState<string | null>(null);
const [saving, setSaving] = useState(false);
```

Add effects:

```ts
useEffect(() => {
  if (!editRequested || !note || !writeEnabled) return;
  const existingDraft = loadNoteDraft(note.slug);
  setDraftContent(existingDraft?.content ?? note.content);
  setEditing(true);
  onEditRequestHandled();
}, [editRequested, note, onEditRequestHandled, writeEnabled]);

useEffect(() => {
  if (!editing || !note) return;
  saveNoteDraft(note.slug, {
    slug: note.slug,
    content: draftContent,
    baseContentHash: note.content_hash,
    savedAt: Date.now(),
  });
}, [draftContent, editing, note]);
```

Add handlers:

```ts
const saveDraft = useCallback(async () => {
  if (!note) return;
  setSaving(true);
  setSaveError(null);
  try {
    await updateNote(note.slug, draftContent, note.content_hash);
    clearNoteDraft(note.slug);
    setEditing(false);
    await loadNote(true);
    await loadNoteLinks();
  } catch (error) {
    setSaveError(
      error instanceof Error && error.name === "ConflictError"
        ? "This note changed on disk. Your draft was kept; reload the latest note before saving."
        : error instanceof Error
          ? error.message
          : "Save failed",
    );
  } finally {
    setSaving(false);
  }
}, [draftContent, loadNote, loadNoteLinks, note]);
```

Replace the markdown body rendering branch:

```tsx
{editing ? (
  <NoteEditor
    value={draftContent}
    saving={saving}
    error={saveError}
    onChange={setDraftContent}
    onSave={() => void saveDraft()}
    onCancel={() => {
      if (!note) return;
      if (draftContent !== note.content && !window.confirm("Discard unsaved draft?")) {
        return;
      }
      clearNoteDraft(note.slug);
      setEditing(false);
      setSaveError(null);
    }}
  />
) : (
  <div ref={noteBodyRef} className="note-body">
    <ReactMarkdown
      remarkPlugins={[remarkGfm, remarkMath]}
      rehypePlugins={rehypePlugins}
      components={markdownComponents}
    >
      {markdown}
    </ReactMarkdown>
  </div>
)}
```

- [ ] **Step 5: Trigger edit from topbar**

In `frontend/src/App.tsx`, add state:

```ts
const [editRequested, setEditRequested] = useState(false);
```

Pass to `AppTopbar`:

```tsx
onEditNote={() => setEditRequested(true)}
writeEnabled={writeCapabilities?.enabled ?? false}
```

Pass to `NotePage`:

```tsx
writeEnabled={writeCapabilities?.enabled ?? false}
editRequested={editRequested}
onEditRequestHandled={() => setEditRequested(false)}
```

In `frontend/src/app/AppTopbar.tsx`, add props:

```ts
  writeEnabled: boolean;
  onEditNote: () => void;
```

Add menu item inside `activeNote ? (...) : null` section:

```tsx
{activeNote && writeEnabled ? (
  <UiButton
    className="close-note"
    role="menuitem"
    onClick={() => {
      onCloseActionsMenu();
      onEditNote();
    }}
  >
    Edit note
  </UiButton>
) : null}
```

- [ ] **Step 6: Add editor CSS**

Add to `frontend/src/styles/note-content.css`:

```css
.note-editor {
  display: grid;
  gap: 0.75rem;
}

.note-editor-toolbar {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  align-items: center;
}

.note-editor-error {
  color: var(--err-fg);
  margin: 0;
}

.note-editor-textarea {
  width: 100%;
  min-height: min(68vh, 820px);
  resize: vertical;
  border: 1px solid var(--rule);
  border-radius: 8px;
  padding: 1rem;
  background: var(--paper);
  color: var(--ink);
  font: 0.95rem/1.55 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
```

- [ ] **Step 7: Fetch write capabilities in App**

Import and use:

```ts
import { getWriteCapabilities } from "./writeApi";
import type { WriteCapabilities } from "./types";
```

Add state:

```ts
const [writeCapabilities, setWriteCapabilities] = useState<WriteCapabilities | null>(null);
```

Add effect:

```ts
useEffect(() => {
  let cancelled = false;
  void (async () => {
    try {
      const capabilities = await getWriteCapabilities();
      if (!cancelled) setWriteCapabilities(capabilities);
    } catch {
      if (!cancelled) setWriteCapabilities({ enabled: false, warnings: [] });
    }
  })();
  return () => {
    cancelled = true;
  };
}, []);
```

- [ ] **Step 8: Run frontend tests**

Run: `npm --prefix frontend test -- --run frontend/src/App.write-mode.test.tsx`

Expected: PASS.

Run: `npm --prefix frontend run typecheck`

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/App.tsx frontend/src/app/AppTopbar.tsx frontend/src/components/NoteEditor.tsx frontend/src/components/NotePage.tsx frontend/src/styles/note-content.css frontend/src/App.write-mode.test.tsx
git commit -m "feat(frontend): add inline note editor" -m "- Add edit mode for the current note with local draft persistence." -m "- Save through the web write API with content-hash conflict handling."
```

## Task 5: Create, Rename, Move, And Delete UI

**Files:**
- Create: `frontend/src/components/NoteActionsDialog.tsx`
- Modify: `frontend/src/App.tsx`
- Modify: `frontend/src/app/AppTopbar.tsx`
- Modify: `frontend/src/App.write-mode.test.tsx`
- Modify: `frontend/src/App.css`

- [ ] **Step 1: Add failing management flow tests**

Append tests to `frontend/src/App.write-mode.test.tsx`:

```tsx
it("creates a new note from the actions menu", async () => {
  const fetchMock = mockReadAndWriteApi();

  render(
    <MemoryRouter initialEntries={["/n/home"]}>
      <App />
    </MemoryRouter>,
  );

  await screen.findByRole("heading", { name: "Home" });
  fireEvent.click(screen.getByRole("button", { name: "More actions" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "New note" }));
  fireEvent.change(screen.getByLabelText("Note path"), {
    target: { value: "Projects/New Note.md" },
  });
  fireEvent.change(screen.getByLabelText("Markdown content"), {
    target: { value: "# New Note\n" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Create" }));

  await waitFor(() => {
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/note",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          relative_path: "Projects/New Note.md",
          content: "# New Note\n",
        }),
      }),
    );
  });
});

it("opens rename, move, and delete dialogs from the actions menu", async () => {
  mockReadAndWriteApi();

  render(
    <MemoryRouter initialEntries={["/n/home"]}>
      <App />
    </MemoryRouter>,
  );

  await screen.findByRole("heading", { name: "Home" });
  fireEvent.click(screen.getByRole("button", { name: "More actions" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Rename note" }));
  expect(screen.getByLabelText("New title")).toBeInTheDocument();
});
```

Update `mockReadAndWriteApi` with these branches before the fallback `404`:

```ts
if (url.endsWith("/api/note") && method === "POST") {
  return new Response(
    JSON.stringify({
      ok: true,
      slug: "projects-new-note",
      relative_path: "Projects/New Note",
      content_hash: "hash-new",
      rewritten_notes: 0,
      moved_assets: 0,
      trashed_path: null,
    }),
    { status: 200 },
  );
}
if (url.includes("/api/note/home/rename") && method === "PATCH") {
  return new Response(
    JSON.stringify({
      ok: true,
      slug: "renamed-note",
      relative_path: "Renamed Note",
      content_hash: "hash-renamed",
      rewritten_notes: 1,
      moved_assets: 0,
      trashed_path: null,
    }),
    { status: 200 },
  );
}
if (url.includes("/api/note/home/move") && method === "PATCH") {
  return new Response(
    JSON.stringify({
      ok: true,
      slug: "archive-home",
      relative_path: "Archive/Home",
      content_hash: "hash-moved",
      rewritten_notes: 0,
      moved_assets: 0,
      trashed_path: null,
    }),
    { status: 200 },
  );
}
if (url.includes("/api/note/home") && method === "DELETE") {
  return new Response(
    JSON.stringify({
      ok: true,
      slug: "home",
      relative_path: "Home",
      content_hash: "hash-1",
      rewritten_notes: 0,
      moved_assets: 0,
      trashed_path: "90-archive/Home.md",
    }),
    { status: 200 },
  );
}
```

- [ ] **Step 2: Run tests to verify failure**

Run: `npm --prefix frontend test -- --run frontend/src/App.write-mode.test.tsx`

Expected: FAIL because note management dialogs are missing.

- [ ] **Step 3: Create dialogs**

Create `frontend/src/components/NoteActionsDialog.tsx`:

```tsx
import { UiButton } from "./ui";

export type NoteActionDialogKind = "create" | "rename" | "move" | "delete";

export function NoteActionsDialog({
  kind,
  error,
  onClose,
  onCreate,
  onRename,
  onMove,
  onDelete,
}: {
  kind: NoteActionDialogKind;
  error: string | null;
  onClose: () => void;
  onCreate: (relativePath: string, content: string) => void;
  onRename: (newTitle: string) => void;
  onMove: (targetFolder: string) => void;
  onDelete: () => void;
}) {
  return (
    <div className="modal-backdrop" role="presentation">
      <section className="modal-panel" role="dialog" aria-modal="true">
        {kind === "create" ? (
          <CreateForm error={error} onClose={onClose} onCreate={onCreate} />
        ) : null}
        {kind === "rename" ? (
          <RenameForm error={error} onClose={onClose} onRename={onRename} />
        ) : null}
        {kind === "move" ? (
          <MoveForm error={error} onClose={onClose} onMove={onMove} />
        ) : null}
        {kind === "delete" ? (
          <DeleteForm error={error} onClose={onClose} onDelete={onDelete} />
        ) : null}
      </section>
    </div>
  );
}

function CreateForm({
  error,
  onClose,
  onCreate,
}: {
  error: string | null;
  onClose: () => void;
  onCreate: (relativePath: string, content: string) => void;
}) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        onCreate(String(data.get("relativePath") ?? ""), String(data.get("content") ?? ""));
      }}
    >
      <h2>Create note</h2>
      <label>
        Note path
        <input name="relativePath" aria-label="Note path" />
      </label>
      <label>
        Markdown content
        <textarea name="content" aria-label="Markdown content" />
      </label>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <UiButton type="submit">Create</UiButton>
      <UiButton type="button" className="close-note" onClick={onClose}>Cancel</UiButton>
    </form>
  );
}
```

Add `RenameForm`, `MoveForm`, and `DeleteForm` in the same file:

```tsx
function RenameForm({
  error,
  onClose,
  onRename,
}: {
  error: string | null;
  onClose: () => void;
  onRename: (newTitle: string) => void;
}) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        onRename(String(data.get("newTitle") ?? ""));
      }}
    >
      <h2>Rename note</h2>
      <label>
        New title
        <input name="newTitle" aria-label="New title" />
      </label>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <UiButton type="submit">Rename</UiButton>
      <UiButton type="button" className="close-note" onClick={onClose}>Cancel</UiButton>
    </form>
  );
}

function MoveForm({
  error,
  onClose,
  onMove,
}: {
  error: string | null;
  onClose: () => void;
  onMove: (targetFolder: string) => void;
}) {
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        const data = new FormData(event.currentTarget);
        onMove(String(data.get("targetFolder") ?? ""));
      }}
    >
      <h2>Move note</h2>
      <label>
        Target folder
        <input name="targetFolder" aria-label="Target folder" />
      </label>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <UiButton type="submit">Move</UiButton>
      <UiButton type="button" className="close-note" onClick={onClose}>Cancel</UiButton>
    </form>
  );
}

function DeleteForm({
  error,
  onClose,
  onDelete,
}: {
  error: string | null;
  onClose: () => void;
  onDelete: () => void;
}) {
  return (
    <div>
      <h2>Delete note</h2>
      <p>This moves the note to Hatchdoor trash using the current content hash.</p>
      {error ? <p className="note-editor-error">{error}</p> : null}
      <UiButton onClick={onDelete}>Delete</UiButton>
      <UiButton className="close-note" onClick={onClose}>Cancel</UiButton>
    </div>
  );
}
```

- [ ] **Step 4: Wire actions in App and Topbar**

In `AppTopbar`, add props:

```ts
  onNewNote: () => void;
  onRenameNote: () => void;
  onMoveNote: () => void;
  onDeleteNote: () => void;
```

Add menu items for write-enabled state:

```tsx
{writeEnabled ? (
  <UiButton className="close-note" role="menuitem" onClick={() => { onCloseActionsMenu(); onNewNote(); }}>
    New note
  </UiButton>
) : null}
{activeNote && writeEnabled ? (
  <UiButton className="close-note" role="menuitem" onClick={() => { onCloseActionsMenu(); onRenameNote(); }}>
    Rename note
  </UiButton>
) : null}
{activeNote && writeEnabled ? (
  <UiButton className="close-note" role="menuitem" onClick={() => { onCloseActionsMenu(); onMoveNote(); }}>
    Move note
  </UiButton>
) : null}
{activeNote && writeEnabled ? (
  <UiButton className="close-note" role="menuitem" onClick={() => { onCloseActionsMenu(); onDeleteNote(); }}>
    Delete note
  </UiButton>
) : null}
```

In `App.tsx`, import:

```ts
import { NoteActionsDialog, type NoteActionDialogKind } from "./components/NoteActionsDialog";
import { createNote, deleteNote, moveNote, renameNote } from "./writeApi";
```

Add state:

```ts
const [noteActionDialog, setNoteActionDialog] = useState<NoteActionDialogKind | null>(null);
const [noteActionError, setNoteActionError] = useState<string | null>(null);
```

Add handlers that call the write API and then `refreshVault()`:

```ts
const handleCreateNote = useCallback(async (relativePath: string, content: string) => {
  setNoteActionError(null);
  try {
    const outcome = await createNote(relativePath, content);
    setNoteActionDialog(null);
    await refreshVault();
    if (outcome.slug) navigate(`/n/${encodeURIComponent(outcome.slug)}`);
  } catch (error) {
    setNoteActionError(error instanceof Error ? error.message : "Create failed");
  }
}, [navigate, refreshVault]);
```

Add `contentHash?: string` to `ActiveNoteMeta` in `frontend/src/types.ts`:

```ts
export type ActiveNoteMeta = {
  title: string;
  slug: string;
  relativePath: string;
  exportContent?: string;
  contentHash?: string;
};
```

Set it in `NotePage`:

```ts
onActiveNoteChange({
  title: note.title,
  slug: note.slug,
  relativePath: note.relative_path,
  exportContent: stripVaultNoteLinks(parsed.body),
  contentHash: note.content_hash,
});
```

Add these handlers in `App.tsx`:

```ts
const requireActiveNoteHash = useCallback(() => {
  if (!activeNote?.slug || !activeNote.contentHash) {
    throw new Error("Current note is not ready for write actions");
  }
  return { slug: activeNote.slug, contentHash: activeNote.contentHash };
}, [activeNote]);

const handleRenameNote = useCallback(async (newTitle: string) => {
  setNoteActionError(null);
  try {
    const { slug, contentHash } = requireActiveNoteHash();
    const outcome = await renameNote(slug, newTitle, contentHash);
    setNoteActionDialog(null);
    await refreshVault();
    if (outcome.slug) navigate(`/n/${encodeURIComponent(outcome.slug)}`);
  } catch (error) {
    setNoteActionError(error instanceof Error ? error.message : "Rename failed");
  }
}, [navigate, refreshVault, requireActiveNoteHash]);

const handleMoveNote = useCallback(async (targetFolder: string) => {
  setNoteActionError(null);
  try {
    const { slug, contentHash } = requireActiveNoteHash();
    const outcome = await moveNote(slug, targetFolder, contentHash);
    setNoteActionDialog(null);
    await refreshVault();
    if (outcome.slug) navigate(`/n/${encodeURIComponent(outcome.slug)}`);
  } catch (error) {
    setNoteActionError(error instanceof Error ? error.message : "Move failed");
  }
}, [navigate, refreshVault, requireActiveNoteHash]);

const handleDeleteNote = useCallback(async () => {
  setNoteActionError(null);
  try {
    const { slug, contentHash } = requireActiveNoteHash();
    await deleteNote(slug, contentHash);
    setNoteActionDialog(null);
    await refreshVault();
    navigate("/");
  } catch (error) {
    setNoteActionError(error instanceof Error ? error.message : "Delete failed");
  }
}, [navigate, refreshVault, requireActiveNoteHash]);
```

- [ ] **Step 5: Render dialog**

Add near the bottom of `App.tsx`:

```tsx
{noteActionDialog ? (
  <NoteActionsDialog
    kind={noteActionDialog}
    error={noteActionError}
    onClose={() => {
      setNoteActionDialog(null);
      setNoteActionError(null);
    }}
    onCreate={(relativePath, content) => void handleCreateNote(relativePath, content)}
    onRename={(newTitle) => void handleRenameNote(newTitle)}
    onMove={(targetFolder) => void handleMoveNote(targetFolder)}
    onDelete={() => void handleDeleteNote()}
  />
) : null}
```

- [ ] **Step 6: Add dialog CSS**

Add to `frontend/src/App.css`:

```css
.modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 80;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1rem;
  background: var(--overlay-bg);
}

.modal-panel {
  width: min(520px, 100%);
  max-height: min(720px, calc(100vh - 2rem));
  overflow: auto;
  border: 1px solid var(--rule);
  border-radius: 8px;
  background: var(--paper);
  padding: 1rem;
}

.modal-panel form {
  display: grid;
  gap: 0.75rem;
}

.modal-panel label {
  display: grid;
  gap: 0.35rem;
}

.modal-panel input,
.modal-panel textarea {
  width: 100%;
  border: 1px solid var(--rule);
  border-radius: 6px;
  padding: 0.65rem;
  background: var(--paper);
  color: var(--ink);
}
```

- [ ] **Step 7: Run tests and typecheck**

Run: `npm --prefix frontend test -- --run frontend/src/App.write-mode.test.tsx`

Expected: PASS.

Run: `npm --prefix frontend run typecheck`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/App.tsx frontend/src/app/AppTopbar.tsx frontend/src/components/NoteActionsDialog.tsx frontend/src/App.write-mode.test.tsx frontend/src/App.css frontend/src/types.ts
git commit -m "feat(frontend): add note management dialogs" -m "- Add create, rename, move, and delete dialogs to the existing actions menu." -m "- Navigate and refresh after successful note management writes."
```

## Task 6: Security Review Fixes

**Files:**
- Modify based on Security Reviewer findings, expected files:
  - `src/handlers/write_api.rs`
  - `src/main.rs`
  - `frontend/src/App.tsx`

- [ ] **Step 1: Invoke Security Reviewer**

Use Security Reviewer role to inspect:

```text
Security Reviewer: Review the frontend write mode implementation for path traversal, CSRF risk, unauthenticated write exposure, content hash enforcement, stale delete behavior, and public deployment warnings. Focus only on security and hardening findings.
```

- [ ] **Step 2: Apply required hardening**

Expected minimum hardening if not already present:

```rust
// In write_capabilities_handler warning text:
"Frontend writes are enabled and this deployment does not require a web bearer token. Do not expose this service to untrusted networks."
```

Confirm create, move, and move-rename only call vault write functions that normalize paths inside the vault. Confirm rename rejects `/` and `\` in `new_title`.

- [ ] **Step 3: Add regression test for traversal rejection**

Add in `src/main.rs`:

```rust
#[tokio::test]
async fn write_api_rejects_note_path_traversal() {
    let (app, _tmp) = app_for_tests();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/note")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"relative_path":"../escape.md","content":"bad"}"#))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 4: Run security regression tests**

Run: `cargo test write_api_rejects_note_path_traversal --bin hatchdoor`

Expected: PASS.

Run: `cargo test write_api_ --bin hatchdoor`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/handlers/write_api.rs src/main.rs frontend/src/App.tsx
git commit -m "fix(write-mode): harden web note writes" -m "- Add security review hardening for unauthenticated write warnings and traversal coverage." -m "- Keep content-hash checks on update and delete paths."
```

If Security Reviewer finds no code changes are needed, skip this commit and record the review result in the implementation summary.

## Task 7: Final Verification

**Files:**
- No planned source changes unless verification finds a defect.

- [ ] **Step 1: Run backend verification**

Run: `cargo test --bin hatchdoor`

Expected: PASS.

- [ ] **Step 2: Run frontend verification**

Run: `npm --prefix frontend run typecheck`

Expected: PASS.

Run: `npm --prefix frontend test`

Expected: PASS.

Run: `npm --prefix frontend run lint`

Expected: PASS.

- [ ] **Step 3: Check git status**

Run: `git status --short`

Expected: clean working tree after all commits.

- [ ] **Step 4: Provide final implementation summary**

Include:

- Backend endpoints added.
- Frontend flows added.
- Security review result.
- Exact verification commands and outcomes.
- Final commit list.
