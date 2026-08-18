---
tags: [type/explanation, topic/vault-organization, topic/agent-workflow]
---

# Why keep a second brain

The pitch for a [[The Second Brain method (external reference)|second brain]] — an external, trusted place for the things worth keeping — doesn't depend on Hatchdoor at all. Plain Markdown files in any folder already satisfy it: durable, portable, readable without special software. What changes with Hatchdoor is who's allowed to act on that store.

## A second brain built for one reader

Most note-taking advice, including Forte's CODE method and Luhmann's slip-box, assumes a single author who is also the only future reader: you capture something, and later *you* organize, distill, and use it. The system's whole design optimizes for that one relationship — a human writing to their future self.

## What changes when an agent can read and write it too

An MCP-connected agent — see [[Connect your agent]] — is a second party with the same read/write access to the Vault a human has, gated by the same guarded tools described in [[MCP tools reference]]. That changes what the mechanical steps of keeping a second brain cost:

- **Capture** stays mostly human — an agent doesn't decide what's worth keeping on your behalf, but it can take a source you hand it and file it as a note without you doing the typing.
- **Organize and Distill** get cheaper. Rewriting a rough note into something concise, merging two notes on the same topic, or filling in cross-links between related pages are exactly the kind of mechanical, well-specified edits an agent can carry out — and does, in the [[How to run an LLM wiki in Hatchdoor|LLM-wiki workflow]], as an ongoing habit rather than a one-off cleanup.
- **Express** can start from what the Vault already contains. An agent asked for a summary or a report searches the Vault first, the same way it's expected to in [[Search and change notes with your agent]], rather than reconstructing an answer from nothing.

None of this requires trusting the agent unsupervised. Every write is optimistic-concurrency-checked against the note's current version, every note stays plain Markdown you can read without Hatchdoor at all, and [[Browse and review through the Web UI|reviewing what changed]] is a normal part of the workflow, not an afterthought.

## Why this doesn't require a specific layout

Nothing above depends on organizing the Vault one particular way. [[The PARA method (external reference)|PARA]], [[The Zettelkasten method (external reference)|Zettelkasten]], and [[The LLM wiki pattern (external reference)|the LLM-wiki pattern]] all describe a different answer to "how should notes be filed and linked" — an agent works the same guarded tools regardless of which one a Vault follows, or none at all. Pick a layout because it fits how you think, not because Hatchdoor asks for it.

---

Related: [[The Second Brain method (external reference)]] · [[The PARA method (external reference)]] · [[The Zettelkasten method (external reference)]] · [[The LLM wiki pattern (external reference)]] · [[MCP tools reference]] · [[The security model]]
