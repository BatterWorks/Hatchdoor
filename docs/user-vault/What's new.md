---
tags: [type/reference]
private: true
---

# What's new

What changed in each Hatchdoor release, in a few plain lines, newest release first. Anything you must do before or after upgrading comes first in its release and starts with **Action needed**. The [full changelog](https://github.com/BatterWorks/Hatchdoor/blob/main/CHANGELOG.md) lists every change.

Agents read this page with `read_docs` and the page name `whats-new`.

## v2.8.0 - 2026-10-10

- **Action needed:** **A fresh install now starts with no Vaults, and installs from 2.4.x or earlier can no longer upgrade directly.** Hatchdoor no longer turns the folder `VAULT_PATH` names into a Vault by itself, whatever it holds: a start with no Vault registry writes an empty one and opens on **No Vaults Yet**, and every Vault is one you added in Settings, through `POST /api/v1/vaults` or with `create_vault`.
- **Action needed:** **An agent session that stayed open across the upgrade can no longer search until it is restarted.** `search_notes` hits are compact by default in this release and no longer carry `content`, `outbound_links`, `metadata` or `chunk_id`.
- The Docker image now says where it comes from.
- Hatchdoor now carries its own manual, and agents can read it.
- The signed-in app can now ask which folders Hatchdoor sees under its Vault mount, the folder `VAULT_PATH` names, so a folder picker can offer them instead of asking for a container path.
- Every instance now serves its manual as plain Markdown at open addresses, so you can tell an agent "Read <your instance>/docs/deploy.md and install Hatchdoor for me" and an ordinary web fetch is enough.
- Hatchdoor now remembers which version it ran before an upgrade, and every release will come with a few plain lines on what changed.
- Settings now shows whether your agent actually got through.
- Help now opens beside whatever you are doing.
- After an upgrade, the app now tells you what changed.
- The screens where people get stuck now link straight to the page of the manual that explains them.
- Hatchdoor can now tell you when a newer release is out, if you ask it to.
- **Add a Vault** now lets you pick your notes folder from a list instead of typing a path inside the container.
- A new install now opens on a setup checklist that takes you from nothing to an agent searching your notes.
- The manual has three new pages carried over from the old starter notes.
- **Add a Vault** can now make the folder for a new Vault, so starting one from nothing no longer needs a shell or a script.
- Hatchdoor can now show you the usage report it would send, before anything is sent.
- Hatchdoor now sends the usage report once a day, and only if you turned it on.
- Hatchdoor now asks about the usage report once in each place you might set it up, and never reminds you.
- Agents can now ask which notes contain an exact string.
- Agents can now read part of a long note instead of all of it.
- A link to a public demo now shows a preview when you share it.
- The formatting toolbar over a selection in the live editor has three more buttons: heading, bullet and to-do.
- The live editor now renders what the Reading view renders.
- A note on a Vault Hatchdoor can write to now opens straight into a live editor over its whole body, the way Obsidian's Live Preview works.
- The note page reads from the top.
- The sidebar changed shape.
- The landing page with no note open is a home page.
- The **…** menu is about the open note: **Rename**, **Move**, **Copy note text**, **Download .md file**, **Copy note link**, **Archive** and **Delete**.
- Hatchdoor no longer writes starter notes into a new Vault.
- The manual's getting-started pages, guides and concept pages are rewritten for people new to Hatchdoor.
- The agent deploy guide, served at `/docs/deploy.md`, is rewritten so a beginner can install Hatchdoor by giving their agent one line: "Read https://hatchdoor.battercloud.cc/docs/deploy.md and install Hatchdoor for me".
- A screen reader now says what each **How does this work?** link explains.
- The agent deploy guide, served at `/docs/deploy.md`, fixes four places where an installing agent was left to guess or told you something untrue.
- The agent deploy guide, served at `/docs/deploy.md`, fixes three things a full install on Windows turned up.
- `search_notes` hits are now compact by default, so an agent pays for locating a note, not for reading ten chunks of it.
- The browser tab now names the page you are on.
- The manual was checked page by page against 2.8.0 and reworded in plainer language, with shorter sentences and no em dashes.
- The access token prompt now looks like the rest of Hatchdoor and follows the light and dark themes.
- The manual's editing guide covers more of the live editor.
- A note is now set in a narrower column, about 75 characters a line, where it used to run the full width of the page, about 120 characters on a wide screen.
- Dropping or pasting a file into a note now follows the upload limit you set.
- The Docker verification stage now includes the link-preview image, so the test checks the real asset's dimensions and size.
- Inline code is readable again while editing a note in the light theme.
- A Vault with Git sync no longer stays marked `stale` when its index is current.
- Search results spell a numbered folder's path the right way round.
- Search no longer says **Could not load** above a list of results.
- The sidebar no longer names a Vault as "did not answer" while that Vault's notes are listed right above the sentence.
- Searching the manual for a tool name, a setting or an error code now finds the pages that mention it.
- Reading a note now returns its tags, aliases and other properties.
- The manual names the browser's controls the way they now read.
- The usage report now tells a release from a development build correctly.
- Running the test suite in Docker with `docker build --target verification .` works again.
- Notes changed from Windows now show up when Hatchdoor runs under Docker Desktop.
- The **Add a Vault** dialog no longer has the open Settings section's buttons drawn over it on a wide screen, and it now fits a phone screen.
- `/ready` now answers `200` on an instance with no Vaults.
- The What's new dialog now holds keyboard focus from the moment it appears.
- Loading the EmbeddingGemma search model no longer looks up its own folder thousands of times.
- Polling `/ready` while Hatchdoor starts no longer fills the log with errors.
- Search snippets no longer show the source of a code block or a diagram as if it were prose.
- Opening Hatchdoor in a browser no longer contacts Google, and the app now shows the fonts it was designed with.
- The bundled fonts now draw the app's heaviest text themselves.
- Hatchdoor now says which Vault is in the way when it refuses a folder for a new or edited Vault.
- A Vault that is reindexing no longer tells agents it cannot be searched.
- Two pieces of built-in help now say what Hatchdoor does.
- On a phone, the Edit button on an open note no longer shows an unexplained `E` in a small pill beside its label.
- Bullet lists in a note are indented again.
- Two notices and a drop hint name the right things.
- The find and replace bar in the live editor now follows the theme and is easier to use.
- The formatting toolbar over a selection in the live editor is dark with square corners again, as designed.
