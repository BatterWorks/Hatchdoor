# Security Policy

## Supported Versions

Security fixes target the current `main` branch and the latest published Docker
image, `battermanz/hatchdoor:latest`.

## Deployment Notes

Hatchdoor serves a private Markdown vault. Treat every network-exposed instance
as sensitive:

- Set `HATCHDOOR_WEB_BEARER_TOKEN` before binding to a non-loopback interface.
- Docker Compose binds Hatchdoor to `0.0.0.0` inside the container, so a web
  bearer token is required there.
- Enable MCP only for trusted clients and always set
  `HATCHDOOR_MCP_BEARER_TOKEN`.
- Keep the SQLite cache outside the Markdown vault. The cache is disposable, but
  the vault is the source of truth.
- If git sync is enabled, use a scoped HTTPS token and monitor
  `get_git_sync_status` for sync failures.

## Reporting Vulnerabilities

Open a private security advisory or contact the maintainer before publishing
details. Include affected version, deployment mode, reproduction steps, and
whether the issue can expose or mutate vault content.
