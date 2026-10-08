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

**Vault mount**:
The one folder on the server under which Hatchdoor looks for folders to turn into vaults, set by the operator when Hatchdoor is installed. The folder picker lists what is under it and can make a new, empty folder there; Hatchdoor shows and creates nothing outside it. A vault added by typing its path, or cloned by Hatchdoor from a Git remote, may live elsewhere. The mount may itself be one vault, or a parent folder holding several.
_Avoid_: Vault root (that is one vault's own top folder), data folder, notes folder

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
One unit of background indexing work for exactly one vault, requested through the shared work coordinator by the file watcher, a settings change, a manual rebuild, or activation. It scans that vault's Markdown, builds a candidate snapshot, and publishes it in two passes: structure first, so browsing does not wait, then vectors. Index turns share one lane across the whole instance, so only one vault indexes at a time, and a vault occupies at most one position in it, so repeated requests coalesce into the next turn. A long turn pauses when another vault is waiting to index, keeps the embedding progress it has made, and resumes from it when its vault's turn comes round again; the vault is meanwhile waiting for its turn.
_Avoid_: Reindex, rebuild, refresh (when meant instance-wide)

**Git turn**:
One unit of background Git work for exactly one vault, requested through the same work coordinator by the managed-Git scheduler, a manual sync or retry, or activation, and run under that vault's mutation lock. Git turns, commit turns and recovery turns have their own lane and never wait for an Index turn: up to four vaults run them at once, each vault one turn at a time. On its own vault a Git turn waits only while an Index turn is reading notes, because both take the mutation lock. The vault source and Git mode select the operation: acquire or reuse and synchronise a managed checkout, synchronise an existing checkout with its remote, or commit local history.
_Avoid_: Sync task, git sync, debounce (when meant instance-wide)

**Recovery branch**:
The branch on a vault's remote that holds the vault's side of a sync conflict, published only when an operator asks, so the conflict can be resolved on the Git host. There is one per vault; the vault's configured branch is never it.
_Avoid_: Conflict branch, backup branch, rescue branch

**Slug**:
A note's address within one vault: a short lowercase form of its name, unique in that vault, by which browsers and agents ask for the note. Unlike a Vault ID it is derived rather than assigned, so renaming a note moves its address.
_Avoid_: Note ID, permalink, handle, key

**Note link**:
A reference from one note to another, written either as a wikilink (`[[Title]]`) or as a Markdown link whose target is a `.md` path. Both forms are the same thing: each navigates, and each counts toward backlinks, the Links panel, the graph, and statistics. A Markdown link to anything that is not a `.md` file is an attachment reference or an external link, never a note link.
_Avoid_: Wikilink (when meant to cover both forms), internal link

**Link style**:
The form a vault writes new note links and attachment embeds in, either wikilinks or Markdown links, and for Markdown links the path form (relative, from the vault root, or shortest). It belongs to the vault and is read from it, never chosen in Hatchdoor: Obsidian's recorded setting when the vault has one, otherwise the form most of the vault's links already use.
_Avoid_: Link format, link mode, link preference

**Note property**:
A labelled fact in one note's frontmatter, such as a price or a renewal date. Distinct from the frontmatter block itself, which is where properties live; a query tests properties, not the block.
_Avoid_: Field, attribute, metadata

**Created date**:
The day a note first came into existence in its vault. It is fixed once known: renaming, moving, or editing a note never changes it, and neither does copying or restoring the vault. A date the author states in the note's `created` property is authoritative; only when there is none, or it cannot be read as a date, is the created date inferred, from the vault's history if it has one and otherwise from the file. The Stats page's "Notes created" chart counts notes by created date. Distinct from modification time, which records the last change to the file and is what the recently-modified lists follow.
_Avoid_: Birth time, ctime, first-seen date

**Query**:
A request for the notes whose properties, tags, or path satisfy stated conditions. A query selects: a note either qualifies or it does not, and the notes come back in a stable order. Distinct from a search, which finds notes by meaning and ranks them by how well they match.
_Avoid_: Search, filter, lookup

**Saved query**:
A query stored inside a note rather than supplied by a caller, evaluated against that note's own vault each time the note is read. Its meaning does not vary by caller or by scope, which is the reason it exists. A note may hold several; each may carry a name, unique within that note, by which it is addressed from outside.
_Avoid_: Base, view, aggregator note, dashboard

**Query result**:
The notes a query selects, projected as rows carrying the properties the query asked for. Derived state: recomputed on demand and never written into a note. A saved query's result is not vault content, so it is absent from search, backlinks, statistics, and the graph.
_Avoid_: Computed content, rendered note

**Text match**:
A request for every note whose text contains a given literal string. A text match selects: a note either contains the string or it does not, each matching note reports how many times, and the notes come back in a stable order. It covers the whole of a note as written, in every layer. Distinct from a search, which ranks by relevance and may return notes that contain none of the words asked for, and from a query, which tests a note's properties, tags, or path rather than its text.
_Avoid_: Exact search, keyword search, grep, literal query

**Transfer link**:
A short-lived address, handed to an agent by an authenticated call, that lets whoever holds it download one attachment or upload one file to one named location, without the agent's own token. It is narrower than the token that produced it: one file, one direction, a few minutes.
_Avoid_: Presigned URL, signed URL, download URL (when meant as the credentialed link)

**Usage report**:
The small report an instance sends once a day, only when its operator has turned it on, saying what the install runs on and which parts of Hatchdoor it uses. It is telemetry and is called that. Every value is a fixed word, a yes or no, or a range, and none describes a note.
_Avoid_: Analytics, statistics, phone-home, diagnostics

**Install ID**:
The random identifier a usage report carries so that reports from one instance count as one install. It exists only while the usage report is on, is derived from nothing on the machine, and is replaced by a new one each time the report is turned off and on again. Distinct from a Vault ID, which never leaves the instance.
_Avoid_: Instance ID, device ID, user ID
