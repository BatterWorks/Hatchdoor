---
tags: [type/how-to, topic/attachments]
---

# How to import and work with attachments

Attachments are images and PDFs living inside a Vault alongside its Markdown, referenced the ordinary Markdown way. This page covers getting them in (from the Web UI and from an agent) and managing them afterward. For the exact embed syntax, see [[Supported Markdown reference#Images and PDFs]].

## What's supported

Every upload path, browser or agent, is limited to the same file types and enforces the same size caps server-side:

| | Allowed | Default limit |
| --- | --- | --- |
| Extensions | `png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `bmp`, `pdf` | — |
| HTTP upload (Web UI and agents) | — | `HATCHDOOR_MAX_ATTACHMENT_BYTES`, 10 MiB |
| MCP base64 fallback (`import_attachment`) | — | `HATCHDOOR_MCP_MAX_BASE64_BYTES`, 5 MiB decoded |

Both limits are adjustable at runtime in **Settings → Uploads** — see [[Settings and environment variables reference]].

## From the Web UI

Paste an image, or drag and drop an image or PDF, directly into the note editor. Hatchdoor:

1. Uploads it into a Vault-root `Attachments/` folder, numbering the filename (`report-1.pdf`, `report-2.pdf`, ...) if one with that name already exists rather than overwriting it.
2. Inserts the right embed syntax at the cursor, with a relative path that walks back out to the vault root correctly even for a note several folders deep.

An unsupported file type or an oversized file is rejected inline with the reason (wrong extension, or how many MB over the limit) — nothing partially uploads.

## From an agent, over MCP or HTTP

An agent has two ways in, and should check which one applies before uploading rather than assuming:

```text
Call get_attachment_import_config with the target vault_id first. It reports
whether uploads are currently possible for that Vault, which method(s) are
available, their byte limits, and the allowed extensions — check this instead
of guessing, since it can differ per Vault (a pull_only Git Vault, for
instance, never accepts writes).
```

| Method | When to use it | How |
| --- | --- | --- |
| `POST /api/v1/vaults/{vault_id}/attachments` | The default — prefer this whenever the client can make an HTTP request (including `curl` from a shell-capable agent) | `multipart/form-data` with fields `target_relative_path` and `file`. Accepts either the web bearer token or a live MCP bearer token, so an MCP agent doesn't need separate web credentials. |
| `import_attachment` (MCP tool) | The fallback, for clients that genuinely cannot make an out-of-band HTTP request | `content` (base64), `target_relative_path`. Rides inside the JSON-RPC message, so it gets unreliable as files approach the base64 size limit — prefer the HTTP path whenever it's available. |

Both return the same shape: `vault_id`, `attachment`, `rewritten_notes`, `trashed_path`, `cleanup_warning`. Neither creates the embed syntax in a note for you — write the returned `attachment.relative_path` into the note yourself, as a link or with `![[...]]` embed syntax, the same way you'd write any other Markdown.

> [!tip]
> There's no requirement to use the Vault-root `Attachments/` folder the Web UI uses — `target_relative_path` is any Vault-relative path you choose. Keeping the Web UI's convention makes files easy to find by browsing, but an agent following its own filing scheme (per-note folders, a `Sources/` layer) works just as well.

## Managing attachments already in the Vault

| Tool | Does |
| --- | --- |
| `list_note_attachments` | Every attachment one note references, without pulling the note's full content — useful before deciding whether a move or rename is safe. |
| `move_attachment` | Moves the file and rewrites every note that referenced it. |
| `rename_attachment` | Renames the file in place and rewrites every reference. |
| `delete_attachment` | Trashes the file under `.hatchdoor-trash` and rewrites every reference — the same trash mechanism `delete_note` uses (see [[MCP tools reference#Write content tools]]), so it's recoverable from disk, not gone. |

All four require `HATCHDOOR_MCP_WRITE_ENABLED` and the same Vault-level `mutate` capability as any other write — see [[MCP tools reference#Write content tools]] for full parameters.

---

Related: [[MCP tools reference]] · [[HTTP API reference]] · [[Supported Markdown reference]] · [[Settings and environment variables reference]] · [[Search and change notes with your agent]]
