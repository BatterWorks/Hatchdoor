---
tags: [type/explanation, topic/vaults]
---

# Vault lifecycle states

This page explains what Hatchdoor means when it describes a Vault's condition, for when a Vault's page in **Settings** or an agent's `list_vaults` reports something you want to understand. If something is wrong and you want the fix, start with [[How to troubleshoot common problems#A Vault won't index or stays in a bad state]].

A Vault has no single "healthy" or "broken" state. Hatchdoor tracks five separate signals plus one on/off switch, because a Vault can be fine in one way and broken in another. Its Git sync can fail while every note still reads and saves normally, for example. Most misreadings of a Vault come from taking one signal for the whole picture.

## `enabled` is the operator's switch, nothing else

Every Vault has one on/off switch, `enabled`, which you set with **Pause Vault** and **Resume Vault** in Settings, or an agent sets with `enable_vault` and `disable_vault`. It answers one question: should this Vault be running? It says nothing about whether the Vault works. An enabled Vault can still be unavailable, with a missing folder or a broken remote, and a paused Vault keeps the state it had when you paused it.

## The five status axes

Once a Vault is enabled, Hatchdoor tracks its condition on five separate axes. They often disagree: a Vault can be readable while still indexing, or have a broken Git remote while its notes read fine.

| Axis | Values | What it answers |
| --- | --- | --- |
| **Activation** | `active`, `disabled`, `unavailable` | Is this Vault running? |
| **Local content** | `read_write`, `read_only`, `unavailable` | Can Hatchdoor read and write the Markdown on disk right now? |
| **Search** | `unavailable`, `indexing`, `browsable`, `ready`, `stale` | What state is the search index in? |
| **Git** | `disabled`, `pending`, `ready`, `unavailable` | Is Git sync working, if the Vault has it? |
| **Watcher** | `running`, `disabled`, `unavailable` | Is the file watcher keeping the index current? |

Beside the axes, `index_turn` says where the Vault's indexing stands in the queue all Vaults share: `running` while it indexes, `waiting` while it is queued behind another Vault or paused part-way to let one through, and absent when no indexing is queued for it. It is kept apart from **Search** on purpose. A Vault that is `waiting` can still be `ready`, `stale` or `browsable`, and search keeps answering from whatever it already has. Waiting is not a fault: the Vault resumes from where it stopped when its turn comes round ([[How indexing and search work#Vaults take turns indexing]]). The sidebar shows **waiting** for such a Vault where it would otherwise show it indexing. A Vault that was already searchable keeps its note count in the sidebar while it waits and while its own reindex runs, since search answers throughout.

**Git** survives a restart for a Vault Hatchdoor polls on a schedule, meaning one with a remote to check. A Vault that last checked cleanly comes back as `ready`, and one whose last check failed comes back as `unavailable` with the reason it showed before. So `pending` means a Vault that has never completed a check, and never one Hatchdoor has forgotten about. A Vault with no remote to poll is different. That is an `existing_git` Vault in `local_history` mode, which only records your own edits. It has no schedule to resume, so it starts as `pending` and settles on its first turn after startup. From then on its Git status reports its commits: it commits a few seconds after each change, and reads `unavailable` if one of those commits fails. `GET /api/v1/vaults` also reports `last_checked_at` and `next_attempt_at` for any Vault with a remote.

**Search** has middle values that are easy to misread:

- `indexing`: the Vault is building its first index, and there is nothing to search yet. A Vault that was already searchable never shows `indexing` during a reindex. It shows `stale`, with `index_turn` `running`.
- `browsable`: the Vault's notes, links and headings are published and current, but it has no vectors yet. You can open and read every note, and semantic search returns nothing. `query_notes` works in full, and so does a note's saved query table, since selecting notes by tag, path or property needs no vectors. A Vault passes through `browsable` once, on its first index, between reading the notes and preparing search by meaning. A later rebuild of a searchable Vault never drops back to it, because search keeps answering from the previous build while the rebuild runs.
- `ready`: fully current, with structure and vectors.
- `stale`: search still works, but what it answers from is one build behind. There are three ways to get here. A newer build is in progress, or paused while it waits its turn behind another Vault. The last build failed. Or a note was written, or a sync pulled changes from the remote, during the build that just finished, so what it published was already behind when it landed. The third is normal during a bulk edit or a migration: every write schedules the next reindex, as does every sync that pulls, and the Vault settles on `ready` once the writing stops. A commit, or a sync that finds nothing new, changes no note and does not make a Vault `stale`. `stale` is not an error. It means what you see might be one build behind.

## What capabilities actually come from

Nine flags decide what the UI shows and what a write over MCP or the API may do: `browse`, `search`, `mutate`, `pull`, `push`, `retry`, `commit`, `sync` and `publish_recovery`. Hatchdoor derives them from the axes above and from the Vault's own definition, never from one axis alone:

- **`browse`**: true whenever local content is `read_write` or `read_only`. It does not depend on the search axis: a Vault in the middle of indexing, or stuck at `browsable`, can still be browsed in full.
- **`search`**: true only for `ready` or `stale`. `browsable` and `indexing` both grant `browse` but not `search`. A reindex of a searchable Vault is `stale`, so the flag stays true from the first index onward unless search itself breaks.
- **`mutate`**: true only when local content is `read_write` and the Vault is not a `pull_only` Git Vault. A `pull_only` Vault never allows local edits, however healthy everything else looks, because an edit would conflict with the next pull.
- **`pull`** / **`push`**: true only when Git status is `ready`, and only for the right Git mode: `pull_only` or `two_way` for pull, `two_way` for push.
- **`commit`** / **`sync`**. `commit` is true when the Vault keeps Git history of its own (`local_history` or `two_way`). `sync` is true when it has a remote to talk to (`pull_only` or `two_way`). Unlike `pull` and `push`, these come from the Vault's definition and not from its current Git status, so they keep their answer while the Vault is failing. That lets the Settings console offer **Commit now** on a Vault with no remote and **Sync now** on one with a remote.
- **`publish_recovery`**. True for a Two-way Vault whose Git error is `managed_git_conflict`: its side of the conflict can be published to a recovery branch on the remote. It follows the Vault's current Git status, so it turns off as soon as a sync resolves the conflict. See [[How to troubleshoot common problems#Resolving a sync conflict]].
- **`retry`**: true if any of the four per-axis error fields (activation, search, git, watcher) is marked retryable. This is what puts a **Try again** button in front of you when a Vault is stuck.

One answer sits outside this scheme on purpose. Whether a Vault can write is the `mutate` capability above. Whether it can write atomically, committing a save as one swap and not as a check followed by a replacement, depends on the filesystem that holds the Vault, and no axis and no part of the definition can tell you. Hatchdoor finds out by trying the operation once when the Vault starts. It reports the answer separately, as `atomic_compare_and_swap` on `GET /api/v1/vaults/{vault_id}/write-capabilities` and as one line per affected Vault in the server log, and never folds it into `local_content` or `mutate`. A Vault that writes without the atomic swap is `read_write` and `mutate: true`, and that is correct: it writes, with a narrower guarantee against an editor outside Hatchdoor. The two stay apart for the same reason as everything else on this page: one signal answers one question.

One exception covers the whole instance. On a public read-only demo (`HATCHDOOR_DEMO_MODE=true`), `GET /api/v1/vaults` reports `mutate`, `pull`, `push`, `retry`, `commit`, `sync` and `publish_recovery` as `false` for every Vault, whatever the axes or the definition say. Nothing about the Vault changed. The demo refuses every write and every Vault-control request with `403 demo_read_only`, so publishing the derived value would offer a visitor a button that cannot work. `browse` and `search` are still derived normally, because those reads do work, and the axes are untouched: a demo Vault on a writable folder still reports local content `read_write`.

## Registry recovery

Registry recovery means the registry file itself, `vaults.json`, fails to load. It is corrupt, or its schema version is unsupported or comes from a newer Hatchdoor than this one. Every request that changes the registry answers `503 vault_registry_recovery_required` until the file is fixed on disk. No action inside the app resolves it, because any fix made from inside Hatchdoor would have to write to the store that is broken. A registry file Hatchdoor cannot read at all, because of a permission or disk error for example, is reported as an error (`500 internal_error`). It is never mistaken for an empty registry, so no change can overwrite the Vaults it holds.

Registry recovery is not a state of one Vault. The registry as a whole cannot tell you about any Vault, which is a different problem from one Vault with a bad Git remote or a locked file.

---

Related: [[Connect your first Vault]] · [[How to set up a Git-backed Vault]] · [[HTTP API reference]]
