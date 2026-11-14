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
- For a Git-backed Vault, use an HTTPS token scoped to that one repository.
  Watch for sync failures in the Vault's Git console under Settings, or in the
  `git` and `git_error` fields of each Vault from MCP `list_vaults` or
  `GET /api/v1/vaults`. Hatchdoor sends the token only to the scheme, host and
  port of the Vault's repository URL, once per attempt; a redirect to another
  host gets no token, and a rejected token is not retried until someone acts.

## Network reach of Git Vaults

Anyone who can create or edit Vaults, through the Web UI, the HTTP API or an
MCP client with write access, can make the server open an HTTPS connection to
any host they name as a repository URL. That includes private and LAN
addresses (`127.0.0.1`, `10.0.0.0/8`, `192.168.0.0/16`, the container
network) and hosts that redirect elsewhere. Hatchdoor deliberately keeps no
address allow-list or block-list, because self-hosted Git servers on the LAN
are a normal setup. Only give Vault-management access to people you would let
reach those addresses from the server, or restrict the container's outbound
network if that is a concern.

## Reporting Vulnerabilities

Open a private security advisory or contact the maintainer before publishing
details. Include affected version, deployment mode, reproduction steps, and
whether the issue can expose or mutate vault content.
