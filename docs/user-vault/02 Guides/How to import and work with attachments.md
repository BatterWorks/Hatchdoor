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
| HTTP upload (Web UI, upload links, and agents holding a token) | — | `HATCHDOOR_MAX_ATTACHMENT_BYTES`, 10 MiB |
| Agent downloads (download links and `get_attachment` base64), and the MCP base64 upload fallback `import_attachment` | — | `HATCHDOOR_MCP_MAX_BASE64_BYTES`, 5 MiB decoded |

Both limits are adjustable at runtime in **Settings → Uploads** — see [[Settings and environment variables reference]].

## From the Web UI

Paste an image, or drag and drop an image or PDF, directly into the note editor. Hatchdoor:

1. Uploads it into a Vault-root `Attachments/` folder, numbering the filename (`report-1.pdf`, `report-2.pdf`, ...) if one with that name already exists rather than overwriting it.
2. Inserts an embed at the cursor in the Vault's link style. A wikilink Vault gets `![[...]]` with a relative path that walks back out to the vault root, even for a note several folders deep. A Markdown-link Vault gets `![](...)`, with the path written in the Vault's path form. See [[Supported Markdown reference#Which link style Hatchdoor writes]].

An unsupported file type or an oversized file is rejected inline with the reason (wrong extension, or how many MB over the limit) — nothing partially uploads.

## From an agent, over MCP or HTTP

An agent has three ways in, and should check which one applies before uploading rather than assuming:

```text
Call get_attachment_import_config with the target vault_id first. It reports
whether uploads are currently possible for that Vault, which method(s) are
available, their byte limits, and the allowed extensions — check this instead
of guessing, since it can differ per Vault (a pull_only Git Vault, for
instance, never accepts writes).
```

| Method | When to use it | How |
| --- | --- | --- |
| `create_upload_link` (MCP tool), then send the file to the link | The default. Use it whenever the client can make an HTTP request, including `curl` from a shell-capable agent. It needs no token and no server address, which an agent inside an MCP client usually does not have. | Call `create_upload_link` with `vault_id`, `target_relative_path`, and optionally `overwrite`. It returns `upload_url`; `POST` the file there as `multipart/form-data` in a field named `file`, for example `curl -F file=@scan.pdf '<upload_url>'`. The link is good for one upload to exactly that path and expires after five minutes. |
| `POST /api/v1/vaults/{vault_id}/attachments` | Only for a client that holds a bearer token and knows the server's address itself | `multipart/form-data` with fields `target_relative_path` and `file`. Accepts either the web bearer token or a live MCP bearer token. |
| `import_attachment` (MCP tool) | The fallback, for clients that genuinely cannot make an out-of-band HTTP request | `content` (base64), `target_relative_path`. Rides inside the JSON-RPC message, so it gets unreliable as files approach the base64 size limit — prefer the HTTP path whenever it's available. |

All three return the same shape: `vault_id`, `attachment`, `rewritten_notes`, `trashed_path`, `cleanup_warning`. Neither creates the embed syntax in a note for you — write the returned `attachment.relative_path` into the note yourself, in the Vault's link style. `list_vaults` reports it on each Vault as `link_style` (`wikilink` for `![[...]]`, `markdown` for `![](...)`) and `link_path_form`. Hatchdoor writes what you send as-is and never converts it.

### Importing a Markdown file as a note

An agent that already has a Markdown file on disk, say a report another tool wrote, should not read it and retype it through `create_note`. That pays for the content twice and depends on the model copying every line. Send it through an upload link instead: give `create_upload_link` a target ending in `.md`, such as `Imports/Report.md`, and `POST` the file to the link it returns with `curl -F file=@report.md '<upload_url>'`. The `.md` target makes it a note upload, which bypasses the extension list above.

The file is written the way `create_note` writes a note. Line endings become LF and a final newline is added if missing, and `quality_warnings` says so, so a byte comparison against the source has to allow for both. A file that is not UTF-8 text, or contains a NUL byte, is refused and nothing is written. The answer is a note-write result with the new note's `slug` and `content_hash`.

To re-import over a note that already exists, pass `overwrite: true` and `expected_content_hash`, the note's current hash from `get_frontmatter`. The upload then replaces the note only if nobody has changed it since you read that hash, and otherwise fails with `write_conflict` and leaves their edit alone. The size limit is the same `HATCHDOOR_MAX_ATTACHMENT_BYTES`. The Web UI's drop zone still refuses Markdown files.

> [!tip]
> There's no requirement to use the Vault-root `Attachments/` folder the Web UI uses — `target_relative_path` is any Vault-relative path you choose. Keeping the Web UI's convention makes files easy to find by browsing, but an agent following its own filing scheme (per-note folders, a `Sources/` layer) works just as well.

## Getting one back out

`get_attachment` is the mirror of the upload flow, with the same two methods and the same tradeoff between them. It takes `vault_id` and the `relative_path` exactly as `list_note_attachments` reports it, and an optional `encoding`:

| `encoding` | Returns | When to use it |
| --- | --- | --- |
| `url` (default) | `content.download_url`, a download link, and `content.expires_at` | The default. The link is a full address that carries its own credential, so fetch it as it is, for example `curl -o manual.pdf '<download_url>'`, with no token. It works for that one file, any number of times, for five minutes; call `get_attachment` again for a fresh one. It is held to the same size ceiling and rate quota as the base64 path (see [[The security model]]). |
| `base64` | `content.content`, the bytes inline | The fallback, for a client that can't make an out-of-band HTTP request at all. Bounded by the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` cap as `import_attachment`; an oversized file is rejected with its measured size rather than truncated. |

Reading an attachment is a read: `get_attachment` works whenever MCP is enabled, with no write mode required.

> [!note]
> A link stops working when it expires, when Hatchdoor restarts, when the MCP password changes, or when MCP is turned off. An upload link also stops when **Let assistants change notes** is turned off. Ask for a new one. If Hatchdoor sits behind a proxy or an HTTPS front end, set **Public address** (`HATCHDOOR_PUBLIC_URL`) so the links point where agents can reach them.

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
