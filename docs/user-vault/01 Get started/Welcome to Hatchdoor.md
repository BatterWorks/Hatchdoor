---
tags: [type/tutorial, audience/self-hoster]
---

# Welcome to Hatchdoor

This short path takes you from nothing to an AI agent that can search your notes. At the end you will have Hatchdoor running on your own computer, your notes added to it, and an agent connected that can read them. You can then let the agent make one small change and check it in the browser.

Hatchdoor runs on your machine and keeps your notes as ordinary Markdown files. It does not send them to a cloud service, and it does not replace the app you write notes in: you can keep using Obsidian or any other Markdown editor on the same folder. [[What Hatchdoor is]] explains the idea in more detail.

## What you need

- [ ] A computer that can run Docker, with Docker Compose. On Windows and macOS that means [Docker Desktop](https://www.docker.com/products/docker-desktop/).
- [ ] A folder of Markdown notes, or an empty folder if you are starting from nothing.
- [ ] An AI agent that supports MCP, such as Claude Code, Codex, OpenClaw or Hermes.

> [!note]
> Hatchdoor starts with no notes in it, and it does not add example notes. To see what a full Hatchdoor looks like first, browse the [public demo](https://hatchdoor.battercloud.cc).

> [!tip]
> Rather have an agent do the installing? Give it [[How to deploy Hatchdoor with an agent]] and it can set everything up for you.

## The path

1. [[Install Hatchdoor with Docker Compose]]. You start Hatchdoor, sign in with the web token, and choose a search model.
2. [[Connect your first Vault]]. You pick the folder that holds your notes.
3. [[Connect your agent]]. You give your agent a read-only connection.
4. [[Search and change notes with your agent]]. You ask it to find a note, then allow one small change.
5. [[Browse and review through the Web UI]]. You check that change in the browser.
6. [[Understand where your data lives]]. You learn what to back up.

Steps 2 to 4 are also on the **Set up Hatchdoor** checklist that a fresh install opens on, so you can follow along there.

Before filing lots of real notes, it is worth five minutes on [[Why keep a second brain]] and one way to organize them: [[The PARA method (external reference)|PARA]], [[The Zettelkasten method (external reference)|Zettelkasten]], or [[The LLM wiki pattern (external reference)|the LLM wiki pattern]]. None of it is needed to install.

---

Previous: [[Home]]
Next: [[Install Hatchdoor with Docker Compose]]
