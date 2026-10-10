# Legacy single-Vault upgrade

Hatchdoor keeps its Vaults as ordinary, UUID-addressed definitions in
`/data/state/vaults.json`, the Vault registry. This file is authoritative
instance state. It must persist across container or binary upgrades; it is not
part of the disposable SQLite cache.

Releases 2.4.x and earlier had no registry. They served one Vault, configured
through `VAULT_PATH`, `HATCHDOOR_EXCLUDE` and the `HATCHDOOR_GIT_*` settings.
Releases 2.5.0 to 2.7.x converted such a deployment into the registry on their
first start. Hatchdoor 2.8.0 removed that conversion (ADR-40, #427).

## Minimum direct-upgrade version

**Hatchdoor 2.8.0 and later upgrade directly only from 2.5.0 or later.** An
install still on 2.4.x or earlier upgrades in two steps:

1. Upgrade to any release from 2.5.0 to 2.7.x and start it once. It converts
   the single Vault into the registry. Its own upgrade notes, in this file at
   that release's tag, describe what it imports and how to recover when it
   cannot.
2. Upgrade to 2.8.0 or later.

An install that skips the first step and has no registry but still stores any
of the retired Git settings (`HATCHDOOR_GIT_SYNC_ENABLED`, `_HTTPS_TOKEN`,
`_HTTPS_USERNAME`, `_REMOTE`, `_BRANCH`, `_DEBOUNCE_SECONDS`) in its settings
file refuses to start, with a log message naming this document.
Nothing is written: the Markdown, the Git repository and the settings file are
left for the 2.5.0 to 2.7.x release to import. Opening on zero Vaults instead
would look like lost notes.

A single-Vault install configured only through environment variables, or
storing only `HATCHDOOR_EXCLUDE` or the commit author, cannot be told apart from
a fresh install: a current install may store those settings too. It starts with no
Vaults. Its Markdown is untouched; add the folder as a Vault in Settings, or
upgrade through a 2.5.0 to 2.7.x release first to carry its Git settings over.

## Before upgrading

Back up the Markdown Vault, its `.git` directory when present, the current
settings file, and `/data/state` when it already exists. Docker Compose mounts
`${HOST_STATE_PATH:-./data/state}` at `/data/state`. Operators running the
binary directly must persist `/data/state` themselves.

The rootless image writes the registry as numeric user/group `65532:65532`.
Create a bind-mounted state directory before starting Docker so it is not
auto-created as root-owned:

```bash
mkdir -p data/state
chmod 700 data/state
sudo chown 65532:65532 data/state
```

For rootless Podman, use `podman unshare chown 65532:65532 data/state` instead.
Use the corresponding directory when `HOST_STATE_PATH` is customized.

## A start with no registry

A start that finds no registry, and no stored single-Vault settings, writes an
empty one and opens on zero Vaults, whatever `VAULT_PATH` holds and whether or
not it is set. The folder `VAULT_PATH` names is never registered as a Vault by
itself and never written to. It stays valid configuration: it is the folder
Hatchdoor can see Vaults in, and every Vault is one you added.

## Leftover settings

Every start on an existing registry removes the retired Git-lane keys
(`HATCHDOOR_GIT_SYNC_ENABLED`, `_HTTPS_TOKEN`, `_HTTPS_USERNAME`, `_REMOTE`,
`_BRANCH`, `_DEBOUNCE_SECONDS`) from stored settings again, so a plaintext Git
token never survives there. A failed removal is logged and retried on the next
start. `HATCHDOOR_EXCLUDE` and the two author keys are left alone; the author
keys are still the commit identity of a Vault without its own.

Per-Vault values set in the environment have no effect: each Vault keeps its
own settings in the registry. Hatchdoor logs a warning at startup naming any
that are still set. Remove them from `.env`, and change a Vault's settings in
Settings or with the `edit_vault` MCP tool. `VAULT_PATH` is exempt, because
Docker Compose sets it on every deployment as the container's Vault mount.

The development-only `HATCHDOOR_VAULT_SOURCE` and `HATCHDOOR_VAULT_GIT_*`
startup-source variables were never part of a released deployment contract.
If they are present, Hatchdoor refuses to start and names them; remove them,
restart, and create or edit the Git-backed registry Vault instead.

## Retired legacy routes

The instance-wide status routes that described the single Vault are removed, and
answer `404`:

| Retired route | What replaces it |
| --- | --- |
| `GET /api/index-status` | `GET /api/v1/vaults` — each Vault reports its own `search` condition and last search error |
| `GET /api/git-status` | `GET /api/v1/vaults` — each Vault reports its own `git` condition and last versioning error |
| `GET /api/vault-status` | `GET /api/startup-status` for process startup; `GET /api/v1/vaults` for a Vault's own state |

`GET /api/startup-status` is unchanged and stays unauthenticated. The two
Settings consoles those routes fed, **Search index** and **Versioning**, are
retired with them; per-Vault settings pages carry the same information.

`PATCH /api/settings` loses the `git_init` and `git_downgrade` consequences for
the same reason: they described the instance-wide versioning lifecycle no
registry deployment runs. `reindex` is unchanged, and an unknown value in
`confirm` is still refused as a validation error.

## Downgrade

Downgrading to a single-Vault release after the registry has been committed is
unsupported. Older releases do not understand the registry authority and may
resume legacy single-Vault behavior. Restore the pre-upgrade backups and the
matching older configuration instead of pointing an older binary at the
converted deployment.
