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

`refresh` and `diagnostics` are retired with the rest of the unscoped API and
have no Vault-scoped replacement: a working per-Vault refresh needs
`VaultWorkKind::Index` dispatch, which does not exist yet, and a Vault-scoped
diagnostics route needs new per-Vault cache-query domain methods that do not
exist either. Both are adapter-only gaps for a later ticket, not something a
client can work around today.

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
`vault_id`. `create_vault` is the revisioned collection-creation exception: the
registry assigns its immutable ID after the successful create, which callers
discover with `list_vaults`.

## Note identity and results

Every Note-bearing result includes `vault_id` and `slug`, even when the request
already named one Vault. Duplicate slugs or overlapping information in different
Vaults remain distinct results.

Search, metadata-query, and recent-Note results are flattened across Vaults;
every item retains its Vault ID. Trees, statistics, and graphs remain grouped by
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

On a public demo instance (`HATCHDOOR_DEMO_MODE=true`), every content
mutation and Vault-control route (collection management, manual Git
sync/retry, Markdown mutations, attachment upload, write-capabilities
discovery) refuses with a `403` and the structured code `demo_read_only`
before any state change, rather than being absent. Discovery, events, exact
reads, contained assets/downloads, and one-or-all tree/recent/stats/graph/
search remain reachable with no token. MCP and Git writeback remain
unavailable in demo mode regardless.

MCP returns the same domain details in an error tool result. Messages are for
people and may change; automation must branch on `code`.
