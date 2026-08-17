# Hatchdoor user documentation handover

## Scope and decisions

- The documentation targets Hatchdoor **v2.5.0** from the `development` branch and should be published with that release.
- Hatchdoor is presented as an **agent-first notes app**. The getting-started journey introduces the agent early and begins with read-only access.
- Public documentation will run in a **dedicated Hatchdoor instance**, separate from both the application and the evaluation demo.
- The documentation instance will be browser-only and read-only. MCP access to the documentation may be considered in a later version.
- The canonical Markdown source is [`docs/user-vault`](user-vault/).
- Publication will initially be manual and may be automated later.
- The documentation site should open `Home.md` by default. Until Hatchdoor has a configurable landing note, the reverse proxy can redirect `/` to the stable Home note URL.
- Installation must use a standalone Docker Compose file. Users should not need to clone the source repository.
- The Compose example binds to `127.0.0.1` by default. An adjacent optional configuration explains how to publish the port on all interfaces for trusted-LAN access.
- Agent setup includes configuration examples for Claude Code, Codex, OpenClaw, and Hermes.
- Screenshots are intentionally deferred. Pages should use Hatchdoor's supported Markdown features—callouts, tables, diagrams, links, and clear hierarchy—to remain visually useful.

## Current Vault

The public documentation Vault currently contains:

1. `Home.md`
2. `Welcome to Hatchdoor.md`
3. `Install Hatchdoor with Docker Compose.md`
4. `Connect your first Vault.md`
5. `Connect your agent.md`
6. `Search and change notes with your agent.md`
7. `Browse and review through the Web UI.md`
8. `Understand where your data lives.md`

Together these form the initial getting-started tutorial. The Vault is registered as **Docs** in the local development server, which watches these files directly.

## Missing documentation

The current Vault is only the getting-started foundation. It still needs a broader information architecture and the following material.

### Tutorials

- A realistic end-to-end agent workflow beyond the introductory read-only test
- A collaborative workflow in which an agent proposes or makes changes and the user reviews them in the Web UI
- A recovery or migration tutorial if those are expected first-release workflows

### How-to guides

- Add, disable, reconnect, and manage multiple Vaults
- Configure and operate Git-backed Vaults
- Import and work with attachments
- Back up and restore the authoritative Markdown and persistent Hatchdoor state
- Upgrade Hatchdoor safely
- Put Hatchdoor behind a reverse proxy and provide secure remote access
- Diagnose permissions, indexing, search, model-download, browser-login, and MCP connection problems

### Reference

- Every supported setting and environment variable
- MCP tools, parameters, permissions, errors, and concurrency/version behavior
- Search modes, scope, filters, and result behavior
- Supported Markdown, frontmatter, links, embeds, and attachments
- Persistent state, cache, model, and Vault paths
- Relevant HTTP endpoints if the HTTP API is intended as a supported user-facing contract

### Explanations

- What “agent-first” means in Hatchdoor
- Why Markdown is authoritative while the SQLite index is disposable
- How indexing and hybrid search work at a user-understandable level
- The security model, including the separate web token and MCP password
- The difference between local, managed-Git, enabled, disabled, healthy, and degraded Vaults

## Release and deployment work still required

- Review every page against the final v2.5.0 behavior and terminology before publication.
- Provision the dedicated read-only documentation instance.
- Mount or deploy `docs/user-vault` into that instance.
- Preserve its Vault registry and stable Vault UUID.
- Configure the root-to-Home reverse-proxy redirect.
- Confirm public navigation, direct note links, mobile rendering, search, and read-only behavior.
- Define and document the initial manual publication checklist.
- Later, automate publication without allowing the deployed instance to overwrite the canonical repository files.
- Decide how documentation for older Hatchdoor versions will be retained once releases move beyond v2.5.0.

## Important constraints

- Do not merge these pages into the evaluation demo Vault.
- Do not make the public documentation instance writable.
- Do not treat the generated index/cache as documentation source data.
- Do not expose Hatchdoor directly to the public internet without an authenticated, encrypted access layer.
- Keep secrets out of documentation, prompts, screenshots, and committed client configuration.
