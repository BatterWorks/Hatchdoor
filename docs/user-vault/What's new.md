---
tags: [type/reference]
private: true
---

# What's new

What changed in each Hatchdoor release, in a few plain lines, newest release first. Anything you must do before or after upgrading comes first in its release and starts with **Action needed**. The [full changelog](https://github.com/BatterWorks/Hatchdoor/blob/main/CHANGELOG.md) lists every change.

Agents read this page with `read_docs` and the page name `whats-new`.

## v2.8.2 - 2026-10-14

- The note page and Help no longer show a note's comments, the text between `%%` marks or inside `<!-- -->`, so visitors to a read-only or demo Vault never see them. [[Supported Markdown reference#Comments]]
- An embedded note, `![[Other note]]`, now shows as a link to that note instead of a broken image.
- What's new now shows every release you missed while away from Hatchdoor, not only the newest one.
- Settings now shows an on/off setting as On whenever it is on, including one set in `.env` as `1`, `yes` or `on`.
- Attachment download and upload links now work for a path that starts with `./`, stop working as soon as you change the MCP token, and never appear in the logs.
- An agent installing Hatchdoor from the deploy page now reads the whole page, so it no longer misses the last step that shows you the web token.

## v2.8.1 - 2026-10-11

- **Optional usage report.** Hatchdoor can send its maintainer one small report a day of how your install is set up, and nothing about your notes, to decide which platforms to test and which parts of Hatchdoor people rely on. It is telemetry and stays off unless you turn it on. Settings shows the exact report under Usage report, and every field is listed in [[Usage report reference]]
- The Docker and Podman images now describe themselves as a release in the usage report. The 2.8.0 images reported a development build from source.
- An agent that installs Hatchdoor from the deploy page now always shows you the web token in its last message.

## v2.8.0 - 2026-10-10

- **Action needed:** An install on 2.4.x or earlier must upgrade to a 2.5.0 to 2.7.x release first, and only then to 2.8.0. [[How to upgrade Hatchdoor#Coming from 2.4.x or earlier]]
- **Action needed:** After upgrading, start a new session in every agent that was connected to Hatchdoor, or reconnect it, because until then its searches fail with "Structured content does not match the tool's output schema". [[MCP tools reference#Compact and full search hits]]
- A new install opens on a setup checklist that takes you from nothing to an agent searching your notes, and you can reopen it any time from Help. [[Browse and review through the Web UI#The setup checklist]]
- Hatchdoor can tell you when a newer release is out, if you turn that on in Settings under Updates. [[How to upgrade Hatchdoor#Hear about new releases]]
- **Optional usage report.** Hatchdoor can now send its maintainer one small report a day of how your install is set up, used to decide which platforms to test and which parts of Hatchdoor people rely on. It is telemetry and stays off unless you turn it on in Settings under Usage report. Every field it sends is listed in [[Usage report reference]]
- A note now opens straight into a live editor, so you click anywhere and type, with diagrams, math and callouts drawn as you write. [[How to edit notes with the live editor]]
