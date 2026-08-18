---
tags: [type/tutorial, audience/self-hoster]
---

# Welcome to Hatchdoor

This tutorial connects a real Markdown Vault to Hatchdoor and lets an agent
work with it safely. You will start with read-only agent access, ask the agent
to find and read a note, explicitly permit one small write, and inspect the
result in the Web UI.

Hatchdoor is not a hosted sync service and does not replace your Markdown app.
It is the agent-first operating layer for notes you keep in ordinary files —
see [[What Hatchdoor is]] for the full picture.

## What you need

- [ ] Docker with Docker Compose
- [ ] A machine where you can run Docker
- [ ] A folder of Markdown notes, or an empty folder for the starter Vault
- [ ] An MCP-capable agent client you trust

> [!note]
> An empty folder is useful for evaluation. For real work, point Hatchdoor at the folder you already use for notes.

You will complete this path:

0. Understand the idea: [[Why keep a second brain]], then pick a way to organize one — [[The PARA method (external reference)|PARA]], [[The Zettelkasten method (external reference)|Zettelkasten]], or [[The LLM wiki pattern (external reference)|the LLM wiki pattern]]. None of it is required before installing, but it's worth five minutes before you start filing real notes.
1. [[Install Hatchdoor with Docker Compose]]
2. [[Connect your first Vault]]
3. [[Connect your agent]]
4. [[Search and change notes with your agent]]
5. [[Browse and review through the Web UI]]
6. [[Understand where your data lives]]

---

Previous: [[Home]]
Next: [[Install Hatchdoor with Docker Compose]]
