# Legacy single-Vault upgrade

The managed-Vault foundation replaces ambient single-Vault configuration with
ordinary, UUID-addressed Vault definitions in `/data/state/vaults.json`. This
file is authoritative instance state. It must persist across container or
binary upgrades; it is not part of the disposable SQLite cache.

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

Keep the existing Vault and cache paths available for the first managed-Vault
startup. Legacy values are resolved with the same precedence as before:
non-empty environment values, then stored settings, then defaults.

## What import does

Import runs only when the registry is absent and there is positive legacy
evidence: Markdown content, a recognized Hatchdoor SQLite cache, Git history
paired with explicit Git configuration, stored Vault-specific settings, or an
explicit non-default `VAULT_PATH`. An empty `./vault` or Compose `/data/vault`
directory by itself is a fresh deployment and does not trigger migration.

A safe import creates one enabled ordinary Vault with a new UUID. It preserves
the legacy directory name, exclusion patterns, local-history or two-way Git
mode, branch, HTTPS remote, and optional credentials that have an ordinary
Vault-definition equivalent. Import only inspects the existing filesystem and
repository: it never seeds, moves, edits, clones, pulls, commits, pushes,
checks out, merges, or otherwise changes Vault content or Git state.

The registry is committed first. Only after that durable commit may Hatchdoor
remove migrated stored settings and the recognized legacy slug-only SQLite
cache. Markdown remains the authority; search stays unavailable until the
per-Vault cache is rebuilt by the later runtime activation step.

Any existing registry, including an intentional zero-Vault registry,
permanently suppresses legacy import. Legacy environment values are ignored
after that point and are reported together so the runtime can emit one
deprecation notice.

## Recovery

When legacy evidence exists but conversion is unsafe, import changes nothing
and reports `legacy_migration_required`. Correct the named path, repository, or
setting and retry. Alternatively, explicitly confirm **Start with no Vaults**;
that writes an intentional empty registry and permanently disables automatic
legacy import. The legacy Vault, Git repository, settings, and cache remain
untouched by a refused conversion.

## Downgrade

Downgrading to a single-Vault release after the registry has been committed is
unsupported. Older releases do not understand the registry authority and may
resume legacy single-Vault behavior. Restore the pre-upgrade backups and the
matching older configuration instead of pointing an older binary at the
converted deployment.
