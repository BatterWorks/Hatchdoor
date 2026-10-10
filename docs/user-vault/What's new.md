---
tags: [type/reference]
private: true
---

# What's new

What changed in each Hatchdoor release, in a few plain lines, newest release first. Anything you must do before or after upgrading comes first in its release and starts with **Action needed**. The [full changelog](https://github.com/BatterWorks/Hatchdoor/blob/main/CHANGELOG.md) lists every change.

Agents read this page with `read_docs` and the page name `whats-new`.

## v2.8.0 - 2026-10-10

- **Action needed:** An install on 2.4.x or earlier must upgrade to a 2.5.0 to 2.7.x release first, and only then to 2.8.0. [[How to upgrade Hatchdoor#Coming from 2.4.x or earlier]]
- **Action needed:** After upgrading, start a new session in every agent that was connected to Hatchdoor, or reconnect it, because until then its searches fail with "Structured content does not match the tool's output schema". [[MCP tools reference#Compact and full search hits]]
- A new install opens on a setup checklist that takes you from nothing to an agent searching your notes, and you can reopen it any time from Help. [[Browse and review through the Web UI#The setup checklist]]
- Hatchdoor can tell you when a newer release is out, if you turn that on in Settings under Updates. [[How to upgrade Hatchdoor#Hear about new releases]]
- **Optional usage report.** Hatchdoor can now send its maintainer one small report a day of how your install is set up, used to decide which platforms to test and which parts of Hatchdoor people rely on. It is telemetry and stays off unless you turn it on in Settings under Usage report. Every field it sends is listed in [[Usage report reference]]
- A note now opens straight into a live editor, so you click anywhere and type, with diagrams, math and callouts drawn as you write. [[How to edit notes with the live editor]]
