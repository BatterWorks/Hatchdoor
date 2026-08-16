# Migrating agents to Vault-scoped clients

Multi-Vault Hatchdoor intentionally removes every implicit or default Vault.
HTTP and MCP clients must discover Vaults and declare the target of every
Vault-dependent operation.

This is a breaking client-contract change. There are no legacy unscoped routes,
no missing-scope MCP fallback, and no selected, sole, or migration-pinned Vault.

## Migration checklist

1. Call `GET /api/v1/vaults` or MCP `list_vaults` and retain immutable
   `vault_id` values. Do not use changeable Vault names as identifiers.
2. Treat a Note's identity as `{ vault_id, slug }`. Store and pass both values.
3. For collection reads, pass one Vault ID or the literal `"all"` as `scope`.
4. For exact reads, mutations, refreshes, and other Vault controls, pass exactly
   one `vault_id`. The literal `"all"` is invalid for these operations.
5. Handle scoped response envelopes, including `partial`, `participants`, and
   `collection_revision`.
6. Branch on structured error `code` values, never on human-readable messages.
7. Replace saved Note URLs with `/v/{vault_id}/n/{slug}`.

## Discover Vaults first

HTTP:

```http
GET /api/v1/vaults
Authorization: Bearer <web-token>
```

MCP:

```json
{
  "name": "list_vaults",
  "arguments": {}
}
```

Discovery remains authenticated and available when no Vault is ready. Returned
definitions and status are safe to display: credentials are never returned;
only `credential_configured` is exposed.

## HTTP before and after

Unscoped routes are removed:

```text
Before: GET /api/note/home
After:  GET /api/v1/vaults/550e8400-e29b-41d4-a716-446655440000/notes/home

Before: GET /api/search?q=solar
After:  GET /api/v1/vaults/all/search?q=solar

Before: POST /api/note
After:  POST /api/v1/vaults/550e8400-e29b-41d4-a716-446655440000/notes
```

The same Vault prefix applies to links, wikilink resolution, attachments,
assets, downloads, tree, recent Notes, statistics, graph, and write-capability
routes. Existing authentication, query-token fallback, optimistic content
hashes, and safe-write semantics remain in force.

`GET /api/v1/vaults/{vault_id}/stats/detail` returns the rich, exact
single-Vault statistics report (word/tag/link counts, top tags, most-linked
notes, activity by month, notes per folder, longest/shortest notes, orphan and
no-tag notes, and notes modified this week/month) as
`{ "vault_id": "...", "stats": { ... } }`. It is a distinct route from the
collection-scope `GET /api/v1/vaults/{scope}/stats` above, which stays lean
(`note_count`, `tag_count`, `link_count`, `vault_size_bytes` per participating
Vault) for one-or-all reads; `stats/detail` never accepts `"all"`.

The unscoped `POST /api/refresh` remains retired. To request a rebuild for one
enabled Vault with usable local Markdown, use the authenticated control route:

```http
POST /api/v1/vaults/550e8400-e29b-41d4-a716-446655440000/refresh
Authorization: Bearer <web-token>
```

It returns `202 Accepted` immediately and never builds a snapshot in the HTTP
request. The response identifies the exact Vault and whether the shared FIFO
added the Index turn or joined one already pending:

```json
{
  "vault_id": "550e8400-e29b-41d4-a716-446655440000",
  "schedule": "queued"
}
```

`schedule` may instead be `"coalesced"`. The literal `"all"`, an omitted
Vault ID, and every legacy unscoped refresh form are invalid; clients must not
poll for this route. Read the normal Vault-scoped search/status projections to
observe the resulting fresh, stale, or unavailable state. `diagnostics`
remains retired with no Vault-scoped replacement.

## MCP before and after

The supported MCP catalogue is deliberately Vault-scoped. Retired scope-less
tools (`query_notes`, `refresh_index`, `layer_diagnostics`, and
`get_git_sync_status`) have no compatibility aliases. Surviving collection
reads gain a required `scope`; exact reads, mutations, write-capability checks,
and existing-Vault controls gain a required `vault_id`.

Broad search across every enabled Vault:

```json
{
  "name": "search_notes",
  "arguments": {
    "scope": "all",
    "query": "solar panel research"
  }
}
```

Search one Vault:

```json
{
  "name": "search_notes",
  "arguments": {
    "scope": "550e8400-e29b-41d4-a716-446655440000",
    "query": "solar panel research"
  }
}
```

Fetch or update an exact Note:

```json
{
  "name": "get_note",
  "arguments": {
    "vault_id": "550e8400-e29b-41d4-a716-446655440000",
    "slug": "home"
  }
}
```

```json
{
  "name": "update_note",
  "arguments": {
    "vault_id": "550e8400-e29b-41d4-a716-446655440000",
    "slug": "home",
    "content": "# Home\n",
    "expected_content_hash": "<hash returned by get_note>"
  }
}
```

Collection-shaped MCP reads such as `search_notes`, `get_tree`, `get_stats`,
`get_graph`, and `recently_modified` accept `scope`. Exact Note/link/resolve
operations and every mutation or control operation on an existing Vault require
`vault_id`. `get_attachment_import_config` survives the migration and now takes
one `vault_id`: its HTTP method reports the Vault-scoped upload path, and its
`enabled` field accounts for both instance-wide MCP write mode and that Vault's
own mutation capability. `create_vault` is the revisioned collection-creation
exception: the
registry assigns its immutable ID after the successful create, which callers
discover with `list_vaults`.

## Note identity and results

Every Note-bearing result includes `vault_id` and `slug`, even when the request
already named one Vault. Duplicate slugs or overlapping information in different
Vaults remain distinct results.

Search and recent-Note results are flattened across Vaults; every item retains
its Vault ID. Trees, statistics, and graphs remain grouped by
Vault, and graph edges never cross Vaults.

## Scoped responses and partial results

Collection-shaped reads use one envelope for a single Vault or `"all"`:

```json
{
  "scope": "all",
  "collection_revision": 42,
  "partial": true,
  "participants": [
    { "vault_id": "...", "state": "fresh" },
    { "vault_id": "...", "state": "stale" },
    {
      "vault_id": "...",
      "state": "unavailable",
      "error": {
        "code": "vault_unavailable",
        "message": "Human-readable explanation",
        "retryable": true
      }
    }
  ],
  "data": {}
}
```

`"all"` means all enabled Vaults. Disabled Vaults do not participate. A partial
all-Vault read still succeeds with available or stale results and identifies
Vaults that could not contribute. A one-Vault read may return a stale previous
snapshot; when it has no usable data, it returns `vault_unavailable` rather than
a misleading empty result.

`registry_revision` belongs to persisted Vault-definition concurrency.
`collection_revision` belongs to live definition, content, index, Git, and
capability changes. Clients must not treat them as interchangeable.

## Structured errors

HTTP and MCP share stable domain error codes such as `invalid_scope`,
`vault_not_found`, `vault_disabled`, `vault_unavailable`, and
`capability_unavailable`. HTTP preserves the existing general status meanings:
malformed requests use `400`, missing resources use `404`, state or concurrency
conflicts use `409`, and temporary unavailability uses `503`. Existing security,
body, media-type, and internal-error statuses remain unchanged.

For `POST /api/v1/vaults/{vault_id}/refresh`, malformed or `"all"` IDs return
`400 invalid_vault_id`; a missing Vault returns `404 vault_not_found`; a disabled
Vault or one without usable local Markdown returns `409 vault_disabled` or
`409 capability_unavailable`; and a Vault that cannot currently accept a
background turn returns retryable `503 vault_unavailable`. As a Vault-control
route, refresh returns `403 demo_read_only` in demo mode before any Vault lookup
or scheduling.

On a public demo instance (`HATCHDOOR_DEMO_MODE=true`), every content
mutation and Vault-control route (collection management, manual Git
sync/retry, Markdown mutations, attachment upload, write-capabilities
discovery) refuses with a `403` and the structured code `demo_read_only`
before any state change, rather than being absent. Discovery, events, exact
reads, contained assets/downloads, and one-or-all tree/recent/stats/graph/
search remain reachable with no token. MCP and Git writeback remain
unavailable in demo mode regardless.

A demo serves a narrower projection of the same shapes, because those reads are
unauthenticated:

- `GET /api/v1/vaults` lists only *enabled* Vaults, and each entry omits
  `source`, `archive_folder`, `commit_identity`, and the `*_error` details, and
  reports an empty `exclude_patterns`. Identity, `enabled`, the four status
  fields, `capabilities`, and `credential_configured` are unchanged. Treat
  `source` as optional: it is always present on an authenticated instance and
  always absent on a demo. Branch on the envelope's `demo_mode` boolean rather
  than inferring the posture from a missing field.
- Demoted Notes (those in a `.hatchdoor-layer` directory) do not exist as far as
  a demo is concerned. They are absent from search under every `layers=` value,
  from trees, graphs, recent lists, and statistics counts, and from the outbound
  links of results that do return. Fetching, resolving, or downloading one
  answers the ordinary not-found, the same as a Note that was never there. The
  `layers=` parameter is accepted and ignored rather than rejected, so an
  existing link keeps working.

MCP returns the same domain details in an error tool result. Messages are for
people and may change; automation must branch on `code`.
