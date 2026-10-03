---
tags: [type/tutorial, topic/web-ui]
---

# Browse and review through the Web UI

The browser and your agent work on the same notes. In this step you open the note the agent just changed and check the change. The rest of the page is a short tour of the browser app, for when you come back to it later.

## Check the agent's change

1. Open `http://localhost:42824` and enter the web token if it asks.
2. In the sidebar, open the note the agent changed.
3. Find the new bullet and check it sits under the heading you asked for.

If it is there and nothing else changed, your agent works the way it should.

## Move between Vaults

The Vault selector narrows the app to one Vault, or **All Vaults** shows every Vault that is turned on.

Narrowing to one Vault while you are reading a note takes you with it: the note you last had open in that Vault comes back, the same way reopening Hatchdoor returns you to the note you left. Pick a Vault you have not read anything in yet and you land on the empty start page instead, with that Vault's notes in the sidebar. Choosing **All Vaults**, or switching while you are in Settings, Statistics or the graph, leaves the page you are on alone.

If the Vault you narrowed to is paused, stops working or is disconnected (from Settings, another tab or an agent), the selector goes back to **All Vaults** by itself and a notice at the top says why.

## Search

Select **Search** in the top bar, or press `/` anywhere outside a text field.

- Normal search finds notes by meaning, so "trip to Japan" can find a note titled "Tokyo itinerary" that never says "trip".
- Turn on **Keyword mode** when the exact words matter, such as a hostname, tag, filename, command or ID.

Search always looks in every Vault that is turned on, whichever one the selector is narrowed to. The list down the left of the search panel says how many matching notes each Vault holds, and starts on the Vault you were browsing. Select another Vault to read its matches, or **All results** to see them together. On a phone the same choice is the **Scope** field under the search box.

> [!note]
> Search in the browser when you want to scan results yourself. Ask the agent when you want it to read what it finds and act on it.

## Edit notes

On a Vault Hatchdoor can write to, you can edit notes in the browser. **New note** creates a note. Click any paragraph, heading, list item or table row to edit it in place; [[How to edit notes with the live editor]] covers it in full.

If those controls are missing, the Vault is read-only or the instance is a public demo. If they carry a warning that the Vault's filesystem cannot swap two files in one step, editing still works; [[Install Hatchdoor with Docker Compose#If your notes are on ZFS or a FUSE mount]] explains what that costs.

Your notes stay plain Markdown files, so you can still open them in Obsidian or any other Markdown app at the same time. Hatchdoor understands both wikilinks and Markdown links between notes, and writes new links in whichever style your Vault already uses.

A note's text is always read straight from its file. Its links and backlinks show an edit made in Hatchdoor straight away. An edit made elsewhere, such as in Obsidian or by a Git sync, can take a few seconds to show up there.

A note can also hold a saved query: a fenced `base` block that lists matching notes, such as every subscription that has not ended yet. The note page draws it as a table, worked out afresh each time you open the note, and each row links to its note. Click a column heading to sort by it; a reload forgets the sort. The table is never written into the file, so another Markdown app shows the block itself. [[Supported Markdown reference]] lists what a saved query can say.

## Get help

Select the **?** button in the top bar to open Help, this manual, beside whatever you are doing. On a phone, Help is the first item of the **…** menu instead, and takes the whole screen. Help always shows the manual for the version of Hatchdoor you are running. Its search box finds pages by the words in them, and **Open full width** gives a long page the whole screen. Press `Escape` or select **Close** to go back where you were.

Help works before you sign in, too. The token prompt links straight to [[Install Hatchdoor with Docker Compose#Where do I find my token?|where to find your token]].

Where you might get stuck, a **How does this work?** link opens Help at the section that explains what you are looking at: the "No Vaults Yet" screen, the search model choice, a Vault list that will not load, each section of **Settings**, the folder list in **Add a Vault**, and a Vault's own page in **Settings**. On a Vault's page the link follows what is going on with that Vault, such as a paused Vault, a folder Hatchdoor cannot read, or a failing Git sync.

## The setup checklist

A new install opens on the **Set up Hatchdoor** checklist instead of an empty workspace. It walks through adding your notes, connecting your agent and trying a search, and ticks each step off by itself once it has really happened. Select **Close the checklist** when you are done; that browser then opens on the workspace as usual. To bring it back, open Help and select **Setup checklist** at the top of Help's home page. See [[Connect your agent#The quick way: the setup checklist]] for the agent step.

## After an upgrade

The first time you open Hatchdoor in a browser after an upgrade, a **What's new** dialog lists the highlights of every release since that browser last looked, newest first. Anything you must do because of the upgrade is pinned at the top under **Action needed**. A highlight's link opens Help at the page that explains it, and **Full changelog** opens [[What's new]]. Select **Got it** or press `Escape` and that browser does not show it again until the next upgrade. Each browser keeps its own record, so a phone and a laptop each show it once. A fresh install never shows it, and neither does a browser that blocks site data, since it could not remember that you had seen it.

If you turned on **Tell me about new releases** in **Settings** → **Updates**, a line at the top of the page says when a newer Hatchdoor is out, with a **What's new** link to its release notes and a **How to upgrade** link to [[How to upgrade Hatchdoor]]. Close it with **×** and that browser leaves it closed until the next release.

Finish with [[Understand where your data lives]].

---

Previous: [[Search and change notes with your agent]]
Next: [[Understand where your data lives]]
