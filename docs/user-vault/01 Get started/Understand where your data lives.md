---
tags: [type/tutorial, topic/data-safety]
---

# Understand where your data lives

Your notes are the only thing you cannot get back if you lose them, so they are what to back up. Everything else Hatchdoor keeps is either small settings worth keeping, or search data it can rebuild from your notes.

Each row below is a folder on the computer running Hatchdoor. The defaults are relative to the folder holding your `compose.yaml`.

| Folder (setting in `.env`) | What is in it | What to do with it |
| --- | --- | --- |
| Your notes (`HOST_VAULT_PATH`, default `./vault`) | Your Markdown notes and their attachments | Back it up. This is your data. |
| `./data/state` (`HOST_STATE_PATH`) | The list of your Vaults (`vaults.json`), including any Git sign-in tokens, plus two small bookkeeping files: `vault-runtime.json` (when each Git-backed Vault last checked its remote) and `instance.json` (which version ran before this one, which agent last connected, with the update check on, the newest release it found, and, with the usage report on, its install ID, the last day each part of Hatchdoor was used and when the last report was sent) | Keep it across upgrades. Back it up, and treat it as secret because it can hold tokens. |
| `./data/cache` (`HOST_CACHE_PATH`) | The search data, which Hatchdoor rebuilds from your notes, and `settings.json` | The search data needs no backup. Keep `settings.json`. |
| `./models` (`HOST_MODELS_PATH`) | The downloaded search model and your Gemma terms choice | Keep it to avoid downloading the model again. |

> [!warning]
> `settings.json` holds your Settings, including the MCP password. It is not throwaway cache, even though it sits in the cache folder. Keep it with the rest of the deployment and protect it like a password.

The web token lives in the `.env` file next to `compose.yaml`. Keep that file too.

## Deleting is recoverable

- Deleting a note moves it, together with any attachments kept in its own folder, to a hidden `.hatchdoor-trash` folder inside the Vault. To get it back, move it out of there with your file manager.
- Notes in the trash folder are left out of search.
- Archiving moves a note to `90-archive/` by default. You can change that folder per Vault.

## Keep Hatchdoor's data out of your notes folder

Hatchdoor refuses a Vault folder that is, contains, or sits inside its own state, cache or settings folder. That way a backup or Git history of your notes holds your notes and attachments, not search data that is rebuilt anyway.

You have finished the Get started path. Return to [[Home]] for the rest of the manual, or read [[How to upgrade Hatchdoor]] for how to keep Hatchdoor up to date.

---

Previous: [[Browse and review through the Web UI]]
Next: [[Home]]
