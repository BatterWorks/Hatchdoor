---
tags: [type/reference, topic/configuration]
---

# Usage report reference

The usage report is telemetry, and it is off unless you turn it on. While it is on, your Hatchdoor sends one small report a day that says the install is alive, what it runs on and which parts of Hatchdoor it uses. It never says anything about your notes.

This page lists every field the report carries. **Settings** → **Usage report** shows the exact report your own install would send, whether the report is on or off, so you can check it against this page before you decide.

## Where you are asked

Hatchdoor mentions the usage report in five places, once in each, and never reminds you afterwards. Wherever you meet it, the answer is the same: it is off until you turn it on.

- **The setup checklist** of a new install has a switch for it, off, below the one for hearing about new releases.
- **The README**, for people who install by hand, names the setting under Configuration. It is left out of `.env.example`, the settings file you copy, so that file never suggests an answer.
- **An agent that installs Hatchdoor for you** asks about it with its other questions, following [[How to deploy Hatchdoor with an agent]]. It turns the report on only if you say yes.
- **[[What's new]]** says so in the release that added it, for people who upgrade.
- **Your agent, once.** On an install that was upgraded to this release, the first agent to connect over MCP afterwards is told that the report exists, and asked to tell you. That is why an agent may bring it up unprompted. It is told one time only, whatever it does with it, and it cannot turn the report on: no MCP tool reads or changes this setting. An install where the report is already on, or set in `.env`, tells its agents nothing.

## What it is used for

The reports show how many installs exist and how they are set up. The maintainer uses them to decide which platforms to test and which parts of Hatchdoor people rely on. Each field below is there because it answers one of those two questions.

Figures from the reports may be quoted as totals where they explain a decision, in a release note or an issue. They are never quoted per install, and there is no public page of numbers.

## Every field

The report is a fixed list. Every value is a word from a short list, a yes or no, or a range. No value is free text, and no count is exact.

| Field | Values | What it says |
| --- | --- | --- |
| `id` | A random ID | The install ID, described under [[#The install ID]]. It lets two reports from the same install be counted as one install. |
| `schema` | `1` | Which version of this field list the report follows. |
| `version` | Such as `2.8.0` | The Hatchdoor release the install runs. |
| `channel` | `stable`, `dev` | Whether it is a published release or a development build. A copy you built from source yourself is `dev`. The exact build is not sent. |
| `os` | Such as `linux` | The operating system Hatchdoor itself runs on. Inside Docker that is Linux, whatever the computer runs. |
| `arch` | Such as `x86_64`, `aarch64` | The processor type. |
| `image` | `docker`, `podman`, `source` | Which published image the install runs, or `source` for anything else. |
| `search_model` | `gemma`, `nomic`, `none` | Which search model is chosen. `none` means no model is chosen yet. |
| `vaults` | `0`, `1`, `2-3`, `4+` | How many Vaults are turned on. |
| `notes` | `0`, `1-99`, `100-999`, `1k-9k`, `10k+` | How many notes those Vaults hold in total. |
| `git_sync` | `none`, `pull_only`, `two_way`, `mixed` | How the Vaults that sync with a Git remote do it. `none` means no Vault has a remote. `mixed` means they do not all use the same mode. |
| `mcp_enabled` | yes or no | Whether agents may connect. |
| `mcp_writes` | yes or no | Whether agents may change notes. |
| `mcp_active_7d` | yes or no | Whether an agent called Hatchdoor in the last seven days. |
| `web_active_7d` | yes or no | Whether anyone used the web app in the last seven days. |
| `agent_claude_code` | yes or no | Whether Claude Code connected in the last 30 days. |
| `agent_claude_desktop` | yes or no | Whether Claude Desktop, or another Claude app, connected in the last 30 days. |
| `agent_codex` | yes or no | Whether Codex connected in the last 30 days. |
| `agent_cursor` | yes or no | Whether Cursor connected in the last 30 days. |
| `agent_vscode` | yes or no | Whether Visual Studio Code connected in the last 30 days. |
| `agent_chatgpt` | yes or no | Whether ChatGPT connected in the last 30 days. |
| `agent_openclaw` | yes or no | Whether OpenClaw connected in the last 30 days. |
| `agent_hermes` | yes or no | Whether Hermes connected in the last 30 days. |
| `agent_other` | yes or no | Whether any other agent connected in the last 30 days. |

The report travels in the event format of the analytics service that receives it, so Settings also shows a few fixed words around these fields: `type`, `website`, `hostname`, `url` and `name`. They are the same for every install. `hostname` is always the word `hatchdoor`, never the name of your machine.

A field is never added quietly. A new field means a new `schema` number, a change to this page and a line in [[What's new]], and it shows in Settings before the first report that carries it.

## What is never sent

- Note content, titles, paths or tags.
- Search queries.
- Vault names or Vault IDs.
- Git remote addresses.
- The name or address of your machine, as part of the report.
- Any token or password.
- Your language or time zone.
- Exact counts of anything.
- The name an agent gives itself. Hatchdoor matches that name against the list of agents above, keeps only which one it was, and throws the name away.
- What you do: no counts per tool, no clicks, no errors.

## The install ID

The install ID is a random number made the first time the report is turned on. It is made from nothing on your machine: no host name, no hardware address, no path. Its only job is to let reports from the same install be counted once. Because the same ID goes with every report, the report is pseudonymous: it does not name you, but two reports from your install can be told to be from the same install.

While the report is on, Settings shows the ID, and Hatchdoor keeps it in `instance.json` in its state folder, with the last day an agent called, the last day the web app was used, the last day each kind of agent connected, and the time the last report was sent. That is everything Hatchdoor keeps for the report.

Turning the report off deletes all of it at once, the ID included. Turning it on again makes a new ID, so the new reports cannot be tied to the old ones.

One thing in that file is not part of the report and stays when the report is off: on an install that was upgraded, a mark saying its agents have been told the report exists, so they are not told twice. It is a yes and nothing else.

## When it is sent

- **The first report** goes within a minute of turning the report on.
- **After that**, at most one report in any 24 hours. Hatchdoor keeps the time of the last report that got through, so restarting it does not send another.
- **If a report does not get through**, Hatchdoor tries again an hour later. It gives each try ten seconds and never holds up startup or anything you are doing. Nothing is saved to send later: a report describes the install as it is now, so a day that was missed stays missed. A failure is written to the log only at debug level.
- **While the report is off**, Hatchdoor never contacts the collector at all.

While the report is on, **Settings** → **Usage report** shows **Last report** with how long ago the last one got through, or **None sent yet**. The time counts for one install ID: turn the report off and on again and the new ID sends its first report within a minute.

## Where the report goes

The report goes to `https://telemetry-hatchdoor.battercloud.cc`, a name used for nothing else, so you can recognise it in a network log and block it if you choose. It is run by Hatchdoor's maintainer, not by a third-party analytics company.

Like the Hatchdoor demo and documentation sites, that address is reached through Cloudflare. Every web request carries the sender's IP address, and this one does too. The address is discarded before anything is stored, so it is never kept beside your install ID.

Each report is one `POST` to `https://telemetry-hatchdoor.battercloud.cc/v1/report`. It carries the user-agent `Hatchdoor`, with no version, and the report shown in Settings as its body. Hatchdoor reads nothing from the answer beyond whether the report got through, and does not follow a redirect to another address. No setting sends the report anywhere else.

## Turn it on or off

- In **Settings** → **Usage report**, switch **Send a usage report** on or off and save. It takes effect straight away, with no restart.
- Or set `HATCHDOOR_USAGE_REPORT_ENABLED=true` in your `.env` and restart. A value set there always wins, and Settings then shows the switch as locked.

Turning it off stops the reports at once. Nothing is sent as a goodbye. A public demo instance never reports and never makes an install ID, even with the variable set.

See [[Settings and environment variables reference]] for the setting itself, and [[The security model#The usage report]] for how it sits beside Hatchdoor's other outbound requests.
