---
tags: [type/how-to, topic/installation]
---

# How to upgrade Hatchdoor

Upgrading means downloading the newer Hatchdoor image and restarting the container on it. Your notes, your Vault list, your settings and the downloaded search model all live in folders outside the container, so they carry over untouched. Hatchdoor never upgrades itself: you decide when.

Before you upgrade, read the release's notes on the [GitHub releases page](https://github.com/BatterWorks/Hatchdoor/releases). A release that needs something from you says so first, under **Action needed**.

## Upgrade with Docker Compose

Open a terminal in the folder that holds your `compose.yaml` (the one you made in [[Install Hatchdoor with Docker Compose]]) and run:

```bash
docker compose pull
docker compose up -d
```

The first command downloads the newest image for the tag your `compose.yaml` names. The second restarts Hatchdoor on it. Give it a minute: it waits for any Git sync in progress to finish before it stops, then starts again and checks your Vaults. Open the web address you normally use and carry on.

If your `compose.yaml` names a fixed version, such as `battermanz/hatchdoor:2.7.0`, change that line to the new version first. The `latest` tag always means the newest release. Podman users run the same two commands with `podman compose` and the `podman-latest` or `podman-<version>` tag.

Want to go back? Put the previous version in the `image:` line and run the same two commands. Your notes are plain Markdown files, so no version can lock you out of them.

## What you see after an upgrade

The first time you open Hatchdoor in a browser after an upgrade, a **What's new** window lists what changed in every release since the version you had, with anything that needs action pinned at the top. Click **Got it** and it stays away in that browser until the next upgrade. A fresh install never shows it.

The same list is in Help, on the [[What's new]] page, whenever you want to read it again.

## Ask your agent to upgrade Hatchdoor

An agent with a terminal on the machine running Hatchdoor, such as Claude Code or Codex, can do the upgrade for you. Give it this sentence:

> Upgrade my Hatchdoor: in the folder with its compose.yaml, read the newest release notes at https://github.com/BatterWorks/Hatchdoor/releases and tell me about anything marked Action needed before you start, then run `docker compose pull` and `docker compose up -d`, and check that the web address answers again.

The agent needs no Hatchdoor token for this: it works with Docker, not with your notes. An agent that only reaches Hatchdoor over MCP cannot upgrade it, because MCP gives no access to Docker.

## Hear about new releases

Hatchdoor can tell you when a newer release is out. This is off until you turn it on, in **Settings** → **Updates** → **Tell me about new releases**, or by setting `HATCHDOOR_UPDATE_CHECK_ENABLED=true` in your `.env`.

While it is on, Hatchdoor asks GitHub's public list of Hatchdoor releases once a day whether a newer one exists. The request carries your server's IP address, as any web request does, and the user-agent `Hatchdoor`. Nothing else goes with it: not your version, your notes, your Vaults, your settings or any token.

When a newer release exists, a line at the top of the page says **Hatchdoor 2.9.0 is available** (with the real version number), with a **What's new** link to that release's notes on GitHub and a **How to upgrade** link back to this page. Close it with **×** and it stays closed in that browser until the next release comes out. If GitHub cannot be reached, the line goes away and Hatchdoor tries again the next day.

Turning it off stops the daily request at once, with no restart. A public demo instance never checks.
