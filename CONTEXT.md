# Hatchdoor

Hatchdoor provides browser and agent access to one or more local Markdown
vaults.

## Language

**Vault**:
A local directory containing the Markdown notes and related assets that
Hatchdoor serves. A vault is local regardless of whether Git backs it.
_Avoid_: Remote vault, Git repository

**Vault ID**:
An immutable identifier unique within one Hatchdoor instance, used by agent
operations, URLs, and configuration to refer to a vault.
_Avoid_: Vault name

**Vault name**:
A human-readable, changeable label shown for a vault in the UI.
_Avoid_: Vault ID

**Git-backed vault**:
A vault whose local contents are versioned and backed up through Git, with an
optional remote repository used for synchronization.
_Avoid_: Git vault, remote vault

**Vault scope**:
The vault or vaults against which a browser or agent operation is performed. A
scope may identify one vault or all available vaults. Agent operations declare
their scope explicitly; mutations always identify exactly one vault.
_Avoid_: Active vault

**Vault tree**:
One vault's folders and notes as a nested structure, grouped per vault and never merged across vaults. Each folder reports the notes held directly inside it, excluding its subfolders. The tree states its vault once; the notes inside it are not separately vault-qualified, unlike the flat results of a search or a recently-modified read.
_Avoid_: Folder tree, file tree, explorer tree (when meant as the returned structure)

**Tree scope**:
The part of a vault tree a read returns: the folder it starts from, how far below that it descends, and whether notes appear at all. Distinct from vault scope, which selects vaults rather than content; a tree read declares both. A tree scope naming a folder the vault does not have is refused, never answered with an empty folder.
_Avoid_: Vault scope, path filter, subtree filter

**Aggregated view**:
A combined view of results from multiple vaults that preserves each result's
vault-qualified identity. It does not merge vaults or create cross-vault links.
_Avoid_: Merged vault, unified vault

**Vault collection**:
The persistent set of vaults connected to one Hatchdoor instance and managed
from Hatchdoor itself. Removing an entry disconnects the vault without deleting
its local files or remote repository.
_Avoid_: Vault layer

**Vault definition**:
The configuration that connects a vault to Hatchdoor, including its local
location and optional Git backing. Definitions persist in the instance vault
collection and are managed through the UI or authenticated MCP.
_Avoid_: Vault settings

**Vault source**:
The way Hatchdoor obtains a vault's local Markdown directory: a local directory,
an existing Git checkout, or a managed Git checkout cloned from a remote.
_Avoid_: Remote vault, Git repository as vault

**Degraded vault**:
A usable local vault with an impaired supporting capability, such as Git
synchronization or indexing. The impairment is reported explicitly without
locking away otherwise available local content.
_Avoid_: Unavailable vault, broken vault

**Layer**:
A named classification of content within one vault. A vault contains layers;
the collection of vaults is not itself another layer.
_Avoid_: Vault layer

**Index turn**:
One unit of background indexing work for exactly one vault, requested through the shared work coordinator by the file watcher, a settings change, a manual rebuild, or activation. It scans that vault's Markdown, builds a candidate snapshot, and publishes it in two passes: structure first, so browsing does not wait, then vectors. A vault occupies at most one coordinator position, so repeated requests coalesce into the next turn.
_Avoid_: Reindex, rebuild, refresh (when meant instance-wide)

**Git turn**:
One unit of background Git work for exactly one vault, requested through the same work coordinator by the managed-Git scheduler, a manual sync or retry, or activation, and run under that vault's mutation lock. The vault source and Git mode select the operation: acquire or reuse and synchronise a managed checkout, synchronise an existing checkout with its remote, or commit local history.
_Avoid_: Sync task, git sync, debounce (when meant instance-wide)
