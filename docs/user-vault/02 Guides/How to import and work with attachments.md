---
tags: [type/how-to, topic/attachments]
---

# How to import and work with attachments

An attachment is any file in a Vault that is not Markdown and that a note refers to. This page covers adding attachments, from the Web UI and from an agent, and managing them afterwards. For the embed syntax, see [[Supported Markdown reference#Images and PDFs]].

Three lists of file types apply, and people often mix them up:

| | Which types | Why it is drawn there |
| --- | --- | --- |
| What may be **uploaded** | `png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `bmp`, `pdf` | Hatchdoor's rule for what may enter a Vault through it. |
| What Hatchdoor will **manage** once present | anything that is not Markdown | A Vault is a plain folder you also edit in Obsidian and sync with Git, so it holds video, audio, data and archives whatever Hatchdoor accepts as an upload. Those files are yours, and Hatchdoor can list, move, rename and delete them. |
| What Hatchdoor will **fetch or display** | `png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`, `avif`, `bmp`, `pdf` | What the Web UI can render and `get_attachment` can hand back. |

So a `.mp4` screen recording already in your Vault can be listed, moved, renamed and deleted, and it moves with its note. It cannot be uploaded through Hatchdoor, and `get_attachment` refuses to return it. Open it from the filesystem or in Obsidian.

## What may be uploaded

Every upload, from the browser or an agent, is held to the same file types and the same size limits, checked on the server:

| | Allowed on upload | Default limit |
| --- | --- | --- |
| Extensions | `png`, `jpg`, `jpeg`, `gif`, `webp`, `avif`, `bmp`, `pdf` | |
| HTTP upload (Web UI, upload links, and agents holding a token) | | `HATCHDOOR_MAX_ATTACHMENT_BYTES`, 10 MiB |
| Agent downloads (download links and `get_attachment` base64), and the MCP base64 upload fallback `import_attachment` | | `HATCHDOOR_MCP_MAX_BASE64_BYTES`, 5 MiB decoded |

You can change both limits in **Settings → Uploads** without a restart. See [[Settings and environment variables reference]].

## From the Web UI

Paste or drop an image or a PDF into a note you are editing. Hatchdoor:

1. Uploads it into an `Attachments/` folder at the Vault root. If a file with that name already exists, it numbers the new one (`report-1.pdf`, `report-2.pdf`, ...) and never overwrites.
2. Writes an embed in the Vault's link style, on a line of its own under the line your caret is on or the line you dropped the file on. **Source** writes it at the cursor instead. A wikilink Vault gets `![[...]]` with a relative path that leads back to the Vault root, even from a note several folders deep. A Markdown-link Vault gets `![](...)`, with the path written in the Vault's path form. See [[Supported Markdown reference#Which link style Hatchdoor writes]].

A file of an unsupported type, or one over the limit, is refused in the editor with the reason: the wrong extension, or how many MB over the limit. No part of it is uploaded.

## From an agent, over MCP or HTTP

An agent has three ways to upload, and should check which one applies first:

```text
Call get_attachment_import_config with the target vault_id first. It reports
whether uploads are currently possible for that Vault, which method(s) are
available, their byte limits, and the allowed extensions. Check this instead
of guessing, since it can differ per Vault (a pull_only Git Vault, for
instance, never accepts writes).
```

| Method | When to use it | How |
| --- | --- | --- |
| `create_upload_link` (MCP tool), then send the file to the link | The default. Use it whenever the client can make an HTTP request, including `curl` from a shell-capable agent. It needs no token and no server address. An agent inside an MCP client usually has neither. | Call `create_upload_link` with `vault_id`, `target_relative_path`, and optionally `overwrite`. It returns `upload_url`; `POST` the file there as `multipart/form-data` in a field named `file`, for example `curl -F file=@scan.pdf '<upload_url>'`. The link is good for one upload to exactly that path and expires after five minutes. |
| `POST /api/v1/vaults/{vault_id}/attachments` | Only for a client that holds a bearer token and knows the server's address itself | `multipart/form-data` with fields `target_relative_path` and `file`. Accepts either the web bearer token or a live MCP bearer token. |
| `import_attachment` (MCP tool) | The fallback, for clients that cannot make an HTTP request outside MCP | `content` (base64), `target_relative_path`. The file travels inside the JSON-RPC message, which becomes unreliable as files approach the base64 size limit. Use an upload link whenever you can. |

All three return the same shape: `vault_id`, `attachment`, `rewritten_notes`, `trashed_path`, `cleanup_warning`. None of them adds the embed to a note, so write the returned `attachment.relative_path` into the note yourself, in the Vault's link style. `list_vaults` reports it on each Vault as `link_style` (`wikilink` for `![[...]]`, `markdown` for `![](...)`) and `link_path_form`. Hatchdoor writes what you send as it is and never converts it.

### Importing a Markdown file as a note

An agent that already has a Markdown file on disk, say a report another tool wrote, should not read it and retype it through `create_note`. That pays for the content twice and depends on the model copying every line. Send it through an upload link instead: give `create_upload_link` a target ending in `.md`, such as `Imports/Report.md`, and `POST` the file to the link it returns with `curl -F file=@report.md '<upload_url>'`. The `.md` target makes it a note upload, which bypasses the extension list above.

The file is written the way `create_note` writes a note. Line endings become LF and a final newline is added if missing, and `quality_warnings` says so, so a byte comparison against the source has to allow for both. A file that is not UTF-8 text, or contains a NUL byte, is refused and nothing is written. The answer is a note-write result with the new note's `slug` and `content_hash`.

To re-import over a note that already exists, pass `overwrite: true` and `expected_content_hash`, the note's current hash from `get_frontmatter`. The upload then replaces the note only if nobody has changed it since you read that hash, and otherwise fails with `write_conflict` and leaves their edit alone. The size limit is the same `HATCHDOOR_MAX_ATTACHMENT_BYTES`. The Web UI's drop zone still refuses Markdown files.

> [!tip]
> An agent does not have to use the `Attachments/` folder the Web UI uses. `target_relative_path` is any path inside the Vault. Following the Web UI makes files easy to find by browsing, and an agent with its own filing scheme, such as a folder per note or a `Sources/` layer, works as well.

## Getting one back out

`get_attachment` is the upload flow in reverse, with a link and a base64 fallback. It takes `vault_id`, the `relative_path` as `list_note_attachments` reports it, and an optional `encoding`:

| `encoding` | Returns | When to use it |
| --- | --- | --- |
| `url` (default) | `content.download_url`, a download link, and `content.expires_at` | The default. The link is a full address that carries its own credential, so fetch it as it is, for example `curl -o manual.pdf '<download_url>'`, with no token. It works for that one file, any number of times, for five minutes. Call `get_attachment` again for a new one. It is held to the same size ceiling and rate quota as the base64 path (see [[The security model]]). |
| `base64` | `content.content`, the bytes inline | The fallback, for a client that cannot make an HTTP request outside MCP. Limited by the same `HATCHDOOR_MCP_MAX_BASE64_BYTES` as `import_attachment`. A file over it is refused with its measured size, never cut short. |

`get_attachment` is a read. It works whenever MCP is on, with write mode on or off.

> [!note]
> A link stops working when it expires, when Hatchdoor restarts, when the MCP password changes, or when MCP is turned off. An upload link also stops when **Let assistants change notes** is turned off. Ask for a new one. Behind a proxy or an HTTPS front end, links follow the address the proxy reports in its forwarded headers. If the proxy sends none, or serves Hatchdoor under a path, set **Public address** (`HATCHDOOR_PUBLIC_URL`) so the links point where agents can reach them.

## Managing attachments already in the Vault

These tools act on bytes the Vault already stores, so the upload list does not apply to them: they accept any file that is not Markdown, whatever its extension, including files with no extension at all. Four things are refused. A note, because moving one this way would skip the backlink rewriting and the hash check the note tools do. Use those tools. A folder's `.hatchdoor-layer` marker, because trashing one would change which notes sit on the default surface (see [[The layer system]]). Anything under `.git`, which is the Vault's version history and not your content. And anything inside a folder the Vault ignores, `.obsidian/` included, so an agent tidying attachments cannot reach your Obsidian settings.

| Tool | Does | Write mode |
| --- | --- | --- |
| `list_note_attachments` | Every attachment one note references, without the note's content. Call it before a move or rename to see what the change touches. | Not required |
| `move_attachment` | Moves the file and rewrites every note that referenced it. | Required |
| `rename_attachment` | Renames the file where it is and rewrites every reference. | Required |
| `delete_attachment` | Trashes the file under `.hatchdoor-trash` and rewrites every reference, the way `delete_note` trashes a note (see [[MCP tools reference#Write content tools]]), so you can get the file back from disk. | Required |

Reference rewriting goes by the file extension, so a link to a file with no extension is left as written when that file moves. Rename the file to carry an extension if you want its links to follow it.

A note that is not valid UTF-8 text, such as a Latin-1 export from an older tool, cannot have its references rewritten without damaging the bytes that are not text. If one references the attachment you move, rename or delete, the call is refused with `link_rewrite_unsupported` and nothing is written; the message names each such note. Re-save those notes as UTF-8 and try again.

The three mutating tools need `HATCHDOOR_MCP_WRITE_ENABLED` and the same Vault-level `mutate` capability as any other write; see [[MCP tools reference#Write content tools]] for full parameters. To move, rename, or delete several attachments in one round trip, put them in a `batch` call (see [[MCP tools reference#Batch]]); one item can fail while the others succeed, so read each item's `ok`.

---

Related: [[MCP tools reference]] · [[HTTP API reference]] · [[Supported Markdown reference]] · [[Settings and environment variables reference]] · [[Search and change notes with your agent]]
