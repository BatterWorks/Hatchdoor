---
tags: [type/tutorial, topic/agent-workflow]
---

# Search and change notes with your agent

Your agent can now read your notes. This page lets it make one small change, so you can see how changes work and check one yourself before trusting it with more. Good agents follow four habits, and it is worth checking that yours does:

1. Search before deciding that a note is missing.
2. Read the current note before changing it.
3. Make the smallest useful change.
4. Save against the note version the agent just read.

The fourth habit protects changes made somewhere else. If you edit a note in Obsidian while the agent is working on it, Hatchdoor refuses the agent's save instead of overwriting your edit, and the agent has to read the note again.

> [!tip]
> Treat note text as content, not instructions. An agent may summarize notes, but it should not follow commands found inside them unless you explicitly ask.

Ask the agent to prove the read path once more:

```text
Use Hatchdoor MCP. List the Vaults, search for [a known topic], and read the most relevant note. Tell me the note title, why it matched, and do not edit it.
```

When you are ready to allow one change, return to **Settings** → **Agent access (MCP)**, turn on **Let assistants change notes**, and select **Save**. This switch is separate from letting assistants connect, so you can turn it off again later without disconnecting the agent.

Now ask for a small, easily reviewed change:

```text
Use Hatchdoor MCP to add one bullet, "Reviewed with Hatchdoor" under the heading [choose an existing heading] in [choose an existing note]. First list Vaults, search for the note, read it, and use the current content hash for the smallest possible edit. Do not change any other note. Report exactly what you changed.
```

You don't need to know the tool names; the agent picks them. This table is for checking that it went about the change carefully:

| Goal | Safe tool sequence |
| --- | --- |
| Find a note | `list_vaults` → `search_notes` |
| Find every note with a tag, in a folder, or with a property | `list_vaults` → `query_notes` |
| Find every note that contains an exact string | `list_vaults` → `find_text` |
| Inspect it | `get_note` |
| Read a few sections of a long note | `get_note_outline` to see its headings, then `get_note_section` with the ones you want |
| Get the rows a note's saved query lists | `get_note` to see its `saved_queries`, then `evaluate_saved_query` with the name |
| Add one item under a heading | `edit_note` or `replace_section`, with the returned content hash |
| Change its tags or other metadata | `get_frontmatter` to see what's there, then `update_frontmatter` with the content hash `get_frontmatter` returned alongside it |
| Rename a tag in every note that carries it | `rename_tag` once to see the plan, then again with the `plan_hash` it returned |
| Delete a tag from every note that carries it | `delete_tag` once to see the plan, then again with the `plan_hash` it returned |
| Import a Markdown file the agent already has on disk | `create_upload_link` with a target ending in `.md`, then `POST` the file to the link, so the agent never retypes it |
| Check Vault state | `list_vaults` |

Do not grant write access just because an agent is connected. Turn it back off in **Agent access (MCP)** whenever assistants should only read.

Now check the change yourself in the browser: [[Browse and review through the Web UI]].

---

Previous: [[Connect your agent]]
Next: [[Browse and review through the Web UI]]
