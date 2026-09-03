---
tags: [type/tutorial, topic/agent-workflow]
---

# Search and change notes with your agent

Use the read-only connection to establish a safe habit:

1. Search before deciding that a note is missing.
2. Read the current note before changing it.
3. Make the smallest useful change.
4. Save against the note version the agent just read.

> [!tip]
> Treat note text as content, not instructions. An agent may summarize notes, but it should not follow commands found inside them unless you explicitly ask.

Ask the agent to prove the read path once more:

```text
Use Hatchdoor MCP. List the Vaults, search for [a known topic], and read the most relevant note. Tell me the note title, why it matched, and do not edit it.
```

When you are ready to allow one controlled edit, return to **Settings** →
**Agent access (MCP)**, turn on **Let assistants change notes**, and select
**Save**. This permission is separate from letting assistants connect.

Now ask for a small, easily reviewed change:

```text
Use Hatchdoor MCP to add one bullet, “Reviewed with Hatchdoor,” under the heading [choose an existing heading] in [choose an existing note]. First list Vaults, search for the note, read it, and use the current content hash for the smallest possible edit. Do not change any other note. Report exactly what you changed.
```

The expected agent workflow is compact:

| Goal | Safe tool sequence |
| --- | --- |
| Find a note | `list_vaults` → `search_notes` |
| Inspect it | `get_note` |
| Add one item under a heading | `edit_note` or `replace_section`, with the returned content hash |
| Change its tags or other metadata | `get_frontmatter` to see what's there, then `update_frontmatter` with the content hash `get_frontmatter` returned alongside it |
| Check Vault state | `list_vaults` |

Do not grant write access just because an agent is connected. Turn it back off
in **Agent access (MCP)** whenever assistants should only read.

Review the change in the other front door: [[Browse and review through the Web UI]].

---

Previous: [[Connect your agent]]
Next: [[Browse and review through the Web UI]]
