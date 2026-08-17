---
tags: [type/tutorial, topic/data-safety]
---

# Understand where your data lives

Your Markdown files are the source of truth. Hatchdoor can rebuild search data
from them, so your notes stay usable with other Markdown tools.

| Host-side location | Contents | How to treat it |
| --- | --- | --- |
| `HOST_VAULT_PATH` (default `./vault`) | Markdown notes and attachments | Primary data. Back it up. |
| `HOST_STATE_PATH` (default `./data/state`) | `vaults.json`, including Vault definitions and possible Git HTTPS credentials | Preserve across upgrades; back up as a secret. |
| `HOST_MODELS_PATH` (default `./models`) | Downloaded model and Gemma-terms receipt | Keep to avoid another download and terms step. |
| `HOST_CACHE_PATH` (default `./data/cache`) | SQLite search cache and `settings.json` | Cache can rebuild; preserve `settings.json`. |

> [!warning]
> `settings.json` stores live Settings, including agent-access configuration. It is not a disposable cache file. Keep it with the deployment data and protect it as sensitive configuration.

Hatchdoor keeps destructive operations recoverable:

- [x] Deleting a note moves it and referenced assets to `.hatchdoor-trash`.
- [x] The trash folder is excluded from indexing.
- [x] Archiving moves notes under `90-archive/` by default.

Keep generated cache data outside the Vault. That way a backup or Git history
contains your notes and attachments, not a disposable index.

You have completed the agent-first path. Return to [[Home]] for the growing
documentation map.

---

Previous: [[Browse and review through the Web UI]]
Next: [[Home]]
