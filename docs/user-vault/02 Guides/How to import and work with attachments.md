---
tags: [type/how-to, topic/attachments]
---

# How to import and work with attachments

An attachment is any non-Markdown file living inside a Vault alongside its Markdown, referenced the ordinary Markdown way. This page covers getting them in (from the Web UI and from an agent) and managing them afterward. For the exact embed syntax, see [[Supported Markdown reference#Images and PDFs]].

Three different sets of file types matter here, and mixing them up is the usual source of confusion:

| | Which types | Why it is drawn there |
| --- | --- | --- |
| What may be **uploaded** | `png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `bmp`, `pdf` | A policy about what may enter a Vault through Hatchdoor. |
| What Hatchdoor will **manage** once present | anything that is not Markdown | A Vault is a plain folder you also edit in Obsidian and sync with Git, so it holds video, audio, data and archives no matter what Hatchdoor accepts on the way in. Those files are yours; Hatchdoor organises them rather than pretending they are not there. |
| What Hatchdoor will **fetch or display** | `png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`, `avif`, `bmp`, `pdf` | What the Web UI can render and `get_attachment` can hand back. |

So a `.mp4` screen recording already in your Vault can be listed, moved, renamed, deleted and carried along when its note moves, but it cannot be uploaded through Hatchdoor, and `get_attachment` will refuse to hand you its bytes. Open it from the filesystem or through Obsidian instead. Widening the upload and fetch lists is a separate decision, not yet made.

## What may be uploaded

Every upload path, browser or agent, is limited to the same file types and enforces the same size caps server-side:

| | Allowed on upload | Default limit |
| --- | --- | --- |
| Extensions | `png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `bmp`, `pdf` | — |
| HTTP upload (Web UI and agents) | — | `HATCHDOOR_MAX_ATTACHMENT_BYTES`, 10 MiB |
| MCP base64 fallback (`import_attachment` in, `get_attachment` out) | — | `HATCHDOOR_MCP_MAX_BASE64_BYTES`, 5 MiB decoded |

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

## Getting one back out

`get_attachment` is the mirror of the upload flow, with the same two methods and the same tradeoff between them. It takes `vault_id` and the `relative_path` exactly as `list_note_attachments` reports it, and an optional `encoding`:

| `encoding` | Returns | When to use it |
| --- | --- | --- |
| `url` (default) | `content.download_url` — a path to resolve against the same scheme, host, and port as the MCP endpoint | The default. Send your MCP bearer token as an `Authorization: Bearer` header; the route accepts it while MCP is enabled, under the same size ceiling and rate quota as the base64 path (see [[The security model]]). The web bearer token also works, as a header or an `access_token` query parameter, and is not subject to those limits. With neither configured, or in demo mode, no credential is needed. |
| `base64` | `content.content`, the bytes inline | The fallback, for a client that can't make an out-of-band HTTP request at all. Bounded by the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` cap as `import_attachment`; an oversized file is rejected with its measured size rather than truncated. |

Reading an attachment is a read: `get_attachment` works whenever MCP is enabled, with no write mode required.

## Managing attachments already in the Vault

These tools act on bytes the Vault already stores, so the upload list does not apply to them: they accept any file that is not Markdown, whatever its extension, including files with no extension at all. Four things are refused. A note, because moving one this way would skip the backlink rewriting and the safety check the note tools do — use those instead. A folder's `.hatchdoor-layer` marker, since trashing one would quietly change which notes sit on the default surface (see [[The layer system]]). Anything under `.git`, which is the Vault's own version history rather than your content. And anything inside a folder the Vault excludes as noise, `.obsidian/` included, so an agent tidying up attachments cannot walk off into your Obsidian configuration.

| Tool | Does | Write mode |
| --- | --- | --- |
| `list_note_attachments` | Every attachment one note references, without pulling the note's full content — useful before deciding whether a move or rename is safe. | Not required |
| `move_attachment` | Moves the file and rewrites every note that referenced it. | Required |
| `rename_attachment` | Renames the file in place and rewrites every reference. | Required |
| `delete_attachment` | Trashes the file under `.hatchdoor-trash` and rewrites every reference — the same trash mechanism `delete_note` uses (see [[MCP tools reference#Write content tools]]), so it's recoverable from disk, not gone. | Required |

One limit worth knowing: reference rewriting keys on the file extension, so a link to a file with no extension at all is left exactly as written when that file moves. Rename the file to carry an extension if you want its links to follow it.

The three mutating tools need `HATCHDOOR_MCP_WRITE_ENABLED` and the same Vault-level `mutate` capability as any other write — see [[MCP tools reference#Write content tools]] for full parameters. To move, rename, or delete several attachments in one round trip, put them in a `batch` call (see [[MCP tools reference#Batch]]); it is best-effort, so read each item's own `ok` rather than assuming the whole set landed.

---

Related: [[MCP tools reference]] · [[HTTP API reference]] · [[Supported Markdown reference]] · [[Settings and environment variables reference]] · [[Search and change notes with your agent]]
